//! MC5000 Charger Controller CLI
//! 
//! Command-line interface for the MC5000 battery charger.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{Parser, Subcommand};
use mc5000_protocol::{
    MC5000Protocol, BatteryChemistry, ChargeConfig, StartStopAction,
    MC5000SlotState, OperationMode,
};
use btleplug::api::Peripheral as _;
use futures::stream::StreamExt;
use tokio::runtime::Runtime;

#[derive(Parser)]
#[command(name = "charger-cli")]
#[command(author = "MC5000 Project")]
#[command(version = "0.1.0")]
#[command(about = "Command-line interface for MC5000 battery charger")]
struct Cli {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
    
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan for available devices
    Scan,
    
    /// Monitor all slots continuously
    Monitor {
        /// Update interval in seconds
        #[arg(short, long, default_value = "2")]
        interval: u64,
    },
    
    /// Get status of a specific slot or all slots
    Status {
        /// Slot number (1-4), or omit for all slots
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
    },
    
    /// Start charging a slot
    Charge {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,
        
        /// Battery chemistry: liion, liion-hv, lifepo4, nimh, nicd, eneloop, nizn, ram, lto, naion
        #[arg(short, long, default_value = "liion")]
        chemistry: String,
        
        /// Charge current in mA
        #[arg(short = 'a', long, default_value = "2000")]
        current: u16,
        
        /// Battery capacity in mAh
        #[arg(short = 'p', long, default_value = "3000")]
        capacity: u16,
    },
    
    /// Start discharging a slot
    Discharge {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,
        
        /// Battery chemistry: liion, liion-hv, lifepo4, nimh, nicd, eneloop, nizn, ram, lto, naion
        #[arg(short, long, default_value = "liion")]
        chemistry: String,
        
        /// Discharge current in mA
        #[arg(short = 'a', long, default_value = "500")]
        current: u16,
        
        /// Battery capacity in mAh
        #[arg(short = 'p', long, default_value = "3000")]
        capacity: u16,
    },
    
    /// Stop charging/discharging a slot
    Stop {
        /// Slot number (1-4), or omit to stop all
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
    },
    
    /// Run cycle mode on a slot (charge/discharge cycles)
    Cycle {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,
        
        /// Battery chemistry: liion, liion-hv, lifepo4, nimh, nicd, eneloop, nizn, ram, lto, naion
        #[arg(short, long, default_value = "liion")]
        chemistry: String,
        
        /// Charge current in mA
        #[arg(short = 'a', long, default_value = "2000")]
        current: u16,
        
        /// Battery capacity in mAh
        #[arg(short = 'p', long, default_value = "3000")]
        capacity: u16,
    },
    
    /// Run refresh mode on a slot (deep cycle for NiMH/NiCd)
    Refresh {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,
        
        /// Battery chemistry: nimh, nicd, eneloop
        #[arg(short, long, default_value = "nimh")]
        chemistry: String,
        
        /// Charge current in mA
        #[arg(short = 'a', long, default_value = "400")]
        current: u16,
        
        /// Discharge current in mA
        #[arg(short = 'd', long, default_value = "250")]
        discharge_current: u16,
        
        /// Battery capacity in mAh
        #[arg(short = 'p', long, default_value = "3000")]
        capacity: u16,
    },
    
    /// Run break-in mode on a slot (conditioning for new NiMH/NiCd batteries)
    BreakIn {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,
        
        /// Battery chemistry: nimh, nicd, eneloop
        #[arg(short, long, default_value = "nimh")]
        chemistry: String,
        
        /// Charge current in mA (typically 0.1-0.3C rate)
        #[arg(short = 'a', long, default_value = "300")]
        current: u16,
        
        /// Battery capacity in mAh
        #[arg(short = 'p', long, default_value = "3000")]
        capacity: u16,
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
    
    /// Debug command to test BLE communication and view notifications
    Debug {
        /// Duration in seconds to listen for notifications
        #[arg(short, long, default_value = "10")]
        duration: u64,
    },
    
    /// Run protocol 3 test: break-in slot 4 + cycle slot 1 + charge slot 2 in one session
    Proto3 {},
    
    /// Run protocol 4 reproduction: full capture 4 sequence (NiMH/Li-Ion/LiFePO4/RAM)
    Proto4 {
        /// Run in dry-run mode (build and print packets without sending)
        #[arg(long)]
        dry_run: bool,
    },
}

fn parse_chemistry(s: &str) -> Option<BatteryChemistry> {
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

fn chemistry_name(chem: BatteryChemistry) -> &'static str {
    match chem {
        BatteryChemistry::LiIon => "Li-Ion (4.2V)",
        BatteryChemistry::LiIonHV => "Li-Ion HV (4.35V)",
        BatteryChemistry::LiFePO4 => "LiFePO4 (3.65V)",
        BatteryChemistry::NiMH => "NiMH (1.65V)",
        BatteryChemistry::NiCd => "NiCd (1.65V)",
        BatteryChemistry::Eneloop => "eneloop (1.65V)",
        BatteryChemistry::NiZn => "NiZn (1.9V)",
        BatteryChemistry::RAM => "RAM (1.65V)",
        BatteryChemistry::LTO => "LTO (2.85V)",
        BatteryChemistry::NaIon => "Na-Ion (4.0V)",
    }
}

fn detect_chemistry_from_voltage(voltage_mv: u32) -> BatteryChemistry {
    match voltage_mv {
        v if v >= 4000 => BatteryChemistry::LiIon,
        v if v >= 3800 => BatteryChemistry::LiIonHV,
        v if v >= 3300 => BatteryChemistry::LiFePO4,
        v if v >= 3000 => BatteryChemistry::LiFePO4,
        v if v >= 2200 => BatteryChemistry::LTO,
        v if v >= 1700 => BatteryChemistry::NiZn,
        v if v >= 1300 => BatteryChemistry::NiMH,
        _ => BatteryChemistry::NiMH,
    }
}

fn state_symbol(state: &MC5000SlotState) -> &'static str {
    match state {
        MC5000SlotState::Idle => "○",
        MC5000SlotState::Charging | MC5000SlotState::ChargingCC | MC5000SlotState::ChargingCV => "⚡",
        MC5000SlotState::Discharging => "▼",
        MC5000SlotState::Completed => "✓",
        MC5000SlotState::Paused => "⏸",
        MC5000SlotState::Error => "✗",
        MC5000SlotState::Empty => "·",
    }
}

fn state_name(state: &MC5000SlotState) -> &'static str {
    match state {
        MC5000SlotState::Idle => "Idle",
        MC5000SlotState::Charging => "Charging",
        MC5000SlotState::ChargingCC => "CC Charge",
        MC5000SlotState::ChargingCV => "CV Charge",
        MC5000SlotState::Discharging => "Discharge",
        MC5000SlotState::Completed => "Done",
        MC5000SlotState::Paused => "Paused",
        MC5000SlotState::Error => "Error",
        MC5000SlotState::Empty => "Empty",
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let cli = Cli::parse();
    
    if cli.verbose {
        std::env::set_var("MC5000_VERBOSE", "1");
        println!("[INFO] Verbose mode enabled");
    }
    
    let rt = Runtime::new()?;
    rt.block_on(async {
        run_command(cli).await
    })
}

async fn run_command(cli: Cli) -> Result<(), Box<dyn Error>> {
    let timeout_duration = Duration::from_secs(600); // Proto4 needs ~5 min with polling
    
    tokio::time::timeout(timeout_duration, async {
        match &cli.command {
            None | Some(Commands::Monitor { .. }) => {
                let interval = match &cli.command {
                    Some(Commands::Monitor { interval }) => *interval,
                    _ => 2,
                };
                run_monitor(cli.verbose, interval).await
            }
            Some(Commands::Scan) => run_scan(cli.verbose).await,
            Some(Commands::Status { slot }) => run_status(cli.verbose, *slot).await,
            Some(Commands::Charge { slot, chemistry, current, capacity }) => {
                let chem = parse_chemistry(chemistry)
                    .ok_or_else(|| format!("Unknown chemistry: {}", chemistry))?;
                run_charge(cli.verbose, *slot, chem, *current, *capacity).await
            }
            Some(Commands::Discharge { slot, chemistry, current, capacity }) => {
                let chem = parse_chemistry(chemistry)
                    .ok_or_else(|| format!("Unknown chemistry: {}", chemistry))?;
                run_discharge(cli.verbose, *slot, chem, *current, *capacity).await
            }
            Some(Commands::Stop { slot }) => run_stop(cli.verbose, *slot).await,
            Some(Commands::Cycle { slot, chemistry, current, capacity }) => {
                let chem = parse_chemistry(chemistry)
                    .ok_or_else(|| format!("Unknown chemistry: {}", chemistry))?;
                run_cycle(cli.verbose, *slot, chem, *current, *capacity).await
            }
            Some(Commands::Refresh { slot, chemistry, current, discharge_current, capacity }) => {
                let chem = parse_chemistry(chemistry)
                    .ok_or_else(|| format!("Unknown chemistry: {}", chemistry))?;
                run_refresh(cli.verbose, *slot, chem, *current, *discharge_current, *capacity).await
            }
            Some(Commands::BreakIn { slot, chemistry, current, capacity }) => {
                let chem = parse_chemistry(chemistry)
                    .ok_or_else(|| format!("Unknown chemistry: {}", chemistry))?;
                run_break_in(cli.verbose, *slot, chem, *current, *capacity).await
            }
            Some(Commands::Auto { slot, current }) => run_auto(cli.verbose, *slot, *current).await,
            Some(Commands::Debug { duration }) => run_debug(cli.verbose, *duration).await,
            Some(Commands::Proto3 {}) => run_proto3(cli.verbose).await,
            Some(Commands::Proto4 { dry_run }) => run_proto4(cli.verbose, *dry_run).await,
        }
    }).await?
}
async fn connect_to_device(verbose: bool) -> Result<(MC5000Protocol, btleplug::platform::Peripheral), Box<dyn Error>> {
    println!("Scanning for MC5000 devices (5s)...");
    let devices = MC5000Protocol::scan_devices(5).await?;
    
    if devices.is_empty() {
        return Err("No Bluetooth devices found".into());
    }
    
    if verbose {
        println!("Found {} devices:", devices.len());
        for dev in &devices {
            println!("  {} (RSSI: {:?}){}",
                dev.name, dev.rssi,
                if dev.is_mc5000 { " <<< MC5000" } else { "" });
        }
    }
    
    let mc5000 = devices.into_iter()
        .find(|d| d.is_mc5000)
        .ok_or("No MC5000 device found")?;
    
    println!("Connecting to {}...", mc5000.name);
    let mut protocol = MC5000Protocol::new();
    protocol.connect(&mc5000.peripheral).await?;
    println!("Connected!\n");
    
    Ok((protocol, mc5000.peripheral))
}

/// Connect in read-only mode (no init sequence — won't disrupt running operations)
async fn connect_to_device_readonly(verbose: bool) -> Result<(MC5000Protocol, btleplug::platform::Peripheral), Box<dyn Error>> {
    println!("Scanning for MC5000 devices (5s)...");
    let devices = MC5000Protocol::scan_devices(5).await?;
    
    if devices.is_empty() {
        return Err("No Bluetooth devices found".into());
    }
    
    if verbose {
        println!("Found {} devices:", devices.len());
        for dev in &devices {
            println!("  {} (RSSI: {:?}){}",
                dev.name, dev.rssi,
                if dev.is_mc5000 { " <<< MC5000" } else { "" });
        }
    }
    
    let mc5000 = devices.into_iter()
        .find(|d| d.is_mc5000)
        .ok_or("No MC5000 device found")?;
    
    println!("Connecting to {}...", mc5000.name);
    let mut protocol = MC5000Protocol::new();
    protocol.connect_without_init(&mc5000.peripheral).await?;
    println!("Connected!\n");
    
    Ok((protocol, mc5000.peripheral))
}

async fn run_scan(verbose: bool) -> Result<(), Box<dyn Error>> {
    println!("=== Scanning for Bluetooth devices ===\n");
    
    let devices = MC5000Protocol::scan_devices(5).await?;
    
    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }
    
    println!("Found {} devices:", devices.len());
    for dev in &devices {
        let mc5000_tag = if dev.is_mc5000 { " [MC5000]" } else { "" };
        println!("  {} - {} (RSSI: {:?}){}",
            dev.id, dev.name, dev.rssi, mc5000_tag);
        
        if verbose {
            println!("    Address: {}", dev.address);
        }
    }
    
    Ok(())
}

async fn run_status(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn Error>> {
    let (protocol, peripheral) = connect_to_device_readonly(verbose).await?;
    
    let slots = match slot {
        Some(s) if s >= 1 && s <= 4 => vec![s],
        Some(s) => return Err(format!("Invalid slot: {}. Must be 1-4", s).into()),
        None => vec![1, 2, 3, 4],
    };
    
    // Set up notification listener to collect responses
    use std::sync::Mutex;
    use std::collections::HashMap;
    
    let slot_status: Arc<Mutex<HashMap<u8, mc5000_protocol::MC5000SlotStatus>>> = 
        Arc::new(Mutex::new(HashMap::new()));
    let slot_status_clone = slot_status.clone();
    let verbose_clone = verbose;
    
    let peripheral_clone = peripheral.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = peripheral_clone.notifications().await {
            while let Some(notification) = stream.next().await {
                let data = &notification.value;
                if verbose_clone && data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                    println!("[RAW] Slot mask=0x{:02X} status=0x{:02X} byte5={} voltage={} data={:02X?}",
                        data[3], data[4], data[5],
                        u16::from_be_bytes([data[6], data[7]]),
                        &data[..data.len().min(20)]);
                }
                if data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                    if let Ok(status) = mc5000_protocol::MC5000SlotStatus::parse_from_response(data) {
                        let slot_id = match data[3] {
                            0x01 => 1,
                            0x02 => 2,
                            0x04 => 3,
                            0x08 => 4,
                            _ => 0,
                        };
                        if slot_id > 0 {
                            if let Ok(mut map) = slot_status_clone.lock() {
                                map.insert(slot_id, status);
                            }
                        }
                    }
                }
            }
        }
    });
    
    // Give notification handler time to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Request status for each slot
    for &slot_num in &slots {
        let _ = protocol.request_slot_status(slot_num).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for responses
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("=== Slot Status ===\n");
    println!("{:>4} {:>12} {:>8} {:>8} {:>9} {:>6} {:>12}",
        "Slot", "State", "Voltage", "Current", "Capacity", "R(mΩ)", "Time");
    println!("{}", "-".repeat(65));
    
    if let Ok(map) = slot_status.lock() {
        for &slot_num in &slots {
            if let Some(status) = map.get(&slot_num) {
                let state_str = format!("{} {}", 
                    state_symbol(&status.state), 
                    state_name(&status.state));
                let voltage = format!("{:.3}V", status.voltage_mv as f32 / 1000.0);
                let current = format!("{}mA", status.current_ma);
                let capacity = format!("{}mAh", status.capacity_mah);
                let resistance = format!("{}", status.resistance_milliohm);
                let time = format_time(status.elapsed_seconds);
                
                println!("{:>4} {:>12} {:>8} {:>8} {:>9} {:>6} {:>12}",
                    slot_num, state_str, voltage, current, capacity, resistance, time);
            } else {
                println!("Slot {}: No response", slot_num);
            }
        }
    }
    
    Ok(())
}

async fn run_charge(
    verbose: bool, 
    slot: u8, 
    chemistry: BatteryChemistry, 
    current: u16, 
    capacity: u16
) -> Result<(), Box<dyn Error>> {
    if slot < 1 || slot > 4 {
        return Err(format!("Invalid slot: {}. Must be 1-4", slot).into());
    }
    
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    println!("=== Starting Charge ===");
    println!("Slot: {}", slot);
    println!("Chemistry: {}", chemistry_name(chemistry));
    println!("Current: {}mA", current);
    println!("Capacity: {}mAh", capacity);
    println!();
    
    // Create charge configuration
    let charge_config = ChargeConfig::new(slot, chemistry, mc5000_protocol::OperationMode::Charge, capacity, current);
    
    // Send configuration
    println!("Sending configuration...");
    let config_cmd = MC5000Protocol::build_charge_config_command(&charge_config);
    if verbose {
        println!("  Config packet: {:02x?}", config_cmd);
    }
    protocol.send_command(&config_cmd).await?;
    println!("  ✓ Configuration sent");
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Send start command for this specific slot
    println!("Sending start command...");
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(1 << (slot - 1)));
    if verbose {
        println!("  Start packet: {:02x?}", start_cmd);
    }
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Start command sent");
    
    // Wait and verify
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    match protocol.get_slot_status(slot).await {
        Ok(status) => {
            println!("\nStatus after start:");
            println!("  State: {:?}", status.state);
            println!("  Voltage: {:.3}V", status.voltage_mv as f32 / 1000.0);
            println!("  Current: {}mA", status.current_ma);
            
            match status.state {
                MC5000SlotState::Charging | MC5000SlotState::ChargingCC | MC5000SlotState::ChargingCV => {
                    println!("\n✓ SUCCESS: Charging started!");
                }
                _ => {
                    println!("\n⚠ WARNING: Not in charging state. State: {:?}", status.state);
                }
            }
        }
        Err(e) => println!("Error checking status: {}", e),
    }
    
    Ok(())
}

async fn run_discharge(
    verbose: bool, 
    slot: u8, 
    chemistry: BatteryChemistry, 
    current: u16, 
    capacity: u16
) -> Result<(), Box<dyn Error>> {
    if slot < 1 || slot > 4 {
        return Err(format!("Invalid slot: {}. Must be 1-4", slot).into());
    }
    
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    println!("=== Starting Discharge ===");
    println!("Slot: {}", slot);
    println!("Chemistry: {}", chemistry_name(chemistry));
    println!("Discharge Current: {}mA", current);
    println!("Capacity: {}mAh", capacity);
    println!();
    
    // Create discharge configuration
    let discharge_config = ChargeConfig::discharge(slot, chemistry, capacity, current);
    
    // Send configuration
    println!("Sending configuration...");
    let config_cmd = MC5000Protocol::build_charge_config_command(&discharge_config);
    if verbose {
        println!("  Config packet: {:02x?}", config_cmd);
    }
    protocol.send_command(&config_cmd).await?;
    println!("  ✓ Configuration sent");
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Send start command for this specific slot
    println!("Sending start command...");
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(1 << (slot - 1)));
    if verbose {
        println!("  Start packet: {:02x?}", start_cmd);
    }
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Start command sent");
    
    // Wait and verify
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    match protocol.get_slot_status(slot).await {
        Ok(status) => {
            println!("\nStatus after start:");
            println!("  State: {:?}", status.state);
            println!("  Voltage: {:.3}V", status.voltage_mv as f32 / 1000.0);
            println!("  Current: {}mA", status.current_ma);
            
            match status.state {
                MC5000SlotState::Discharging => {
                    println!("\n✓ SUCCESS: Discharging started!");
                }
                _ => {
                    println!("\n⚠ WARNING: Not in discharging state. State: {:?}", status.state);
                }
            }
        }
        Err(e) => println!("Error checking status: {}", e),
    }
    
    Ok(())
}

async fn run_stop(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn Error>> {
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    match slot {
        Some(s) if s >= 1 && s <= 4 => {
            println!("Stopping slot {}...", s);
            let stop_cmd = MC5000Protocol::build_start_stop_command(
                StartStopAction::ChannelMask(1 << (s - 1))
            );
            protocol.send_command(&stop_cmd).await?;
            println!("  ✓ Stop command sent for slot {}", s);
        }
        Some(s) => return Err(format!("Invalid slot: {}. Must be 1-4", s).into()),
        None => {
            println!("Stopping all slots...");
            let stop_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::StopAll);
            protocol.send_command(&stop_cmd).await?;
            println!("  ✓ Stop all command sent");
        }
    }
    
    Ok(())
}

async fn run_cycle(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    if slot < 1 || slot > 4 {
        return Err(format!("Invalid slot: {}. Must be 1-4", slot).into());
    }
    
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    println!("=== Starting Cycle ===");
    println!("Slot: {}", slot);
    println!("Chemistry: {}", chemistry_name(chemistry));
    println!("Charge Current: {}mA", current);
    println!("Capacity: {}mAh", capacity);
    println!();
    
    let cycle_config = ChargeConfig::cycle(slot, chemistry, capacity, current);
    
    println!("Sending configuration...");
    let config_cmd = MC5000Protocol::build_charge_config_command(&cycle_config);
    if verbose {
        println!("  Config packet: {:02x?}", config_cmd);
    }
    protocol.send_command(&config_cmd).await?;
    println!("  ✓ Configuration sent");
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("Sending start command...");
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(1 << (slot - 1)));
    if verbose {
        println!("  Start packet: {:02x?}", start_cmd);
    }
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Cycle started");
    
    Ok(())
}

async fn run_refresh(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    charge_current: u16,
    discharge_current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    if slot < 1 || slot > 4 {
        return Err(format!("Invalid slot: {}. Must be 1-4", slot).into());
    }
    
    // Refresh is only for NiMH and NiCd
    if !matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd) {
        return Err("Refresh mode is only available for NiMH and NiCd chemistries".into());
    }
    
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    println!("=== Starting Refresh ===");
    println!("Slot: {}", slot);
    println!("Chemistry: {}", chemistry_name(chemistry));
    println!("Charge Current: {}mA", charge_current);
    println!("Discharge Current: {}mA", discharge_current);
    println!("Capacity: {}mAh", capacity);
    println!();
    
    let refresh_config = ChargeConfig::refresh(slot, chemistry, capacity, charge_current, discharge_current);
    
    println!("Sending configuration...");
    let config_cmd = MC5000Protocol::build_charge_config_command(&refresh_config);
    if verbose {
        println!("  Config packet: {:02x?}", config_cmd);
    }
    protocol.send_command(&config_cmd).await?;
    println!("  ✓ Configuration sent");
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("Sending start command...");
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(1 << (slot - 1)));
    if verbose {
        println!("  Start packet: {:02x?}", start_cmd);
    }
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Refresh started");
    
    Ok(())
}

async fn run_break_in(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    if slot < 1 || slot > 4 {
        return Err(format!("Invalid slot: {}. Must be 1-4", slot).into());
    }
    
    // Break-in is only for NiMH and NiCd
    if !matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd) {
        return Err("Break-in mode is only available for NiMH and NiCd chemistries".into());
    }
    
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    println!("=== Starting Break-In ===");
    println!("Slot: {}", slot);
    println!("Chemistry: {}", chemistry_name(chemistry));
    println!("Charge Current: {}mA", current);
    println!("Capacity: {}mAh", capacity);
    println!();
    
    let break_in_config = ChargeConfig::break_in(slot, chemistry, capacity, current);
    
    println!("Sending configuration...");
    let config_cmd = MC5000Protocol::build_charge_config_command(&break_in_config);
    if verbose {
        println!("  Config packet: {:02x?}", config_cmd);
    }
    protocol.send_command(&config_cmd).await?;
    println!("  ✓ Configuration sent");
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("Sending start command...");
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(1 << (slot - 1)));
    if verbose {
        println!("  Start packet: {:02x?}", start_cmd);
    }
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Break-in started");
    
    Ok(())
}

async fn run_auto(verbose: bool, slot: Option<u8>, initial_current: u16) -> Result<(), Box<dyn Error>> {
    let (protocol, _peripheral) = connect_to_device(verbose).await?;
    
    let slots = match slot {
        Some(s) if s >= 1 && s <= 4 => vec![s],
        Some(s) => return Err(format!("Invalid slot: {}. Must be 1-4", s).into()),
        None => vec![1, 2, 3, 4],
    };
    
    println!("=== Auto-Charge Mode ===\n");
    
    for slot_num in slots {
        match protocol.get_slot_status(slot_num).await {
            Ok(status) => {
                if status.voltage_mv < 100 {
                    println!("Slot {}: Empty, skipping", slot_num);
                    continue;
                }
                
                if !matches!(status.state, MC5000SlotState::Idle | MC5000SlotState::Empty) {
                    println!("Slot {}: Already active ({:?}), skipping", slot_num, status.state);
                    continue;
                }
                
                let chemistry = detect_chemistry_from_voltage(status.voltage_mv as u32);
                println!("Slot {}: Detected {} at {:.3}V",
                    slot_num,
                    chemistry_name(chemistry),
                    status.voltage_mv as f32 / 1000.0);
                
                // Create charge configuration with initial current
                let charge_config = ChargeConfig::new(slot_num, chemistry, mc5000_protocol::OperationMode::Charge, 3000, initial_current);
                
                let config_cmd = MC5000Protocol::build_charge_config_command(&charge_config);
                protocol.send_command(&config_cmd).await?;
                
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(e) => println!("Slot {}: Error - {}", slot_num, e),
        }
    }
    
    // Start all configured slots
    println!("\nStarting charge...");
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::StartAll);
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Charge started with {}mA initial current", initial_current);
    println!("\nNote: Run 'charger-cli monitor' to watch progress and see current adjustment");
    
    Ok(())
}

async fn run_monitor(verbose: bool, interval: u64) -> Result<(), Box<dyn Error>> {
    let (protocol, peripheral) = connect_to_device_readonly(verbose).await?;
    
    println!("=== Continuous Monitoring ===");
    println!("Press Ctrl+C to stop\n");
    
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        println!("\nStopping monitor...");
    }).ok();
    
    // Set up notification listener with slot status storage
    use std::sync::Mutex;
    use std::collections::HashMap;
    
    let slot_status: Arc<Mutex<HashMap<u8, mc5000_protocol::MC5000SlotStatus>>> = 
        Arc::new(Mutex::new(HashMap::new()));
    let slot_status_clone = slot_status.clone();
    
    let peripheral_clone = peripheral.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = peripheral_clone.notifications().await {
            while let Some(notification) = stream.next().await {
                let data = &notification.value;
                // Parse slot status from notification
                if data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                    if let Ok(status) = mc5000_protocol::MC5000SlotStatus::parse_from_response(data) {
                        let slot_id = match data[3] {
                            0x01 => 1,
                            0x02 => 2,
                            0x04 => 3,
                            0x08 => 4,
                            _ => 0,
                        };
                        if slot_id > 0 {
                            if let Ok(mut map) = slot_status_clone.lock() {
                                map.insert(slot_id, status);
                            }
                        }
                    }
                }
            }
        }
    });
    
    // Give notification handler time to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let mut last_print = std::time::Instant::now();
    
    while running.load(Ordering::SeqCst) {
        // Request status for all slots periodically
        if last_print.elapsed() >= Duration::from_secs(interval) {
            // Request status for each slot
            for slot in 1..=4u8 {
                let _ = protocol.request_slot_status(slot).await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            
            // Wait a bit for responses
            tokio::time::sleep(Duration::from_millis(200)).await;
            
            // Print status from collected data
            let now = chrono::Local::now().format("%H:%M:%S");
            print!("[{}] ", now);
            
            if let Ok(map) = slot_status.lock() {
                for slot in 1..=4u8 {
                    if let Some(status) = map.get(&slot) {
                        let symbol = state_symbol(&status.state);
                        let voltage = status.voltage_mv as f32 / 1000.0;
                        let current = status.current_ma;
                        
                        if status.voltage_mv > 0 {
                            print!("S{}: {} {:.2}V {}mA  ", slot, symbol, voltage, current);
                        } else {
                            print!("S{}: ·  ", slot);
                        }
                    } else {
                        print!("S{}: -  ", slot);
                    }
                }
            }
            println!();
            
            last_print = std::time::Instant::now();
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    Ok(())
}

fn format_time(seconds: u16) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{}:{:02}", minutes, secs)
    }
}

/// Debug command to test BLE communication - Protocol Validation Mode
async fn run_debug(verbose: bool, duration: u64) -> Result<(), Box<dyn Error>> {
    let (protocol, peripheral) = connect_to_device_readonly(verbose).await?;
    
    println!("=== Protocol Validation Mode ===");
    println!("Listening for {} seconds...\n", duration);
    
    // Spawn notification listener with detailed decoding
    let peripheral_clone = peripheral.clone();
    let notifications_received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notifications_received_clone = notifications_received.clone();
    
    let notifications_task = tokio::spawn(async move {
        if let Ok(mut stream) = peripheral_clone.notifications().await {
            while let Some(notification) = stream.next().await {
                notifications_received_clone.fetch_add(1, Ordering::SeqCst);
                let now = chrono::Local::now().format("%H:%M:%S%.3f");
                let data = &notification.value;
                
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("[{}] NOTIFY ({} bytes)", now, data.len());
                println!("RAW: {:02X?}", data);
                
                if data.len() >= 3 && data[0] == 0x0F {
                    let len = data[1];
                    let cmd = data[2];
                    println!("HEADER: start=0x0F len={} cmd=0x{:02X}", len, cmd);
                    
                    match cmd {
                        0x06 => {
                            println!("TYPE: Greeting (unsolicited)");
                        }
                        0x57 => {
                            println!("TYPE: Device Info Response");
                            if data.len() >= 22 {
                                let serial = String::from_utf8_lossy(&data[4..11]);
                                println!("  Serial: {}", serial);
                                println!("  FW: {}.{}", data[16], data[17]);
                                println!("  HW: {}.{}", data[18], data[19]);
                            }
                        }
                        0x91 => {
                            println!("TYPE: Slot Status (0x91)");
                            if data.len() >= 23 {
                                let channel = data[3];
                                let slot = match channel { 0x01=>1, 0x02=>2, 0x04=>3, 0x08=>4, _=>0 };
                                println!("SLOT {} (channel=0x{:02X})", slot, channel);
                                println!("────────────────────────────────────────");
                                println!("  [4]  status_byte = 0x{:02X}", data[4]);
                                println!("  [5]  byte5       = {} (0x{:02X})", data[5], data[5]);
                                let voltage = u16::from_be_bytes([data[6], data[7]]);
                                println!("  [6-7] voltage    = {} mV ({:.3}V)", voltage, voltage as f32/1000.0);
                                println!("  [8]  byte8       = 0x{:02X}", data[8]);
                                println!("  [9]  byte9       = 0x{:02X}", data[9]);
                                let cap_short = u16::from_be_bytes([data[10], data[11]]);
                                println!("  [10-11] cap_short = {} mAh", cap_short);
                                println!("  [12] byte12      = 0x{:02X}", data[12]);
                                println!("  [13] byte13      = 0x{:02X}", data[13]);
                                let elapsed = u16::from_be_bytes([data[14], data[15]]);
                                println!("  [14-15] elapsed  = {} s ({:.1} min)", elapsed, elapsed as f32/60.0);
                                let resistance = u16::from_be_bytes([data[16], data[17]]);
                                println!("  [16-17] resistance = {} mΩ", resistance);
                                println!("  [18] deltav      = {} mV", data[18]);
                                println!("  [19] byte19      = 0x{:02X}", data[19]);
                                println!("  [20] byte20      = 0x{:02X}", data[20]);
                                println!("  [21] byte21      = 0x{:02X}", data[21]);
                                println!("  [22] checksum    = 0x{:02X}", data[22]);
                                
                                // Interpretation
                                let current_ma = match data[4] {
                                    0x07 => (data[5] as u16) * 10, // Discharge uses ~10x multiplier
                                    _ => (data[5] as u16) * 4,
                                };
                                let state_str = match data[4] {
                                    0x00 if voltage == 0 => "Empty",
                                    0x00 if data[5] > 0 => "Charging (NiMH style)",
                                    0x00 => "Idle",
                                    0x01 => "Charging CC",
                                    0x02 => "Charging CV",
                                    0x03 => "Charging",
                                    0x04 => "Completed",
                                    0x05 => "Charging CV/Trickle",
                                    0x06 => "Charging (alt)",
                                    0x07 => "Discharging",
                                    0x09 => "Paused",
                                    _ => "Unknown",
                                };
                                println!("INTERPRETED: {} @ {}mA", state_str, current_ma);
                            }
                        }
                        0x93 => {
                            println!("TYPE: Start/Stop ACK");
                            if data.len() >= 4 {
                                println!("  action = 0x{:02X}", data[3]);
                            }
                        }
                        0x94 => {
                            println!("TYPE: Config ACK");
                            if data.len() >= 5 {
                                println!("  channel = 0x{:02X}", data[3]);
                                println!("  status  = {} ({})", data[4], 
                                    if data[4] == 1 { "ACCEPTED" } else { "REJECTED" });
                            }
                        }
                        _ => {
                            println!("TYPE: Unknown command 0x{:02X}", cmd);
                        }
                    }
                }
                println!();
            }
        }
    });
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Request status for all slots
    println!("Requesting status for all 4 slots...\n");
    for slot in 1..=4u8 {
        let channel_mask = 1u8 << (slot - 1);
        let status_cmd = MC5000Protocol::build_channel_status_command(channel_mask);
        println!(">>> Slot {} request: {:02X?}", slot, status_cmd);
        protocol.send_command(&status_cmd).await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    
    let remaining = duration.saturating_sub(2);
    println!("\nWaiting {} more seconds...\n", remaining);
    tokio::time::sleep(Duration::from_secs(remaining)).await;
    
    let count = notifications_received.load(Ordering::SeqCst);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Session complete. {} notifications received.", count);
    notifications_task.abort();
    
    Ok(())
}

/// Protocol 3 test: replicate capture 3 sequence in a single BLE session.
/// Sends all 3 configs then a combined start, matching the official app's behaviour.
async fn run_proto3(verbose: bool) -> Result<(), Box<dyn Error>> {
    let (protocol, _peripheral) = connect_to_device(verbose).await?;

    println!("=== Protocol 3 Test ===");
    println!("  Slot 1: Cycle D→C, Li-Ion, 2A");
    println!("  Slot 2: Charge, Li-Ion, 1A, 5000mAh");
    println!("  Slot 4: Break-in, NiMH, 0.3A");
    println!();

    // Send configs first, then a combined start at the end.

    // 1) Send config for slot 4: break-in (NiMH, 300mA)
    let breakin_cfg = ChargeConfig::break_in(4, BatteryChemistry::NiMH, 3000, 300);
    let breakin_cmd = MC5000Protocol::build_charge_config_command(&breakin_cfg);
    println!("Sending break-in config (slot 4)...");
    if verbose { println!("  Packet: {:02X?}", breakin_cmd); }
    protocol.send_command(&breakin_cmd).await?;
    println!("  ✓ Break-in config sent");
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 2) Send config for slot 1: cycle D→C (Li-Ion, 2000mA)
    let cycle_cfg = ChargeConfig::cycle(1, BatteryChemistry::LiIon, 3000, 2000);
    let cycle_cmd = MC5000Protocol::build_charge_config_command(&cycle_cfg);
    println!("Sending cycle config (slot 1)...");
    if verbose { println!("  Packet: {:02X?}", cycle_cmd); }
    protocol.send_command(&cycle_cmd).await?;
    println!("  ✓ Cycle config sent");
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 3) Send config for slot 2: charge (Li-Ion, 1000mA, 5000mAh)
    let charge_cfg = ChargeConfig::new(2, BatteryChemistry::LiIon, OperationMode::Charge, 5000, 1000);
    let charge_cmd = MC5000Protocol::build_charge_config_command(&charge_cfg);
    println!("Sending charge config (slot 2)...");
    if verbose { println!("  Packet: {:02X?}", charge_cmd); }
    protocol.send_command(&charge_cmd).await?;
    println!("  ✓ Charge config sent");
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 4) Send combined start for all 3 slots
    let start_mask: u8 = 0x01 | 0x02 | 0x08; // slots 1, 2, 4
    let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(start_mask));
    println!("Sending start-all (mask=0x{:02X})...", start_mask);
    if verbose { println!("  Packet: {:02X?}", start_cmd); }
    protocol.send_command(&start_cmd).await?;
    println!("  ✓ Start-all sent");

    println!();
    println!("Keeping connection alive with status polling (15s)...\n");

    // Set up notification listener to collect status responses
    use std::sync::Mutex;
    use std::collections::HashMap;
    
    let slot_status: Arc<Mutex<HashMap<u8, mc5000_protocol::MC5000SlotStatus>>> = 
        Arc::new(Mutex::new(HashMap::new()));
    let slot_status_clone = slot_status.clone();
    let verbose_clone = verbose;
    
    let peripheral_clone = _peripheral.clone();
    let notification_task = tokio::spawn(async move {
        if let Ok(mut stream) = peripheral_clone.notifications().await {
            while let Some(notification) = stream.next().await {
                let data = &notification.value;
                if data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                    if let Ok(status) = mc5000_protocol::MC5000SlotStatus::parse_from_response(data) {
                        let slot_id = match data[3] {
                            0x01 => 1, 0x02 => 2, 0x04 => 3, 0x08 => 4, _ => 0,
                        };
                        if slot_id > 0 {
                            if let Ok(mut map) = slot_status_clone.lock() {
                                map.insert(slot_id, status);
                            }
                        }
                    }
                }
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Poll status continuously for 15 seconds (matching capture: ~2s interval)
    for round in 1..=5 {
        // Request status for all 4 slots
        for slot in 1..=4u8 {
            let channel_mask = 1u8 << (slot - 1);
            let status_cmd = MC5000Protocol::build_channel_status_command(channel_mask);
            protocol.send_command(&status_cmd).await?;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        
        // Wait for responses
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // Display current status
        println!("--- Poll #{} ---", round);
        if let Ok(map) = slot_status.lock() {
            for &s in &[1u8, 2, 3, 4] {
                if let Some(status) = map.get(&s) {
                    println!("  Slot {}: {:?} {:.3}V {}mA {}mAh {}s",
                        s, status.state,
                        status.voltage_mv as f32 / 1000.0,
                        status.current_ma,
                        status.capacity_mah,
                        status.elapsed_seconds);
                }
            }
        }
        println!();
        
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Now send stop
    println!("Sending stop-all...");
    let stop_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::StopAll);
    protocol.send_command(&stop_cmd).await?;
    println!("  ✓ Stop-all sent");
    
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Final status check
    for slot in 1..=4u8 {
        let channel_mask = 1u8 << (slot - 1);
        let status_cmd = MC5000Protocol::build_channel_status_command(channel_mask);
        protocol.send_command(&status_cmd).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("\n--- Final Status (after stop) ---");
    if let Ok(map) = slot_status.lock() {
        for &s in &[1u8, 2, 3, 4] {
            if let Some(status) = map.get(&s) {
                println!("  Slot {}: {:?} {:.3}V {}mA {}mAh {}s",
                    s, status.state,
                    status.voltage_mv as f32 / 1000.0,
                    status.current_ma,
                    status.capacity_mah,
                    status.elapsed_seconds);
            }
        }
    }
    
    notification_task.abort();
    println!("\nProto3 test complete.");
    Ok(())
}

/// Reproduce the capture 4 action sequence from protocol 4.md
///
/// Battery configuration (must match for meaningful comparison):
/// - Slot 1: NiMH battery
/// - Slot 2: NiMH battery
/// - Slot 3: Li-Ion battery
/// - Slot 4: Li-Ion battery
///
/// Capture analysis shows the Android app:
/// 1. Maintains CONTINUOUS round-robin status polling (all 4 slots) throughout
/// 2. Uses ONLY 0x00 (stop all), 0x01 (start), 0x03 (start all) for start/stop
/// 3. Sends CONFIG while charging is active (no stop needed to reconfigure a slot)
/// 4. Has ~10-15s of polling between control commands
///
/// Sequence from capture:
/// 1. START_ALL (0x03) — Li-Ion charge applied to all slots
/// 2. CONFIG slot 4 (storage) — update on-the-fly  
/// 3. STOP_ALL (0x00)
/// 4. CONFIG slot 1 (NiMH) → START (0x01) → CONFIG slot 2 (NiCd)
/// 5-10. Further configs and starts for LiFePO4, LiHV, RAM, etc.
async fn run_proto4(verbose: bool, dry_run: bool) -> Result<(), Box<dyn Error>> {
    use std::sync::Mutex;
    use std::collections::HashMap;
    
    println!("=== Protocol 4 Reproduction ===");
    println!("Expected battery layout:");
    println!("  Slot 1: NiMH");
    println!("  Slot 2: NiMH");
    println!("  Slot 3: Li-Ion");
    println!("  Slot 4: Li-Ion");
    println!();

    // Connect (or skip in dry-run mode)
    let protocol;
    let _peripheral;
    let slot_status: Arc<Mutex<HashMap<u8, mc5000_protocol::MC5000SlotStatus>>> = 
        Arc::new(Mutex::new(HashMap::new()));
    let notification_task;

    if dry_run {
        println!("[DRY RUN] Skipping device connection, building packets only.\n");
        protocol = MC5000Protocol::new();
        _peripheral = None;
        notification_task = None;
    } else {
        let (proto, periph) = connect_to_device(verbose).await?;
        
        // Set up notification listener
        let slot_status_clone = slot_status.clone();
        let periph_clone = periph.clone();
        let ntask = tokio::spawn(async move {
            if let Ok(mut stream) = periph_clone.notifications().await {
                while let Some(notification) = stream.next().await {
                    let data = &notification.value;
                    if data.len() >= 10 && data[0] == 0x0F && data[2] == 0x91 {
                        if let Ok(status) = mc5000_protocol::MC5000SlotStatus::parse_from_response(data) {
                            let slot_id = match data[3] {
                                0x01 => 1, 0x02 => 2, 0x04 => 3, 0x08 => 4, _ => 0,
                            };
                            if slot_id > 0 {
                                if let Ok(mut map) = slot_status_clone.lock() {
                                    map.insert(slot_id, status);
                                }
                            }
                        }
                    }
                    // Also display ACKs for config commands
                    if data.len() >= 6 && data[0] == 0x0F && data[2] == 0x94 {
                        println!("  RX ACK: {:02X?} (channel=0x{:02X}, status=0x{:02X})",
                            data, data[3], data[4]);
                    }
                }
            }
        });
        
        protocol = proto;
        _peripheral = Some(periph);
        notification_task = Some(ntask);
    }

    /// Do one round of status polling for all 4 slots (matches capture's round-robin pattern)
    async fn poll_round(
        protocol: &MC5000Protocol,
        dry_run: bool,
    ) -> Result<(), Box<dyn Error>> {
        if dry_run { return Ok(()); }
        for slot in [0x01u8, 0x02, 0x04, 0x08] {
            let cmd = MC5000Protocol::build_channel_status_command(slot);
            protocol.send_command(&cmd).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(())
    }

    /// Send a command with proper polling context (simulating the capture's continuous polling)
    /// Does N rounds of round-robin polling before and after the command.
    async fn send_with_polling(
        protocol: &MC5000Protocol,
        cmd: &[u8],
        rounds_before: usize,
        rounds_after: usize,
        dry_run: bool,
    ) -> Result<(), Box<dyn Error>> {
        if dry_run {
            println!("  TX: {:02X?}", cmd);
            return Ok(());
        }
        for _ in 0..rounds_before {
            poll_round(protocol, false).await?;
        }
        protocol.send_command(cmd).await?;
        for _ in 0..rounds_after {
            poll_round(protocol, false).await?;
        }
        Ok(())
    }

    /// Display current status snapshot
    fn display_status(
        slot_status: &Arc<Mutex<HashMap<u8, mc5000_protocol::MC5000SlotStatus>>>,
        label: &str,
    ) {
        println!("--- Status: {} ---", label);
        if let Ok(map) = slot_status.lock() {
            for &s in &[1u8, 2, 3, 4] {
                if let Some(status) = map.get(&s) {
                    println!("    Slot {}: {:?} {:.3}V {}mA {}mAh R={}mΩ {}s",
                        s, status.state,
                        status.voltage_mv as f32 / 1000.0,
                        status.current_ma,
                        status.capacity_mah,
                        status.resistance_milliohm,
                        status.elapsed_seconds);
                }
            }
        }
        println!();
    }

    /// Do several rounds of polling and display status
    async fn monitor_period(
        protocol: &MC5000Protocol,
        slot_status: &Arc<Mutex<HashMap<u8, mc5000_protocol::MC5000SlotStatus>>>,
        dry_run: bool,
        rounds: usize,
        label: &str,
    ) -> Result<(), Box<dyn Error>> {
        if dry_run { return Ok(()); }
        for _ in 0..rounds {
            poll_round(protocol, false).await?;
        }
        display_status(slot_status, label);
        Ok(())
    }

    /// Stop all charging by sending init sequence (0x74 + 0x65 + 0xFE).
    /// CRITICAL DISCOVERY: 0x93 0x00 (stop-all) is silently IGNORED in-session.
    /// The 0xFE query command acts as a device reset that stops all operations.
    /// The init sequence reliably stops charging in any session state.
    async fn stop_all_via_init(
        protocol: &MC5000Protocol,
        dry_run: bool,
    ) -> Result<(), Box<dyn Error>> {
        if dry_run {
            println!("  TX init sequence (0x74 + 0x65 + 0xFE) → stop all");
            return Ok(());
        }
        // Send init sequence: version(0x74) + settings(0x65) + FE query(0xFE)
        // The 0xFE command resets the device and stops all charging operations.
        protocol.send_init_sequence().await?;
        println!("  ✓ Init sequence sent (device reset/stop)");
        Ok(())
    }

    // ── Initial monitoring (matches capture: ~6 rounds of polling before first command) ──
    if !dry_run {
        println!("Initial monitoring...");
        monitor_period(&protocol, &slot_status, false, 6, "Initial").await?;
    }

    // ========== PHASE 1: Li-Ion charge applied to all slots ==========
    // Capture: START_STOP 0x03 sent BEFORE config (no config needed for "apply to all")
    // Then CONFIG for slot 4 storage comes later.
    // Here we send config first (matching the fact that app has a config screen),
    // then START ALL (0x03).
    println!("━━━ Phase 1: Li-Ion charge → ALL SLOTS (300mA, 5000mAh, 4.2V) ━━━");
    println!("  Expected: Slots 3-4 start charging; Slots 1-2 Battery Type Error");
    {
        let mut config = ChargeConfig::new(1, BatteryChemistry::LiIon, OperationMode::Charge, 5000, 300)
            .for_all_slots();
        config.discharge_current_ma = 2000;
        let config_cmd = MC5000Protocol::build_charge_config_command(&config);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Li-Ion charge config → all slots");
    }
    {
        // START ALL (0x03) — matches capture exactly
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::StartAll);
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start ALL (0x03)");
    }
    // Monitor for ~10s (matches capture's ~14s of polling between phase 1 and phase 2)
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 1 start").await?;

    // ========== PHASE 2: Config slot 4 for storage (while charging, like capture) ==========
    // Capture: CONFIG 0x08 sent while Li-Ion charge is active → then STOP ALL → then monitoring
    println!("━━━ Phase 2: Config slot 4 → Li-Ion Storage (1A/1.5A, timer 120min) ━━━");
    {
        let mut storage_cfg = ChargeConfig::liion_storage(4, 5000, 3800, 1000, 1500);
        storage_cfg.cutoff_timer_min = 120;
        let config_cmd = MC5000Protocol::build_charge_config_command(&storage_cfg);
        println!("  TX config: {:02X?}", config_cmd);
        // CONFIG sent while charging — matches capture order
        send_with_polling(&protocol, &config_cmd, 2, 3, dry_run).await?;
        println!("  ✓ Slot 4: Li-Ion storage config (while charging continues)");
    }
    // Monitor to observe the config taking effect
    monitor_period(&protocol, &slot_status, dry_run, 8, "After storage config (charging continues)").await?;

    // STOP ALL — uses init sequence (0x74+0x65+0xFE) instead of 0x93 0x00
    // because 0x93 0x00 is silently ignored in-session; 0xFE resets the device.
    stop_all_via_init(&protocol, dry_run).await?;
    // Monitor to confirm stop — capture shows ~14s of polling here
    monitor_period(&protocol, &slot_status, dry_run, 10, "After Stop ALL").await?;

    // ========== PHASE 3: Config slot 1 NiMH, start, then config slot 2 NiCd ==========
    // Capture: CONFIG 0x01 → START 0x01 → CONFIG 0x02 → FE query
    println!("━━━ Phase 3: Slot 1 NiMH + Slot 2 NiCd cycle ━━━");
    {
        let mut nimh_cfg = ChargeConfig::nimh(1, 3000, 300)
            .with_trickle_charge(50)
            .with_keep_voltage(1300)
            .with_cutoff_timer(90);
        nimh_cfg.discharge_current_ma = 1000;
        let config_cmd = MC5000Protocol::build_charge_config_command(&nimh_cfg);
        println!("  TX config slot 1: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 1: NiMH charge (300mA, trickle=50mA, keep=1.3V, timer=90min)");
    }
    {
        // START 0x01 — matches capture exactly
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(0x01));
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 3, dry_run).await?;
        println!("  ✓ Start (0x01)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 5, "After slot 1 NiMH start").await?;
    {
        let nicd_cycle_cfg = ChargeConfig::cycle(2, BatteryChemistry::NiCd, 3000, 200);
        let config_cmd = MC5000Protocol::build_charge_config_command(&nicd_cycle_cfg);
        println!("  TX config slot 2: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 2: NiCd cycle (200mA)");
    }
    // FE query (matches capture)
    if !dry_run {
        let fe_cmd = MC5000Protocol::build_fe_query_command();
        protocol.send_command(&fe_cmd).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 3 complete").await?;

    // ========== PHASE 4: Stop slot 4 → Storage mode #2 ==========
    // (Beyond captured data — extrapolating from protocol 4.md description)
    println!("━━━ Phase 4: Li-Ion Storage #2 (700/800mA, timer 120min) ━━━");
    {
        let mut storage2_cfg = ChargeConfig::liion_storage(4, 5000, 3800, 700, 800);
        storage2_cfg.cutoff_timer_min = 120;
        let config_cmd = MC5000Protocol::build_charge_config_command(&storage2_cfg);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 4: Li-Ion storage (700/800mA, timer=120min)");
    }
    {
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(0x01));
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start (0x01)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 4 storage #2").await?;

    // ========== PHASE 5: Stop slot 1 → NiMH charge #2 ==========
    println!("━━━ Phase 5: Stop → NiMH charge (400mA) ━━━");
    stop_all_via_init(&protocol, dry_run).await?;
    monitor_period(&protocol, &slot_status, dry_run, 5, "After Stop for Phase 5").await?;
    {
        let nimh2_cfg = ChargeConfig::nimh(1, 3000, 400);
        let config_cmd = MC5000Protocol::build_charge_config_command(&nimh2_cfg);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 1: NiMH charge (400mA)");
    }
    {
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(0x01));
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start (0x01)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 5 NiMH #2").await?;

    // ========== PHASE 6: Stop all → LiFePO4 charge slot 4 ==========
    println!("━━━ Phase 6: Stop ALL → LiFePO4 charge slot 4 (3A, 5000mAh) ━━━");
    stop_all_via_init(&protocol, dry_run).await?;
    monitor_period(&protocol, &slot_status, dry_run, 5, "After Stop for Phase 6").await?;
    {
        let lfp_cfg = ChargeConfig::lifepo4(4, 5000, 3000);
        let config_cmd = MC5000Protocol::build_charge_config_command(&lfp_cfg);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 4: LiFePO4 charge (3A, 5000mAh, 3.65V)");
    }
    {
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(0x01));
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start (0x01)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 6 LiFePO4").await?;

    // ========== PHASE 7: LiHV charge slot 4 ==========
    println!("━━━ Phase 7: Li-Ion HV charge slot 4 (3A, 5000mAh, 4.35V) ━━━");
    stop_all_via_init(&protocol, dry_run).await?;
    monitor_period(&protocol, &slot_status, dry_run, 5, "After Stop for Phase 7").await?;
    {
        let lihv_cfg = ChargeConfig::liion_hv(4, 5000, 3000);
        let config_cmd = MC5000Protocol::build_charge_config_command(&lihv_cfg);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 4: Li-Ion HV charge (3A, 5000mAh, 4.35V)");
    }
    {
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(0x01));
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start (0x01)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 7 LiHV").await?;

    // ========== PHASE 8: Li-Ion charge all slots ==========
    println!("━━━ Phase 8: Li-Ion charge → ALL SLOTS (300mA, 5000mAh, 4.2V) ━━━");
    println!("  Expected: Slots 1-2 Battery Error, Slots 3-4 charging");
    stop_all_via_init(&protocol, dry_run).await?;
    monitor_period(&protocol, &slot_status, dry_run, 5, "After Stop for Phase 8").await?;
    {
        let mut config = ChargeConfig::new(1, BatteryChemistry::LiIon, OperationMode::Charge, 5000, 300)
            .for_all_slots();
        config.discharge_current_ma = 2000;
        let config_cmd = MC5000Protocol::build_charge_config_command(&config);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Li-Ion charge config → all slots");
    }
    {
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::StartAll);
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start ALL (0x03)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 8 Li-Ion all").await?;

    // ========== PHASE 9: Stop slot 4 → RAM charge ==========
    println!("━━━ Phase 9: RAM charge slot 4 (1A, 3000mAh, 1.65V) ━━━");
    stop_all_via_init(&protocol, dry_run).await?;
    monitor_period(&protocol, &slot_status, dry_run, 5, "After Stop for Phase 9").await?;
    {
        let ram_cfg = ChargeConfig::new(4, BatteryChemistry::RAM, OperationMode::Charge, 3000, 1000);
        let config_cmd = MC5000Protocol::build_charge_config_command(&ram_cfg);
        println!("  TX config: {:02X?}", config_cmd);
        send_with_polling(&protocol, &config_cmd, 2, 2, dry_run).await?;
        println!("  ✓ Slot 4: RAM charge (1A, 3000mAh, 1.65V)");
    }
    {
        let start_cmd = MC5000Protocol::build_start_stop_command(StartStopAction::ChannelMask(0x01));
        println!("  TX start: {:02X?}", start_cmd);
        send_with_polling(&protocol, &start_cmd, 1, 2, dry_run).await?;
        println!("  ✓ Start (0x01)");
    }
    monitor_period(&protocol, &slot_status, dry_run, 8, "After Phase 9 RAM charge").await?;

    // ========== PHASE 10: Final stop ==========
    println!("━━━ Phase 10: Stop all → Final status ━━━");
    stop_all_via_init(&protocol, dry_run).await?;
    monitor_period(&protocol, &slot_status, dry_run, 8, "FINAL").await?;

    if let Some(task) = notification_task {
        task.abort();
    }
    
    println!("Proto4 reproduction complete.");
    Ok(())
}
