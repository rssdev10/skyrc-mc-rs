//! MC5000 Charger Controller CLI
//!
//! Command-line interface for the MC5000 battery charger. Every command talks to the
//! charger through [`skyrc_mc5000::Session`], the same connection the GUI uses.

use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use skyrc_mc5000::{
    charging, BatteryChemistry, ChargeConfig, ConnectMode, Protocol, SlotState,
    SlotStatus, OperationMode, Session, SessionEvent,
};
use tokio::runtime::Runtime;

#[derive(Parser)]
#[command(name = "charger-cli")]
#[command(author = "MC5000 Project")]
#[command(version)]
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
    
    /// Show the charger's model, firmware and hardware versions, settings and slots
    /// (read only; running programs are not touched)
    Info,

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
    
    /// Detect each idle cell's chemistry (Li-Ion at 2.7 V or more, NiMH below 1.5 V) and
    /// charge it at a fixed 500 mA. Other voltages are skipped as ambiguous.
    Auto {
        /// Slot number (1-4), or omit for all slots
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
    },

    /// Detect each idle cell's chemistry, measure its internal resistance with a short
    /// low-current probe, then charge at a current chosen from it (NiMH at most 400 mA,
    /// Li-Ion 1000 mA, or 2000 mA for large cells). Worn cells are charged gently; cells
    /// with too high a resistance are stopped.
    Smart {
        /// Slot number (1-4), or omit for all slots
        #[arg(value_name = "SLOT")]
        slot: Option<u8>,
    },
    
    /// Debug command to test BLE communication and view notifications
    Debug {
        /// Duration in seconds to listen for notifications
        #[arg(short, long, default_value = "10")]
        duration: u64,
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

fn state_symbol(state: &SlotState) -> &'static str {
    match state {
        SlotState::Idle => "○",
        SlotState::Charging | SlotState::ChargingCC | SlotState::ChargingCV => "⚡",
        SlotState::Discharging => "▼",
        SlotState::Completed => "✓",
        SlotState::Paused => "⏸",
        SlotState::Error => "✗",
        SlotState::Empty => "·",
    }
}

fn state_name(state: &SlotState) -> &'static str {
    match state {
        SlotState::Idle => "Idle",
        SlotState::Charging => "Charging",
        SlotState::ChargingCC => "CC Charge",
        SlotState::ChargingCV => "CV Charge",
        SlotState::Discharging => "Discharge",
        SlotState::Completed => "Done",
        SlotState::Paused => "Paused",
        SlotState::Error => "Error",
        SlotState::Empty => "Empty",
    }
}


/// Logging: `RUST_LOG` as usual; `MC5000_VERBOSE` adds the charger library's debug log and
/// `MC5000_DEBUG` its trace log (every frame sent and received).
fn init_logging() {
    let mut builder = env_logger::Builder::from_default_env();
    if std::env::var_os("MC5000_DEBUG").is_some() {
        builder.filter_module("skyrc_mc5000", log::LevelFilter::Trace);
    } else if std::env::var_os("MC5000_VERBOSE").is_some() {
        builder.filter_module("skyrc_mc5000", log::LevelFilter::Debug);
    }
    builder.init();
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    if cli.verbose {
        std::env::set_var("MC5000_VERBOSE", "1");
        println!("[INFO] Verbose mode enabled");
    }
    init_logging();

    Runtime::new()?.block_on(run_command(cli))
}

async fn run_command(cli: Cli) -> Result<(), Box<dyn Error>> {
    match &cli.command {
        None => run_monitor(cli.verbose, 2).await,
        Some(Commands::Monitor { interval }) => run_monitor(cli.verbose, *interval).await,
        Some(Commands::Scan) => run_scan(cli.verbose).await,
        Some(Commands::Info) => run_info(cli.verbose).await,
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
        Some(Commands::Auto { slot }) => run_auto(cli.verbose, *slot).await,
        Some(Commands::Smart { slot }) => run_smart(cli.verbose, *slot).await,
        Some(Commands::Debug { duration }) => run_debug(cli.verbose, *duration).await,
    }
}

/// How long to wait for one slot's status before re-requesting or giving up.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Scans and connects to the first MC5000 found.
async fn connect(verbose: bool, mode: ConnectMode) -> Result<Session, Box<dyn Error>> {
    println!("Scanning for MC5000 devices (5s)...");
    let chargers = Session::scan(5).await?;

    if verbose {
        println!("Found {} charger(s):", chargers.len());
        for dev in &chargers {
            println!("  {} (RSSI: {:?})", dev.name, dev.rssi);
        }
    }

    let charger = chargers.into_iter().next().ok_or("No MC5000 device found")?;
    println!("Connecting to {}...", charger.name);
    let session = Session::connect(&charger, mode).await?;
    println!("Connected!\n");
    Ok(session)
}

async fn run_scan(verbose: bool) -> Result<(), Box<dyn Error>> {
    println!("=== Scanning for Bluetooth devices ===\n");

    let devices = Protocol::scan_devices(5).await?;

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    println!("Found {} devices:", devices.len());
    for dev in &devices {
        let mc5000_tag = if dev.is_mc5000 { " [MC5000]" } else { "" };
        println!("  {} - {} (RSSI: {:?}){}", dev.id, dev.name, dev.rssi, mc5000_tag);

        if verbose {
            println!("    Address: {}", dev.address);
        }
    }

    Ok(())
}

/// The configured program and chemistry the charger reports, e.g. "Li-Ion Discharge".
fn mode_name(status: &SlotStatus) -> String {
    match (status.chemistry, status.mode) {
        (Some(chemistry), Some(mode)) => format!("{} {}", chemistry.name(), mode.name()),
        (None, Some(mode)) => mode.name().to_string(),
        _ => "unknown mode".to_string(),
    }
}

fn is_valid_slot(slot: u8) -> bool {
    (1..=4).contains(&slot)
}

fn check_slot(slot: u8) -> Result<(), Box<dyn Error>> {
    if is_valid_slot(slot) {
        Ok(())
    } else {
        Err(format!("Invalid slot: {}. Must be 1-4", slot).into())
    }
}

/// The slots a command applies to: one slot, or all four.
fn selected_slots(slot: Option<u8>) -> Result<Vec<u8>, Box<dyn Error>> {
    match slot {
        Some(s) => check_slot(s).map(|()| vec![s]),
        None => Ok(vec![1, 2, 3, 4]),
    }
}

/// Mask of slots the charger reports as running a program.
fn running_mask(statuses: &[Option<SlotStatus>; 4]) -> u8 {
    statuses
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_ref().is_some_and(SlotStatus::is_running))
        .fold(0, |mask, (i, _)| mask | 1 << i)
}

/// The charger's error code (status byte 20) as text: "-" for none.
fn error_name(code: Option<u8>) -> String {
    match code {
        None | Some(0) => "-".to_string(),
        Some(c) => skyrc_mc5000::bluetooth::charger_error_text(c)
            .map_or_else(|| format!("code {c}"), str::to_string),
    }
}

fn status_row(slot: u8, status: &SlotStatus) -> String {
    let state_str = format!("{} {}", state_symbol(&status.state), state_name(&status.state));
    let voltage = format!("{:.3}V", status.voltage_mv as f32 / 1000.0);
    let current = format!("{}mA", status.current_ma);
    let capacity = format!("{}mAh", status.capacity_mah);
    let resistance = status.resistance_milliohm.to_string();
    let time = format_time(status.elapsed_seconds);
    format!(
        "{:>4} {:>12} {:>8} {:>8} {:>9} {:>6} {:>12}  {}",
        slot, state_str, voltage, current, capacity, resistance, time, error_name(status.error)
    )
}

fn print_status_header() {
    println!(
        "{:>4} {:>12} {:>8} {:>8} {:>9} {:>6} {:>12}  Charger error",
        "Slot", "State", "Voltage", "Current", "Capacity", "R(mΩ)", "Time"
    );
    println!("{}", "-".repeat(80));
}

/// Device information for the customer: what the charger is, its versions and settings,
/// and what each slot holds. Connects read-only (handshake only).
async fn run_info(verbose: bool) -> Result<(), Box<dyn Error>> {
    let session = connect(verbose, ConnectMode::Monitor).await?;
    let info = session.device_info(Duration::from_secs(3)).await;
    let settings = session.system_settings(Duration::from_secs(3)).await;
    let slots = session.read_all_status(Duration::from_secs(2)).await;
    let _ = session.disconnect().await;

    println!("\n{}", info_report(session.name(), session.id(), info.as_ref().ok(), settings.as_ref().ok(), &slots));
    if let Err(e) = &info {
        println!("(device info not received: {e})");
    }
    if let Err(e) = &settings {
        println!("(settings not received: {e})");
    }
    Ok(())
}

/// The text `info` prints. Separate from the I/O so it can be tested.
fn info_report(
    name: &str,
    id: &str,
    info: Option<&skyrc_mc5000::DeviceInfo>,
    settings: Option<&skyrc_mc5000::SystemSettings>,
    slots: &[Option<SlotStatus>; 4],
) -> String {
    let unknown = || "unknown".to_string();
    let mut out = Vec::new();
    out.push("Charger".to_string());
    out.push(format!("  Bluetooth name:   {name}"));
    out.push(format!(
        "  Bluetooth ID:     {id}{}",
        if cfg!(target_os = "macos") { " (assigned by macOS, not the hardware address)" } else { "" }
    ));
    let model = info.map(|i| {
        if i.model_id == "100214" {
            format!("SkyRC MC5000 (model ID {})", i.model_id)
        } else {
            format!("model ID {}", i.model_id)
        }
    });
    out.push(format!("  Model:            {}", model.unwrap_or_else(unknown)));
    out.push(format!("  Firmware version: {}", info.map(|i| i.firmware_version.clone()).unwrap_or_else(unknown)));
    out.push(format!("  Hardware version: {}", info.map(|i| i.hardware_version.clone()).unwrap_or_else(unknown)));
    out.push(format!(
        "  OEM code:         {}",
        info.map(|i| if i.oem == "0000" { "0000 (SkyRC)".to_string() } else { i.oem.clone() })
            .unwrap_or_else(unknown)
    ));
    out.push("  Serial number:    not reported by the charger".to_string());
    out.push(format!(
        "  Device code:      {}",
        info.map(|i| i.code.iter().map(|b| format!("{b:02X}")).collect::<String>()).unwrap_or_else(unknown)
    ));

    if let Some(s) = settings {
        out.push(String::new());
        out.push("Charger settings (raw values as the charger reports them)".to_string());
        out.push(format!(
            "  System beep {}, notification beep {}, end beep {}, LCD {}, warning {}, sleep {}",
            s.system_beep, s.notification_beep, s.end_beep, s.lcd, s.warning, s.sleep
        ));
    }

    out.push(String::new());
    out.push("Slots".to_string());
    for (i, slot) in slots.iter().enumerate() {
        let line = match slot {
            None => "no answer".to_string(),
            Some(s) if s.state == SlotState::Empty => "empty".to_string(),
            Some(s) => {
                let mut line = format!(
                    "{:.3} V, {}, program: {}",
                    s.voltage_mv as f32 / 1000.0,
                    state_name(&s.state),
                    mode_name(s)
                );
                if s.temperature_c > 0 {
                    line.push_str(&format!(", {} °C", s.temperature_c));
                }
                if let Some(code @ 1..) = s.error {
                    line.push_str(&format!(", charger error: {}", error_name(Some(code))));
                }
                line
            }
        };
        out.push(format!("  {}: {line}", i + 1));
    }
    out.join("\n")
}

async fn run_status(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn Error>> {
    let slots = selected_slots(slot)?;
    let session = connect(verbose, ConnectMode::Monitor).await?;

    let mut rows = Vec::with_capacity(slots.len());
    for &slot_num in &slots {
        rows.push((slot_num, session.read_status(slot_num, STATUS_TIMEOUT).await));
    }

    println!("=== Slot Status ===\n");
    print_status_header();
    for (slot_num, result) in &rows {
        match result {
            Ok(status) => println!("{}", status_row(*slot_num, status)),
            Err(_) => println!("Slot {}: No response", slot_num),
        }
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// Connects, configures `config` and starts it, keeping slots that are already running.
async fn start_operation(
    verbose: bool,
    title: &str,
    details: &[String],
    config: ChargeConfig,
) -> Result<(), Box<dyn Error>> {
    let session = connect(verbose, ConnectMode::Control).await?;

    println!("=== Starting {} ===", title);
    for line in details {
        println!("{}", line);
    }
    println!();

    let statuses = session.read_all_status(STATUS_TIMEOUT).await;
    let keep_running = running_mask(&statuses) & !config.channel_bitmask;
    for (i, status) in statuses.iter().enumerate() {
        let bit = 1u8 << i;
        if bit & config.channel_bitmask != 0 {
            continue;
        }
        match status {
            Some(s) if s.is_running() => println!(
                "Slot {} is running ({}, {}mA); it keeps running.",
                i + 1,
                mode_name(s),
                s.current_ma
            ),
            None => println!(
                "Note: slot {} did not answer; if it is running, the start command may stop it.",
                i + 1
            ),
            _ => {}
        }
    }

    if verbose {
        println!(
            "  Config packet: {:02x?}",
            Protocol::build_charge_config_command(&config)
        );
        println!("  Start mask: 0x{:02X}", keep_running | config.channel_bitmask);
    }

    println!("Sending configuration and start...");
    session.start_slots(std::slice::from_ref(&config), keep_running).await?;
    println!("  ✓ Configuration and start sent");

    // Give the charger time to begin, then confirm from its own status report.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let slot = (1..=4u8)
        .find(|s| config.channel_bitmask == 1 << (s - 1))
        .unwrap_or(1);
    match session.read_status(slot, STATUS_TIMEOUT).await {
        Ok(status) if status.is_running() => println!(
            "\n✓ SUCCESS: slot {} is running {} ({}): {:.3}V {}mA",
            slot,
            mode_name(&status),
            state_name(&status.state),
            status.voltage_mv as f32 / 1000.0,
            status.current_ma
        ),
        Ok(status) => println!(
            "\n⚠ WARNING: slot {} is not running ({}, {:.3}V). Check the charger's display.",
            slot,
            state_name(&status.state),
            status.voltage_mv as f32 / 1000.0
        ),
        Err(e) => println!("\n⚠ WARNING: could not read slot {} status: {}", slot, e),
    }

    let _ = session.disconnect().await;
    Ok(())
}

async fn run_charge(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    check_slot(slot)?;
    let details = [
        format!("Slot: {}", slot),
        format!("Chemistry: {}", chemistry_name(chemistry)),
        format!("Current: {}mA", current),
        format!("Capacity: {}mAh", capacity),
    ];
    let config = ChargeConfig::new(slot, chemistry, OperationMode::Charge, capacity, current);
    start_operation(verbose, "Charge", &details, config).await
}

async fn run_discharge(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    check_slot(slot)?;
    let details = [
        format!("Slot: {}", slot),
        format!("Chemistry: {}", chemistry_name(chemistry)),
        format!("Discharge Current: {}mA", current),
        format!("Capacity: {}mAh", capacity),
    ];
    let config = ChargeConfig::discharge(slot, chemistry, capacity, current);
    start_operation(verbose, "Discharge", &details, config).await
}

async fn run_cycle(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    check_slot(slot)?;
    let details = [
        format!("Slot: {}", slot),
        format!("Chemistry: {}", chemistry_name(chemistry)),
        format!("Charge Current: {}mA", current),
        format!("Capacity: {}mAh", capacity),
    ];
    let config = ChargeConfig::cycle(slot, chemistry, capacity, current);
    start_operation(verbose, "Cycle", &details, config).await
}

async fn run_refresh(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    charge_current: u16,
    discharge_current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    check_slot(slot)?;
    // Refresh is only for NiMH and NiCd
    if !matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd) {
        return Err("Refresh mode is only available for NiMH and NiCd chemistries".into());
    }
    let details = [
        format!("Slot: {}", slot),
        format!("Chemistry: {}", chemistry_name(chemistry)),
        format!("Charge Current: {}mA", charge_current),
        format!("Discharge Current: {}mA", discharge_current),
        format!("Capacity: {}mAh", capacity),
    ];
    let config =
        ChargeConfig::refresh(slot, chemistry, capacity, charge_current, discharge_current);
    start_operation(verbose, "Refresh", &details, config).await
}

async fn run_break_in(
    verbose: bool,
    slot: u8,
    chemistry: BatteryChemistry,
    current: u16,
    capacity: u16,
) -> Result<(), Box<dyn Error>> {
    check_slot(slot)?;
    // Break-in is only for NiMH and NiCd
    if !matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd) {
        return Err("Break-in mode is only available for NiMH and NiCd chemistries".into());
    }
    let details = [
        format!("Slot: {}", slot),
        format!("Chemistry: {}", chemistry_name(chemistry)),
        format!("Charge Current: {}mA", current),
        format!("Capacity: {}mAh", capacity),
    ];
    let config = ChargeConfig::break_in(slot, chemistry, capacity, current);
    start_operation(verbose, "Break-In", &details, config).await
}

async fn run_stop(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn Error>> {
    if let Some(s) = slot {
        check_slot(s)?;
    }
    let session = connect(verbose, ConnectMode::Control).await?;

    // One FE stop per slot (a multi-slot FE mask stops only the lowest); others keep running.
    let stopped = match slot {
        None => {
            println!("Stopping all slots...");
            session.stop_slots(0x0F).await?;
            println!("  ✓ Stop all command sent");
            0x0F
        }
        Some(s) => {
            let bit = 1u8 << (s - 1);
            println!("Stopping slot {}...", s);
            session.stop_slots(bit).await?;
            println!("  ✓ Stop command sent for slot {}", s);
            bit
        }
    };

    tokio::time::sleep(Duration::from_secs(1)).await;
    println!();
    print_status_header();
    for slot_num in (1..=4u8).filter(|s| stopped & (1 << (s - 1)) != 0) {
        match session.read_status(slot_num, STATUS_TIMEOUT).await {
            Ok(status) => println!("{}", status_row(slot_num, &status)),
            Err(_) => println!("Slot {}: No response", slot_num),
        }
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// The idle, non-empty cells in `slots` with their detected chemistry; prints why the
/// others are skipped. Detection is shared with the GUI (`skyrc_mc5000::charging`).
fn detect_cells(
    slots: &[u8],
    statuses: &[Option<SlotStatus>; 4],
) -> Vec<(u8, BatteryChemistry, f32)> {
    let mut cells = Vec::new();
    for &slot_num in slots {
        let Some(status) = &statuses[usize::from(slot_num - 1)] else {
            println!("Slot {slot_num}: no response, skipping");
            continue;
        };
        let voltage = status.voltage_mv as f32 / 1000.0;
        if status.is_running() {
            println!("Slot {slot_num}: already running ({}), skipping", mode_name(status));
            continue;
        }
        match charging::detect_chemistry(voltage) {
            Ok(chemistry) => {
                println!("Slot {slot_num}: {} at {voltage:.3} V", chemistry_name(chemistry));
                cells.push((slot_num, chemistry, voltage));
            }
            Err(reason) => println!("Slot {slot_num}: skipped: {reason}"),
        }
    }
    cells
}

async fn run_auto(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn Error>> {
    let slots = selected_slots(slot)?;
    let session = connect(verbose, ConnectMode::Control).await?;

    println!("=== Auto: detected chemistry, {} mA ===\n", charging::AUTO_CURRENT_MA);
    let statuses = session.read_all_status(STATUS_TIMEOUT).await;
    let configs: Vec<ChargeConfig> = detect_cells(&slots, &statuses)
        .into_iter()
        .map(|(slot, chemistry, _)| {
            ChargeConfig::new(slot, chemistry, OperationMode::Charge, 3000, charging::AUTO_CURRENT_MA)
        })
        .collect();

    if configs.is_empty() {
        println!("\nNothing to start.");
        let _ = session.disconnect().await;
        return Ok(());
    }
    session.start_slots(&configs, running_mask(&statuses)).await?;
    println!(
        "\n✓ Charging {} slot(s) at {} mA. Run 'charger-cli monitor' to watch progress.",
        configs.len(),
        charging::AUTO_CURRENT_MA
    );
    let _ = session.disconnect().await;
    Ok(())
}

/// What Smart decided for one probed slot.
#[derive(Debug, PartialEq)]
enum SmartOutcome {
    Charge(charging::SmartCurrent),
    Refused(charging::SmartRefusal),
    /// No reading in time: the cell keeps charging at the probe current.
    NoReading,
    /// The probe ended before a reading (the charger stopped it, or never started it).
    Ended,
}

/// Smart's decision for a probe slot's latest status, or `None` to keep waiting.
/// `seen_running`: the charger has reported the probe running at least once (it takes a
/// couple of seconds to start, so an idle report before that means nothing yet).
fn smart_decision(
    chemistry: BatteryChemistry,
    status: &SlotStatus,
    seen_running: bool,
    timed_out: bool,
) -> Option<SmartOutcome> {
    if !status.is_running() {
        return (seen_running || timed_out).then_some(SmartOutcome::Ended);
    }
    let measured = status.resistance_milliohm > 0
        && status.elapsed_seconds >= charging::SMART_PROBE_SETTLE_S;
    if measured {
        return Some(match charging::smart_current_ma(chemistry, status.resistance_milliohm) {
            Ok(current) => SmartOutcome::Charge(current),
            Err(refusal) => SmartOutcome::Refused(refusal),
        });
    }
    timed_out.then_some(SmartOutcome::NoReading)
}

async fn run_smart(verbose: bool, slot: Option<u8>) -> Result<(), Box<dyn Error>> {
    let slots = selected_slots(slot)?;
    let session = connect(verbose, ConnectMode::Control).await?;

    println!("=== Smart: probe, measure resistance, choose the current ===\n");
    let statuses = session.read_all_status(STATUS_TIMEOUT).await;
    let cells = detect_cells(&slots, &statuses);
    if cells.is_empty() {
        println!("\nNothing to start.");
        let _ = session.disconnect().await;
        return Ok(());
    }

    let probes: Vec<ChargeConfig> = cells
        .iter()
        .map(|&(slot, chemistry, _)| {
            ChargeConfig::new(slot, chemistry, OperationMode::Charge, 3000, charging::probe_current_ma(chemistry))
        })
        .collect();
    session.start_slots(&probes, running_mask(&statuses)).await?;
    println!("\nProbing {} slot(s) (NiMH 200 mA, Li-Ion 300 mA)...", probes.len());

    let deadline = Instant::now() + Duration::from_secs(charging::SMART_PROBE_TIMEOUT_S);
    // (slot, chemistry, seen running)
    let mut pending: Vec<(u8, BatteryChemistry, bool)> =
        cells.iter().map(|&(s, c, _)| (s, c, false)).collect();
    while !pending.is_empty() {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let timed_out = Instant::now() >= deadline;
        let mut still_waiting = Vec::new();
        for (slot_num, chemistry, seen_running) in pending {
            let Ok(status) = session.read_status(slot_num, STATUS_TIMEOUT).await else {
                if timed_out {
                    println!("Slot {slot_num}: no status; left at the probe current");
                } else {
                    still_waiting.push((slot_num, chemistry, seen_running));
                }
                continue;
            };
            let seen_running = seen_running || status.is_running();
            match smart_decision(chemistry, &status, seen_running, timed_out) {
                None => still_waiting.push((slot_num, chemistry, seen_running)),
                Some(SmartOutcome::Charge(choice)) => {
                    let config = ChargeConfig::new(slot_num, chemistry, OperationMode::Charge, 3000, choice.current_ma);
                    session.start_slots(&[config], session.active_mask()).await?;
                    println!(
                        "Slot {slot_num}: {} mΩ → charging at {} mA{}",
                        status.resistance_milliohm,
                        choice.current_ma,
                        if choice.worn { " (worn cell: charging gently)" } else { "" }
                    );
                }
                Some(SmartOutcome::Refused(reason)) => {
                    session.stop_slots(1 << (slot_num - 1)).await?;
                    println!("Slot {slot_num}: stopped: {reason}");
                }
                Some(SmartOutcome::NoReading) => {
                    println!("Slot {slot_num}: resistance not measured; keeps charging at the probe current")
                }
                Some(SmartOutcome::Ended) => println!(
                    "Slot {slot_num}: the charger ended the probe ({}); not charged",
                    error_name(status.error)
                ),
            }
        }
        pending = still_waiting;
    }
    println!("\nRun 'charger-cli monitor' to watch progress.");
    let _ = session.disconnect().await;
    Ok(())
}

/// One monitor line: `[12:00:00] S1: ⚡ 3.55V 1505mA  S2: ...`.
fn monitor_line(session: &Session) -> String {
    let mut line = format!("[{}] ", chrono::Local::now().format("%H:%M:%S"));
    for slot in 1..=4u8 {
        match session.latest_status(slot) {
            Some((status, _)) if status.voltage_mv > 0 => line.push_str(&format!(
                "S{}: {} {:.2}V {}mA  ",
                slot,
                state_symbol(&status.state),
                status.voltage_mv as f32 / 1000.0,
                status.current_ma
            )),
            Some(_) => line.push_str(&format!("S{}: ·  ", slot)),
            None => line.push_str(&format!("S{}: -  ", slot)),
        }
    }
    line
}

/// Automatic reconnect attempts after the link is lost.
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

async fn run_monitor(verbose: bool, interval: u64) -> Result<(), Box<dyn Error>> {
    let mut session = connect(verbose, ConnectMode::Monitor).await?;

    println!("=== Continuous Monitoring ===");
    println!("Press Ctrl+C to stop\n");

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        println!("\nStopping monitor...");
    })
    .ok();

    let interval = Duration::from_secs(interval.max(1));
    let mut events = session.subscribe();
    session.start_polling(interval);

    // Let the first polling round complete before the first line.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut next_print = Instant::now();

    while running.load(Ordering::SeqCst) {
        loop {
            use tokio::sync::broadcast::error::TryRecvError;
            match events.try_recv() {
                Ok(SessionEvent::Disconnected) | Err(TryRecvError::Closed) => {
                    println!("Connection to the charger was lost; reconnecting...");
                    session = reconnect(verbose, &running).await?;
                    events = session.subscribe();
                    session.start_polling(interval);
                    break;
                }
                Ok(SessionEvent::Frame(_)) | Err(TryRecvError::Lagged(_)) => {}
                Err(TryRecvError::Empty) => break,
            }
        }
        if Instant::now() >= next_print {
            println!("{}", monitor_line(&session));
            next_print += interval;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// Reconnects (read-only) after a lost link, with growing delays between attempts.
async fn reconnect(verbose: bool, running: &AtomicBool) -> Result<Session, Box<dyn Error>> {
    let mut last_error = String::new();
    for attempt in 1..=MAX_RECONNECT_ATTEMPTS {
        if !running.load(Ordering::SeqCst) {
            return Err("Stopped while reconnecting".into());
        }
        // First attempt at once; then 2 s, 4 s, ...
        tokio::time::sleep(Duration::from_secs(2 * u64::from(attempt - 1))).await;
        match connect(verbose, ConnectMode::Monitor).await {
            Ok(session) => return Ok(session),
            Err(e) => {
                println!("Reconnect attempt {attempt}/{MAX_RECONNECT_ATTEMPTS} failed: {e}");
                last_error = e.to_string();
            }
        }
    }
    Err(format!("Connection to the charger was lost; reconnecting failed: {last_error}").into())
}

fn format_time(seconds: u32) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{}:{:02}", minutes, secs)
    }
}

/// Prints one received frame with a byte-level breakdown.
fn print_frame(frame: &skyrc_mc5000::Frame) {
    let data = frame.bytes();
    let now = chrono::Local::now().format("%H:%M:%S%.3f");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("[{}] FRAME ({} bytes)", now, data.len());
    println!("RAW: {:02X?}", data);
    println!("HEADER: start=0x0F len={} cmd=0x{:02X}", data[1], data[2]);

    match frame.command() {
        0x06 => println!("TYPE: Greeting (unsolicited)"),
        0x57 => {
            println!("TYPE: Device Info Response");
            match skyrc_mc5000::DeviceInfo::parse_from_response(data) {
                Ok(info) => println!(
                    "  Model: {}  FW: {}  HW: {}",
                    info.model_id, info.firmware_version, info.hardware_version
                ),
                Err(e) => println!("  (not decoded: {e})"),
            }
        }
        0x74 => println!(
            "TYPE: Password check: {}",
            if data.get(4) == Some(&1) { "accepted" } else { "rejected" }
        ),
        0x65 => println!("TYPE: Firmware-update flag reply"),
        0x96 => match skyrc_mc5000::SystemSettings::parse_from_response(data) {
            Ok(s) => println!("TYPE: System settings {s:?}"),
            Err(_) => println!("TYPE: System settings (not decoded)"),
        },
        0xFE => println!(
            "TYPE: Stop reply: {}",
            if data.get(4) == Some(&1) { "stopped" } else { "FAILED" }
        ),
        0x91 => {
            println!("TYPE: Slot Status (0x91)");
            for (i, byte) in data.iter().enumerate().skip(3) {
                println!("  [{:2}] 0x{:02X} ({})", i, byte, byte);
            }
            match frame.status() {
                Some(s) => println!(
                    "DECODED: slot {} {:?} phase={:?} mode={} {:.3}V {}mA {}mAh {}s R={}mΩ T={}°C error={}",
                    s.slot_id,
                    s.state,
                    s.phase,
                    mode_name(&s),
                    s.voltage_mv as f32 / 1000.0,
                    s.current_ma,
                    s.capacity_mah,
                    s.elapsed_seconds,
                    s.resistance_milliohm,
                    s.temperature_c,
                    error_name(s.error),
                ),
                None => println!("DECODED: unknown slot"),
            }
        }
        0x93 => println!("TYPE: 0x93 reply (the charger is not known to answer 0x93)"),
        0x94 => {
            println!("TYPE: Config ACK");
            if data.len() >= 5 {
                println!("  channel = 0x{:02X}", data[3]);
                println!(
                    "  status  = {} ({})",
                    data[4],
                    if data[4] == 1 { "ACCEPTED" } else { "REJECTED" }
                );
            }
        }
        cmd => println!("TYPE: Unknown command 0x{:02X}", cmd),
    }
    println!();
}

/// Debug command to test BLE communication - Protocol Validation Mode
async fn run_debug(verbose: bool, duration: u64) -> Result<(), Box<dyn Error>> {
    let session = connect(verbose, ConnectMode::Monitor).await?;

    println!("=== Protocol Validation Mode ===");
    println!("Listening for {} seconds...\n", duration);

    let frames_received = Arc::new(AtomicUsize::new(0));
    let counter = frames_received.clone();
    let mut events = session.subscribe();
    let printer = tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match events.recv().await {
                Ok(SessionEvent::Frame(frame)) => {
                    counter.fetch_add(1, Ordering::SeqCst);
                    print_frame(&frame);
                }
                Ok(SessionEvent::Disconnected) | Err(RecvError::Closed) => {
                    println!("Charger disconnected.");
                    break;
                }
                Err(RecvError::Lagged(missed)) => println!("(missed {} frames)", missed),
            }
        }
    });

    println!("Requesting status for all 4 slots...\n");
    for slot in 1..=4u8 {
        let status_cmd = Protocol::build_channel_status_command(1 << (slot - 1));
        println!(">>> Slot {} request: {:02X?}", slot, status_cmd);
        session.request_status(slot).await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    let remaining = duration.saturating_sub(2);
    println!("\nWaiting {} more seconds...\n", remaining);
    tokio::time::sleep(Duration::from_secs(remaining)).await;

    printer.abort();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "Session complete. {} frames received.",
        frames_received.load(Ordering::SeqCst)
    );
    let _ = session.disconnect().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(slot: u8, current_ma: u16) -> Option<SlotStatus> {
        Some(SlotStatus {
            slot_id: slot,
            state: if current_ma > 0 { SlotState::Charging } else { SlotState::Idle },
            voltage_mv: 3700,
            current_ma,
            capacity_mah: 0,
            elapsed_seconds: 0,
            resistance_milliohm: 0,
            temperature_c: 0,
            phase: Some(if current_ma > 0 { 2 } else { 0 }),
            mode: None,
            chemistry: None,
            error: Some(0),
        })
    }

    #[test]
    fn info_report_lists_versions_and_slots() {
        // Replies read from an MC5000 with firmware 1.83.
        let info = skyrc_mc5000::DeviceInfo::parse_from_response(&[
            0x0F, 0x13, 0x57, 0x00, 0x31, 0x30, 0x30, 0x32, 0x31, 0x34, 0x04, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x53, 0x01, 0x00, 0x7F, 0x57,
        ])
        .unwrap();
        let settings = skyrc_mc5000::SystemSettings::parse_from_response(&[
            0x0F, 0x09, 0x96, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x98,
        ])
        .unwrap();
        let mut full = status(2, 0).unwrap();
        full.error = Some(13);
        let mut empty = status(1, 0).unwrap();
        empty.state = SlotState::Empty;
        let report = info_report("#Charger-FF34", "id", Some(&info), Some(&settings), &[Some(empty), Some(full), None, status(4, 500)]);
        for expected in [
            "SkyRC MC5000 (model ID 100214)",
            "Firmware version: 1.83",
            "Hardware version: 1",
            "OEM code:         0000 (SkyRC)",
            "Serial number:    not reported by the charger",
            "Device code:      3130303231340400000000015301007F",
            "LCD 1",
            "1: empty",
            "2: 3.700 V, Idle, program: unknown mode, charger error: Fully charged",
            "3: no answer",
            "4: 3.700 V, Charging",
        ] {
            assert!(report.contains(expected), "missing {expected:?} in:\n{report}");
        }
    }

    #[test]
    fn smart_waits_for_a_settled_reading_then_decides() {
        let probe = |r: u16, elapsed: u32| {
            let mut s = status(2, 200).unwrap();
            s.resistance_milliohm = r;
            s.elapsed_seconds = elapsed;
            s
        };
        let nimh = BatteryChemistry::NiMH;
        assert_eq!(smart_decision(nimh, &probe(0, 10), true, false), None, "no reading yet");
        assert_eq!(smart_decision(nimh, &probe(336, 2), true, false), None, "reading not settled");
        assert_eq!(
            smart_decision(nimh, &probe(336, 6), true, false),
            Some(SmartOutcome::Charge(charging::SmartCurrent { current_ma: 250, worn: false }))
        );
        assert_eq!(
            smart_decision(nimh, &probe(646, 6), true, false),
            Some(SmartOutcome::Charge(charging::SmartCurrent { current_ma: 200, worn: true }))
        );
        assert_eq!(
            smart_decision(nimh, &probe(1200, 6), true, false),
            Some(SmartOutcome::Refused(charging::SmartRefusal::TooResistive { resistance_mohm: 1200 }))
        );
        assert_eq!(smart_decision(nimh, &probe(0, 30), true, true), Some(SmartOutcome::NoReading));
        let idle = status(2, 0).unwrap();
        // The charger takes a couple of seconds to start the probe: idle means nothing yet.
        assert_eq!(smart_decision(nimh, &idle, false, false), None, "not started yet");
        assert_eq!(smart_decision(nimh, &idle, true, false), Some(SmartOutcome::Ended), "ended");
        assert_eq!(smart_decision(nimh, &idle, false, true), Some(SmartOutcome::Ended), "never started");
    }

    #[test]
    fn running_mask_counts_only_slots_with_current() {
        let statuses = [status(1, 500), status(2, 0), None, status(4, 1200)];
        assert_eq!(running_mask(&statuses), 0b1001);
    }

    #[test]
    fn selected_slots_validates_range() {
        assert_eq!(selected_slots(None).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(selected_slots(Some(3)).unwrap(), vec![3]);
        assert!(selected_slots(Some(5)).is_err());
    }
}
