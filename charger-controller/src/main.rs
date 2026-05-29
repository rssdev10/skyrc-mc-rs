use iced::window;

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

rust_i18n::i18n!("locales", fallback = "en");

mod app;
mod slot;
mod ui;
mod data;
mod export;
mod config_dialog;
mod settings;
mod i18n;
mod profiles;

use app::ChargerApp;

#[derive(Parser)]
#[command(name = "charger-controller")]
#[command(author = "MC5000 Project")]
#[command(version = "0.1.0")]
#[command(about = "MC5000 Battery Charger Controller with GUI")]
struct Cli {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
    
    /// Run in headless mode (no GUI, just execute command and exit)
    #[arg(short = 'H', long)]
    headless: bool,
    
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Clone)]
enum Commands {
    /// Scan for available devices
    Scan,
    
    /// Get status of a specific slot or all slots
    Status {
        /// Slot number (1-4), or omit for all slots
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
    },
    
    /// Monitor all slots continuously
    Monitor {
        /// Update interval in seconds
        #[arg(short, long, default_value = "2")]
        interval: u64,
    },
    
    /// Start charging a slot
    Start {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,
        
        /// Battery chemistry: liion, liion-hv, lifepo4, nimh, nicd, nizn, lto, ram
        #[arg(short, long, default_value = "liion")]
        chemistry: String,
        
        /// Charge current in mA
        #[arg(short = 'a', long, default_value = "2000")]
        current: u16,
        
        /// Battery capacity in mAh
        #[arg(short = 'p', long, default_value = "3000")]
        capacity: u16,
    },
    
    /// Stop charging a slot
    Stop {
        /// Slot number (1-4), or omit to stop all
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
    },
    
    /// Auto-detect battery chemistry and start charging
    Auto {
        /// Slot number (1-4), or omit for all slots
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
        
        /// Starting current in mA (will adjust based on resistance)
        #[arg(short = 'a', long, default_value = "100")]
        current: u16,
    },
    
    /// Open GUI (default if no command specified)
    Gui,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    // Set up verbose mode
    if cli.verbose {
        println!("╔═══════════════════════════════════════════════════════════════════════╗");
        println!("║     MC5000 Charger Controller - VERBOSE MODE ENABLED                 ║");
        println!("╚═══════════════════════════════════════════════════════════════════════╝");
        println!("[INFO] Verbose mode enabled");
        println!("[INFO] Setting environment variables:");
        std::env::set_var("MC5000_VERBOSE", "1");
        std::env::set_var("RUST_LOG", "debug");
        println!("[INFO]   MC5000_VERBOSE=1");
        println!("[INFO]   RUST_LOG=debug");
    }
    
    env_logger::init();
    
    // Handle CLI commands
    match &cli.command {
        Some(Commands::Scan) => {
            run_scan(cli.verbose)?;
        }
        Some(Commands::Status { slot }) => {
            run_status(cli.verbose, *slot)?;
        }
        Some(Commands::Monitor { interval }) => {
            run_monitor(cli.verbose, *interval)?;
        }
        Some(Commands::Start { slot, chemistry, current, capacity }) => {
            run_start(cli.verbose, *slot, chemistry, *current, *capacity)?;
        }
        Some(Commands::Stop { slot }) => {
            run_stop(cli.verbose, *slot)?;
        }
        Some(Commands::Auto { slot, current }) => {
            run_auto(cli.verbose, *slot, *current)?;
        }
        Some(Commands::Gui) | None => {
            // If headless flag is set with no command, just print info
            if cli.headless {
                println!("Headless mode requires a command. Use --help for available commands.");
                return Ok(());
            }
            
            // Run GUI
            run_gui(cli.verbose)?;
        }
    }
    
    Ok(())
}

fn run_gui(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    if verbose {
        println!("[INFO] Starting MC5000 Charger Controller GUI");
        println!("[INFO] Window configuration:");
        println!("[INFO]   Size: 1200x800");
        println!("[INFO]   Position: Centered");
        println!("[INFO]   Min size: 800x600");
        println!("[INFO]   Icon: img/mc5000.jpg");
        println!("[INFO]   Theme: Dark");
        println!("════════════════════════════════════════════════════════════════════════\n");
    }
    
    // Load window icon
    let icon = load_icon();
    
    iced::application(ChargerApp::new, ChargerApp::update, ChargerApp::view)
        .title(ChargerApp::title)
        .subscription(ChargerApp::subscription)
        .theme(ChargerApp::theme)
        .window(window::Settings {
            size: iced::Size::new(1200.0, 800.0),
            position: iced::window::Position::Centered,
            min_size: Some(iced::Size::new(1000.0, 600.0)),
            icon,
            ..Default::default()
        })
        .run()?;
    
    Ok(())
}

fn run_scan(_verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    use mc5000_protocol::MC5000Protocol;
    
    println!("Scanning for MC5000 devices...");
    
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match MC5000Protocol::scan_devices(5).await {
            Ok(devices) => {
                if devices.is_empty() {
                    println!("No devices found.");
                } else {
                    println!("Found {} device(s):", devices.len());
                    for device in &devices {
                        let mc5000_tag = if device.is_mc5000 { " [MC5000]" } else { "" };
                        println!("  - {} (RSSI: {:?}){}", device.name, device.rssi, mc5000_tag);
                    }
                }
            }
            Err(e) => {
                eprintln!("Scan failed: {}", e);
            }
        }
    });
    
    Ok(())
}

fn run_status(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn std::error::Error>> {
    use mc5000_protocol::{MC5000SlotStatus, MC5000Protocol};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use futures::stream::StreamExt;
    use btleplug::api::Peripheral as _;
    
    println!("Connecting to MC5000...");
    
    // Create Arc outside async block so it outlives the block_on
    let slot_statuses: Arc<Mutex<HashMap<u8, MC5000SlotStatus>>> = 
        Arc::new(Mutex::new(HashMap::new()));
    let slot_statuses_for_spawn = slot_statuses.clone();
    let slot_statuses_for_print = slot_statuses.clone();
    
    let rt = tokio::runtime::Runtime::new()?;
    let slots_to_query: Vec<u8> = match slot {
        Some(s) if (1..=4).contains(&s) => vec![s],
        _ => vec![1, 2, 3, 4],
    };
    let slots_to_query_clone = slots_to_query.clone();
    
    rt.block_on(async move {
        // Scan for devices first
        let devices = MC5000Protocol::scan_devices(5).await;
        let devices = match devices {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Scan failed: {}", e);
                return;
            }
        };
        
        let mc5000 = devices.iter().find(|d| d.is_mc5000);
        
        if let Some(device) = mc5000 {
            println!("Found: {}", device.name);
            
            let mut protocol = MC5000Protocol::new();
            if let Err(e) = protocol.connect(&device.peripheral).await {
                eprintln!("Connection failed: {}", e);
                return;
            }
            println!("Connected!\n");
            
            // Set up notification listener with its own Arc clone
            let peripheral = device.peripheral.clone();
            let verbose_clone = verbose;
            tokio::spawn(async move {
                if let Ok(mut stream) = peripheral.notifications().await {
                    while let Some(notification) = stream.next().await {
                        let data = &notification.value;
                        if verbose_clone && data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                            println!("[RAW] Slot mask=0x{:02X} status=0x{:02X} byte5={} voltage={}",
                                data[3], data[4], data[5],
                                u16::from_be_bytes([data[6], data[7]]));
                        }
                        if data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                            if let Ok(status) = MC5000SlotStatus::parse_from_response(data) {
                                let slot_id = match data[3] {
                                    0x01 => 1, 0x02 => 2, 0x04 => 3, 0x08 => 4, _ => 0,
                                };
                                if slot_id > 0 {
                                    if let Ok(mut map) = slot_statuses_for_spawn.lock() {
                                        map.insert(slot_id, status);
                                    }
                                }
                            }
                        }
                    }
                }
            });
            
            // Wait for notification handler
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            
            // Request status for each slot
            for slot_num in &slots_to_query_clone {
                let _ = protocol.request_slot_status(*slot_num).await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            
            // Wait for responses
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        } else {
            eprintln!("No MC5000 device found. Available devices:");
            for d in &devices {
                eprintln!("  - {}", d.name);
            }
        }
    });
    
    // Print results after block_on completes
    println!("=== Slot Status ===\n");
    println!("{:>4}  {:>12}  {:>8}  {:>8}  {:>8}  {:>6}  {:>10}",
        "Slot", "State", "Voltage", "Current", "Capacity", "R(mΩ)", "Time");
    println!("{}", "-".repeat(70));
    
    if let Ok(map) = slot_statuses_for_print.lock() {
        for slot_num in &slots_to_query {
            if let Some(status) = map.get(slot_num) {
                let state_str = format!("{:?}", status.state);
                let state_icon = match status.state {
                    mc5000_protocol::MC5000SlotState::Charging => "⚡",
                    mc5000_protocol::MC5000SlotState::ChargingCC => "⚡",
                    mc5000_protocol::MC5000SlotState::ChargingCV => "⚡",
                    mc5000_protocol::MC5000SlotState::Discharging => "▼",
                    mc5000_protocol::MC5000SlotState::Completed => "✓",
                    mc5000_protocol::MC5000SlotState::Idle => "○",
                    mc5000_protocol::MC5000SlotState::Empty => "·",
                    mc5000_protocol::MC5000SlotState::Paused => "⏸",
                    mc5000_protocol::MC5000SlotState::Error => "✗",
                };
                
                let mins = status.elapsed_seconds / 60;
                let secs = status.elapsed_seconds % 60;
                
                println!("{:>4}  {} {:>10}  {:>6.3}V  {:>6}mA  {:>6}mAh  {:>6}  {:>6}:{:02}",
                    slot_num,
                    state_icon,
                    state_str,
                    status.voltage_mv as f32 / 1000.0,
                    status.current_ma,
                    status.capacity_mah,
                    status.resistance_milliohm,
                    mins,
                    secs
                );
            } else {
                println!("{:>4}  {:>12}  (no response)", slot_num, "Unknown");
            }
        }
    }
    
    Ok(())
}

fn run_monitor(verbose: bool, interval: u64) -> Result<(), Box<dyn std::error::Error>> {
    println!("Monitor mode - Press Ctrl+C to stop");
    println!("Update interval: {}s\n", interval);
    
    // For now, just loop calling status
    loop {
        run_status(verbose, None)?;
        println!();
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

fn run_start(verbose: bool, slot: u8, chemistry: &str, current: u16, capacity: u16) -> Result<(), Box<dyn std::error::Error>> {
    use mc5000_protocol::{ChargeConfig, OperationMode, MC5000Protocol};
    
    if !(1..=4).contains(&slot) {
        eprintln!("Invalid slot number. Must be 1-4.");
        return Ok(());
    }
    
    let chem = parse_chemistry(chemistry).ok_or("Invalid chemistry")?;
    
    println!("Starting charge on slot {} with {} chemistry...", slot, chemistry);
    if verbose {
        println!("[VERBOSE] Current: {}mA, Capacity: {}mAh", current, capacity);
    }
    
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let devices = match MC5000Protocol::scan_devices(5).await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Scan failed: {}", e);
                return;
            }
        };
        
        let mc5000 = devices.iter().find(|d| d.is_mc5000);
        
        if let Some(device) = mc5000 {
            let mut protocol = MC5000Protocol::new();
            match protocol.connect(&device.peripheral).await {
                Ok(_) => {
                    println!("Connected!");
                    
                    let config = ChargeConfig::new(slot, chem, OperationMode::Charge, capacity, current);
                    let cmd = MC5000Protocol::build_charge_config_command(&config);
                    
                    if verbose {
                        println!("[VERBOSE] Sending config: {:02X?}", cmd);
                    }
                    
                    if let Err(e) = protocol.send_command(&cmd).await {
                        eprintln!("Failed to send config: {}", e);
                        return;
                    }
                    
                    // Start
                    let start_cmd = MC5000Protocol::build_start_stop_command(
                        mc5000_protocol::StartStopAction::ChannelMask(1 << (slot - 1))
                    );
                    
                    if let Err(e) = protocol.send_command(&start_cmd).await {
                        eprintln!("Failed to start: {}", e);
                        return;
                    }
                    
                    println!("✓ Charging started on slot {}", slot);
                }
                Err(e) => eprintln!("Connection failed: {}", e),
            }
        } else {
            eprintln!("No MC5000 device found.");
        }
    });
    
    Ok(())
}

fn run_stop(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn std::error::Error>> {
    use mc5000_protocol::{MC5000Protocol, StartStopAction};
    
    let slot_msg = slot.map(|s| format!("slot {}", s)).unwrap_or("all slots".to_string());
    println!("Stopping {}...", slot_msg);
    
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let devices = match MC5000Protocol::scan_devices(5).await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Scan failed: {}", e);
                return;
            }
        };
        
        let mc5000 = devices.iter().find(|d| d.is_mc5000);
        
        if let Some(device) = mc5000 {
            let mut protocol = MC5000Protocol::new();
            match protocol.connect(&device.peripheral).await {
                Ok(_) => {
                    println!("Connected!");
                    
                    let action = match slot {
                        Some(s) if (1..=4).contains(&s) => StartStopAction::ChannelMask(1 << (s - 1)),
                        _ => StartStopAction::StopAll,
                    };
                    
                    let stop_cmd = MC5000Protocol::build_start_stop_command(action);
                    
                    if verbose {
                        println!("[VERBOSE] Sending stop command: {:02X?}", stop_cmd);
                    }
                    
                    if let Err(e) = protocol.send_command(&stop_cmd).await {
                        eprintln!("Failed to stop: {}", e);
                        return;
                    }
                    
                    println!("✓ Stopped {}", slot_msg);
                }
                Err(e) => eprintln!("Connection failed: {}", e),
            }
        } else {
            eprintln!("No MC5000 device found.");
        }
    });
    
    Ok(())
}

fn run_auto(_verbose: bool, _slot: Option<u8>, _current: u16) -> Result<(), Box<dyn std::error::Error>> {
    println!("Auto-charge not yet implemented in GUI CLI. Use charger-cli for full auto-charge support.");
    Ok(())
}

fn parse_chemistry(s: &str) -> Option<mc5000_protocol::BatteryChemistry> {
    use mc5000_protocol::BatteryChemistry;
    match s.to_lowercase().as_str() {
        "liion" | "li-ion" => Some(BatteryChemistry::LiIon),
        "liion-hv" | "li-ion-hv" => Some(BatteryChemistry::LiIonHV),
        "lifepo4" | "lfp" => Some(BatteryChemistry::LiFePO4),
        "nimh" | "ni-mh" => Some(BatteryChemistry::NiMH),
        "nicd" | "ni-cd" => Some(BatteryChemistry::NiCd),
        "eneloop" => Some(BatteryChemistry::Eneloop),
        "nizn" | "ni-zn" => Some(BatteryChemistry::NiZn),
        "ram" | "alkaline" => Some(BatteryChemistry::RAM),
        "lto" => Some(BatteryChemistry::LTO),
        "naion" | "na-ion" => Some(BatteryChemistry::NaIon),
        _ => None,
    }
}

fn candidate_icon_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(executable) = std::env::current_exe() {
        if let Some(executable_dir) = executable.parent() {
            candidates.push(executable_dir.join("img/mc5000.jpg"));

            if let Some(contents_dir) = executable_dir.parent() {
                candidates.push(contents_dir.join("Resources/img/mc5000.jpg"));
            }
        }
    }

    candidates.push(PathBuf::from("img/mc5000.jpg"));
    candidates
}

fn find_icon_path() -> Option<PathBuf> {
    candidate_icon_paths()
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
}

fn load_icon() -> Option<window::Icon> {
    let Some(icon_path) = find_icon_path() else {
        eprintln!("Warning: Icon file not found in expected locations");
        return None;
    };

    match image::open(&icon_path) {
        Ok(img) => {
            // Convert to RGBA format and resize to reasonable icon size
            let rgba = img.resize(256, 256, image::imageops::FilterType::Lanczos3).to_rgba8();
            let (width, height) = rgba.dimensions();
            let pixels = rgba.into_raw();
            
            match window::icon::from_rgba(pixels, width, height) {
                Ok(icon) => {
                    println!("[INFO] ✓ Window icon loaded successfully");
                    Some(icon)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to create icon: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: Failed to load icon image: {}", e);
            None
        }
    }
}
