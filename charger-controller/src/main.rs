use iced::window;

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
mod slot_persist;
mod config_store;
mod control;
mod auto_charge;

use app::ChargerApp;
use control::{ControlOptions, Source};

/// MC5000 Battery Charger Controller.
///
/// Headless mode runs the real GUI app with an invisible window and drives it through a
/// text control channel, so scripts and agents exercise exactly what a user clicks.
/// Feedback is printed as JSON lines. Subcommands are ready-made control scripts.
#[derive(Parser)]
#[command(name = "charger-controller")]
#[command(version)]
#[command(about = "MC5000 Battery Charger Controller with GUI")]
struct Cli {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Run without a visible window; read control commands from stdin (or run the
    /// subcommand's script) and print JSON feedback. Type `help` for the commands.
    #[arg(short = 'H', long)]
    headless: bool,

    /// Keep the window visible but also accept control commands on stdin.
    #[arg(long, conflicts_with = "headless")]
    control: bool,

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

    /// Start charging a slot (through the Configure dialog)
    Start {
        /// Slot number (1-4)
        #[arg(value_name = "SLOT")]
        slot: u8,

        /// Battery chemistry: liion, liionhv, lifepo4, nimh, nicd, eneloop, nizn, ram, lto, naion
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

    /// Press the Auto button: detect chemistry and charge idle slots at 500 mA
    Auto,

    /// Open GUI (default if no command specified)
    Gui,
}

/// Script steps shared by every subcommand that needs a connection.
fn connect_steps() -> Vec<String> {
    ["wait-until scanned 20", "connect", "wait-until connected 70", "wait-until readings 15"]
        .map(String::from)
        .to_vec()
}

/// The control script a subcommand runs, or `None` for the interactive GUI.
fn script_for(command: &Commands) -> Result<Option<Vec<String>>, String> {
    let check_slot = |slot: u8| {
        if (1..=4).contains(&slot) {
            Ok(())
        } else {
            Err(format!("Invalid slot: {slot}. Must be 1-4"))
        }
    };
    let mut script = Vec::new();
    match command {
        Commands::Gui => return Ok(None),
        Commands::Scan => script.extend(["wait-until scanned 20".into(), "devices".into()]),
        Commands::Status { slot } => {
            if let Some(slot) = slot {
                check_slot(*slot)?;
            }
            script.extend(connect_steps());
            script.push("state".into());
        }
        Commands::Monitor { interval } => {
            script.extend(connect_steps());
            script.push(format!("watch {}", interval.max(&1)));
        }
        Commands::Start { slot, chemistry, current, capacity } => {
            check_slot(*slot)?;
            if control::parse_chemistry(chemistry).is_none() {
                return Err(format!("Unknown chemistry: {chemistry}"));
            }
            script.extend(connect_steps());
            script.push(format!(
                "start {slot} chemistry={chemistry} mode=charge current={current} capacity={capacity}"
            ));
            script.extend(["wait 4".into(), "state".into()]);
        }
        Commands::Stop { slot } => {
            script.extend(connect_steps());
            match slot {
                Some(slot) => {
                    check_slot(*slot)?;
                    script.push(format!("stop {slot}"));
                }
                None => script.push("stop".into()),
            }
            script.extend(["wait 3".into(), "state".into()]);
        }
        Commands::Auto => {
            script.extend(connect_steps());
            script.extend(["auto".into(), "wait 5".into(), "state".into()]);
        }
    }
    script.push("quit".into());
    Ok(Some(script))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

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

    let script = match &cli.command {
        Some(command) => script_for(command)?,
        None => None,
    };
    // A subcommand other than `gui` implies headless mode.
    let headless = cli.headless || script.is_some();
    let control = match script {
        Some(lines) => Some(ControlOptions { source: Source::Script(Arc::new(lines)), quit_at_end: true }),
        None if headless => Some(ControlOptions { source: Source::Stdin, quit_at_end: true }),
        None if cli.control => Some(ControlOptions { source: Source::Stdin, quit_at_end: false }),
        None => None,
    };

    run_gui(cli.verbose, headless, control)?;

    if control::FAILED.load(std::sync::atomic::Ordering::SeqCst) {
        std::process::exit(1);
    }
    Ok(())
}

fn run_gui(
    verbose: bool,
    headless: bool,
    control: Option<ControlOptions>,
) -> Result<(), Box<dyn std::error::Error>> {
    if verbose {
        println!("[INFO] Starting MC5000 Charger Controller GUI");
        println!("[INFO] Window configuration:");
        println!("[INFO]   Size: 1200x800");
        println!("[INFO]   Position: Centered");
        println!("[INFO]   Min size: 800x600");
        println!("[INFO]   Icon: img/mc5000.jpg");
        println!("[INFO]   Theme: Dark");
        if headless {
            println!("[INFO]   Headless: window hidden, control channel on");
        }
        println!("════════════════════════════════════════════════════════════════════════\n");
    }

    // Load window icon
    let icon = load_icon();

    iced::application(
        move || ChargerApp::new_with(control.clone()),
        ChargerApp::update,
        ChargerApp::view,
    )
    .title(ChargerApp::title)
    .subscription(ChargerApp::subscription)
    .theme(ChargerApp::theme)
    .window(window::Settings {
        size: iced::Size::new(1200.0, 800.0),
        position: iced::window::Position::Centered,
        min_size: Some(iced::Size::new(1000.0, 600.0)),
        icon,
        visible: !headless,
        ..Default::default()
    })
    .run()?;

    Ok(())
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
