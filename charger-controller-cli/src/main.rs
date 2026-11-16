//! MC5000 Charger Controller CLI
//!
//! Command-line interface for the MC5000 battery charger. Every command talks to the
//! charger through [`mc5000_protocol::Session`], the same connection the GUI uses.

use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use mc5000_protocol::{
    BatteryChemistry, ChargeConfig, ConnectMode, MC5000Protocol, MC5000SlotState,
    MC5000SlotStatus, OperationMode, Session, SessionEvent,
};
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

    Runtime::new()?.block_on(run_command(cli))
}

async fn run_command(cli: Cli) -> Result<(), Box<dyn Error>> {
    match &cli.command {
        None => run_monitor(cli.verbose, 2).await,
        Some(Commands::Monitor { interval }) => run_monitor(cli.verbose, *interval).await,
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

    let devices = MC5000Protocol::scan_devices(5).await?;

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
fn mode_name(status: &MC5000SlotStatus) -> String {
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
fn running_mask(statuses: &[Option<MC5000SlotStatus>; 4]) -> u8 {
    statuses
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_ref().is_some_and(MC5000SlotStatus::is_running))
        .fold(0, |mask, (i, _)| mask | 1 << i)
}

fn status_row(slot: u8, status: &MC5000SlotStatus) -> String {
    let state_str = format!("{} {}", state_symbol(&status.state), state_name(&status.state));
    let voltage = format!("{:.3}V", status.voltage_mv as f32 / 1000.0);
    let current = format!("{}mA", status.current_ma);
    let capacity = format!("{}mAh", status.capacity_mah);
    let resistance = status.resistance_milliohm.to_string();
    let time = format_time(status.elapsed_seconds);
    format!(
        "{:>4} {:>12} {:>8} {:>8} {:>9} {:>6} {:>12}",
        slot, state_str, voltage, current, capacity, resistance, time
    )
}

fn print_status_header() {
    println!(
        "{:>4} {:>12} {:>8} {:>8} {:>9} {:>6} {:>12}",
        "Slot", "State", "Voltage", "Current", "Capacity", "R(mΩ)", "Time"
    );
    println!("{}", "-".repeat(65));
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
            MC5000Protocol::build_charge_config_command(&config)
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

    // The FE query selects the slots that 0x93 0x00 then stops; others keep running.
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

async fn run_auto(verbose: bool, slot: Option<u8>, initial_current: u16) -> Result<(), Box<dyn Error>> {
    let slots = selected_slots(slot)?;
    let session = connect(verbose, ConnectMode::Control).await?;

    println!("=== Auto-Charge Mode ===\n");

    let statuses = session.read_all_status(STATUS_TIMEOUT).await;
    let mut configs = Vec::new();
    for slot_num in slots {
        let Some(status) = &statuses[usize::from(slot_num - 1)] else {
            println!("Slot {}: No response, skipping", slot_num);
            continue;
        };
        if status.voltage_mv < 100 {
            println!("Slot {}: Empty, skipping", slot_num);
            continue;
        }
        if !matches!(status.state, MC5000SlotState::Idle | MC5000SlotState::Empty) {
            println!("Slot {}: Already active or finished ({:?}), skipping", slot_num, status.state);
            continue;
        }

        let chemistry = detect_chemistry_from_voltage(status.voltage_mv as u32);
        println!(
            "Slot {}: Detected {} at {:.3}V",
            slot_num,
            chemistry_name(chemistry),
            status.voltage_mv as f32 / 1000.0
        );
        configs.push(ChargeConfig::new(
            slot_num,
            chemistry,
            OperationMode::Charge,
            3000,
            initial_current,
        ));
    }

    if configs.is_empty() {
        println!("\nNothing to start.");
        let _ = session.disconnect().await;
        return Ok(());
    }

    println!("\nStarting charge...");
    session.start_slots(&configs, running_mask(&statuses)).await?;
    println!(
        "  ✓ Charge started on {} slot(s) with {}mA initial current",
        configs.len(),
        initial_current
    );
    println!("\nNote: Run 'charger-cli monitor' to watch progress");

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

/// Prints one received frame with a byte-level breakdown.
fn print_frame(frame: &mc5000_protocol::Frame) {
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
            if data.len() >= 22 {
                println!("  Serial: {}", String::from_utf8_lossy(&data[4..11]));
                println!("  FW: {}.{}", data[16], data[17]);
                println!("  HW: {}.{}", data[18], data[19]);
            }
        }
        0x74 => println!("TYPE: Version Info"),
        0x65 => println!("TYPE: Settings"),
        0xFE => println!("TYPE: Slot Query (0xFE)"),
        0x91 => {
            println!("TYPE: Slot Status (0x91)");
            for (i, byte) in data.iter().enumerate().skip(3) {
                println!("  [{:2}] 0x{:02X} ({})", i, byte, byte);
            }
            match frame.status() {
                Some(s) => println!(
                    "DECODED: slot {} {:?} phase={:?} mode={} {:.3}V {}mA {}mAh {}s R={}mΩ",
                    s.slot_id,
                    s.state,
                    s.phase,
                    mode_name(&s),
                    s.voltage_mv as f32 / 1000.0,
                    s.current_ma,
                    s.capacity_mah,
                    s.elapsed_seconds,
                    s.resistance_milliohm
                ),
                None => println!("DECODED: unknown slot"),
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
        let status_cmd = MC5000Protocol::build_channel_status_command(1 << (slot - 1));
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

    fn status(slot: u8, current_ma: u16) -> Option<MC5000SlotStatus> {
        Some(MC5000SlotStatus {
            slot_id: slot,
            state: if current_ma > 0 { MC5000SlotState::Charging } else { MC5000SlotState::Idle },
            voltage_mv: 3700,
            current_ma,
            capacity_mah: 0,
            elapsed_seconds: 0,
            resistance_milliohm: 0,
            temperature: 0,
            phase: Some(if current_ma > 0 { 2 } else { 0 }),
            mode: None,
            chemistry: None,
        })
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
