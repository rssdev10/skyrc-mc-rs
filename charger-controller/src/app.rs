use iced::{Element, Subscription, Task, Theme};

use std::time::{Duration, Instant};
use std::hash::{Hash, Hasher};

use skyrc_mc5000::{
    SlotState as BtSlotState, ChargeConfig, OperationMode,
    Error as ChargerError, ConnectMode, DiscoveredCharger,
    SlotStatus, Session, SessionEvent, VoltageHistory,
};
use chrono::{DateTime, Utc};

/// How often the shared session polls all four slots while connected.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
use crate::slot::{Slot, SlotId, SlotState, TaskConfig, BatteryChemistry, TaskType};
use crate::data::{DataLogger, MeasurementPoint};
use crate::export::CsvExporter;
use crate::ui;
use crate::config_dialog::{ChargeMode};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AppMessage {
    /// UI timer only (elapsed-time display, cooldowns, the quit deadline). Never touches
    /// the charger.
    Tick,
    /// Quit (control `quit`, or the window's close button): exits once the commands in
    /// flight, above all stops, have reached the charger.
    QuitRequested,
    /// A decoded slot status from the shared session.
    StatusReceived(SlotStatus),
    DeviceSelected(String),
    BluetoothScanComplete(Result<Vec<DiscoveredCharger>, String>),
    /// Result of a quick targeted scan for a previously known device.
    /// `Ok(Some(device))` → found, ready to connect.
    /// `Ok(None)` → not found, fall back to full scan.
    QuickScanResult(Result<Option<DiscoveredCharger>, String>),
    ConnectDevice,
    /// Result of a background connect.
    Connected(Result<Session, String>),
    /// The session's notification stream ended.
    ConnectionLost,
    /// A command sequence for the slots in the mask finished.
    CommandFinished(CommandKind, u8, Result<(), String>),
    /// Result of an automatic reconnect attempt (attempt number, 1-based).
    ReconnectResult(u32, Result<Session, String>),
    DisconnectDevice,
    RefreshDevices,
    SlotMessage(SlotId, SlotMessage),
    StartTask(SlotId, TaskConfig),
    StopTask(SlotId),
    ExportData,
    FileSelected(Option<std::path::PathBuf>),
    ClearData,
    ConfigureSlot(SlotId),
    UpdateSlotChemistry(SlotId, BatteryChemistry),
    UpdateSlotTaskType(SlotId, TaskType),
    UpdateSlotCapacity(SlotId, u32),
    UpdateSlotChargeCurrent(SlotId, u16),
    CancelSlotConfig(SlotId),
    ApplySlotConfig(SlotId),
    SlotSelected(usize),
    // Configuration dialog messages
    ShowConfigDialog(SlotId, f32),  // slot_id, current_voltage
    ConfigChemistryChanged(BatteryChemistry),
    /// eneloop cell type picked in the Configure dialog.
    ConfigEneloopTypeChanged(skyrc_mc5000::EneloopType),
    ConfigModeChanged(ChargeMode),
    ConfigCapacityChanged(String),
    ConfigChargeCurrentChanged(String),
    ConfigDischargeCurrentChanged(String),
    ConfigTargetVoltageChanged(String),
    ConfigCutoffVoltageChanged(String),
    ConfigStorageVoltageChanged(String),
    ConfigDeltaPeakChanged(String),
    ConfigTrickleChargeChanged(String),
    ConfigCutoffTimerChanged(String),
    ConfigChargeCutoffCurrentChanged(String),
    ConfigDischargeCutoffCurrentChanged(String),
    ConfigChargeRestingChanged(String),
    ConfigDischargeRestingChanged(String),
    ConfigCycleCountChanged(String),
    ConfigDialogCancel,
    ConfigDialogConfirm,
    ConfigDialogDefault,  // Reset to defaults for current chemistry+mode
    // Profile management
    ConfigProfileNameChanged(String),
    ConfigSaveProfile,
    ConfigUpdateProfile,  // Update selected profile with current config
    ConfigDeleteProfile,
    ConfigUndoDelete,     // Undo last profile deletion
    ConfigSelectProfile(usize),
    ConfigExportProfiles,
    ConfigImportProfiles,
    ConfigProfilesImported(Option<std::path::PathBuf>),
    ConfigProfilesExported(Option<std::path::PathBuf>),
    SimpleAutoCharge,  // Simple auto: detect Li-Ion/NiMH, charge at 500mA
    SmartChargeAll,    // Smart charge: detect chemistry, measure resistance, optimize current
    StopAllSlots,      // Stop all active slots
    // Data statistics messages
    ToggleDetailedStats,  // Toggle detailed per-slot sample stream visibility
    ExportTimeAligned,    // Export time-aligned CSV (default)
    ExportAllSamples,     // Export all individual samples CSV
    FileSelectedTimeAligned(Option<std::path::PathBuf>),
    FileSelectedAllSamples(Option<std::path::PathBuf>),
    // Settings dialog messages
    SettingsOpen,
    SettingsClose,
    SettingsChangeTheme(crate::settings::AppTheme),
    SettingsChangeLanguage(String),
    SettingsToggleSaveDevice,
    SettingsOpenRepo,
    /// One line from the control channel (headless runs and agents).
    Control(String),
    /// The charger's voltage history (0x98) of a slot's program, requested by
    /// [`ChargerApp::history_task`].
    HistoryReceived(HistoryRequest, Result<VoltageHistory, String>),
    /// Graph: switch between Roll and Infinite.
    GraphToggleMode,
    /// Graph: the Roll window field changed.
    GraphWindowInput(String),
    /// Graph: apply the Roll window field.
    GraphWindowApply,
    /// Graph: hide what is recorded so far (kept for export).
    GraphClear,
}

/// What a voltage-history request was for: the slot (0-3), the program it belongs to and
/// its run time when asked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryRequest {
    pub slot: usize,
    pub program_start: DateTime<Utc>,
    pub elapsed_s: u32,
}

/// The app reads the charger's history only when its own data of the program has a hole
/// longer than this: it connected late, or the link was down for a while.
const HISTORY_HOLE_S: i64 = 15;
/// How long a history read may take (up to three pages, resent once if lost).
const HISTORY_TIMEOUT: Duration = Duration::from_secs(4);
/// A program-start estimate further than this from the previous one is a new program.
const PROGRAM_START_TOLERANCE_S: i64 = 5;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SlotMessage {
    UpdateState(SlotState),
    UpdateMeasurement(f32, f32), // voltage, current
}

pub struct ChargerApp {
    /// Chargers found by the last scan.
    devices: Vec<DiscoveredCharger>,
    /// The shared connection, while connected.
    session: Option<Session>,
    slots: [Slot; 4],
    data_logger: DataLogger,
    csv_exporter: CsvExporter,
    last_update: Instant,
    update_counter: u64,  // Force UI redraw
    selected_device: Option<String>,
    connection_status: ConnectionStatus,
    scanning: bool,  // Track if Bluetooth scan is in progress
    slot_configs: [Option<TaskConfig>; 4],
    configuring_slot: Option<SlotId>,
    selected_slot: Option<usize>,  // Track selected slot for graph display
    config_dialog_state: Option<crate::ui::components::config_dialog::ConfigDialogState>,
    pending_slot: Option<SlotId>,
    saved_slot_configs: [Option<crate::ui::components::config_dialog::ConfigDialogState>; 4],  // Persist config per slot
    slot_config_store: crate::slot_persist::SlotConfigStore,  // Disk-persisted per-slot configs
    /// The charger last connected, for automatic reconnects.
    last_device: Option<DiscoveredCharger>,
    /// Slots whose stop could not be delivered; sent first after (re)connecting.
    pending_stop: u8,
    /// Start and stop command sequences sent to the session whose result has not arrived.
    commands_in_flight: u32,
    /// Control-channel commands processed (published in the snapshot; see `control.rs`).
    control_seq: u64,
    /// Slots whose program is being re-sent (see [`Restart`]).
    restarts: [Option<Restart>; 4],
    /// Set by [`AppMessage::QuitRequested`] while commands are in flight: exit when they
    /// finish, or at this deadline.
    quit_deadline: Option<Instant>,
    /// Per-slot message for the user (why Auto/Smart skipped or limited a cell, a failed
    /// command). Kept until something new starts on the slot or the cell is removed.
    slot_notices: [Option<String>; 4],
    /// Last error code each slot's status reported (byte 20, 0 = none). A new code shows
    /// a notice; when the code clears, its notice goes too.
    slot_error_codes: [u8; 4],
    /// Smart charge probes waiting for the charger to report internal resistance.
    smart_probes: [Option<SmartProbe>; 4],  // Slots waiting for resistance measurement in auto-charge mode
    show_detailed_stats: bool,  // Toggle for detailed per-slot sample stream
    settings: crate::settings::Settings,
    show_settings: bool,
    profile_store: crate::profiles::ProfileStore,
    /// Timestamp when buttons should be re-enabled (for command-in-flight cooldown)
    buttons_locked_until: Option<Instant>,
    /// Per-slot lock: (timestamp, slot-state at button-press).
    /// Cleared when a notification brings a *different* state, or after 10s fallback.
    slot_command_pending: [Option<(Instant, SlotState)>; 4],
    /// Which slots the charger has reported since connecting.
    status_seen: [bool; 4],
    /// Program each slot is configured with, as the charger reports it ("NiMH Cycle").
    reported_modes: [Option<String>; 4],
    /// Control channel and the snapshots it waits on, when enabled.
    control: Option<Control>,
    /// When each slot's current or last program started (now minus its run time, while
    /// it runs). Kept after the program ends: the charger keeps its history.
    program_starts: [Option<DateTime<Utc>>; 4],
    /// The program (and connection) whose history need each slot has checked, so it is
    /// checked once per connection and program.
    history_checked: [Option<(u64, DateTime<Utc>)>; 4],
    /// Samples the last history backfill added per slot (for the control snapshot).
    history_samples: [usize; 4],
    /// Graph Roll-window field as typed.
    graph_window_input: String,
    /// Set by the graph's Clear: older samples are not drawn.
    graph_epoch: Option<DateTime<Utc>>,
}

struct Control {
    options: crate::control::ControlOptions,
    snapshots: tokio::sync::watch::Sender<crate::control::Snapshot>,
}

/// A Smart charge probe running on one slot.
#[derive(Debug, Clone, Copy)]
struct SmartProbe {
    chemistry: BatteryChemistry,
    started: Instant,
}

/// How long Smart waits for a resistance reading before keeping the probe current.
const SMART_PROBE_TIMEOUT: Duration =
    Duration::from_secs(skyrc_mc5000::charging::SMART_PROBE_TIMEOUT_S);
/// Charger-reported run time before the resistance reading is trusted.
const SMART_PROBE_SETTLE_S: u32 = skyrc_mc5000::charging::SMART_PROBE_SETTLE_S;
/// A program re-sent to a running slot (Smart switching from its probe to the chosen
/// current). The charger stops the old program and starts the new one, so it reports the slot
/// idle in between; that idle is not shown.
#[derive(Debug, Clone, Copy)]
struct Restart {
    since: Instant,
    /// The restart command has reached the charger.
    sent: bool,
}

/// How long a slot counts as waiting for the charger to confirm a start or stop (the same
/// fallback after which the view re-enables its buttons).
const COMMAND_CONFIRM_WAIT: Duration = Duration::from_secs(10);

/// Whether `samples` (one slot's, in time order) leave part of `from..to` uncovered for
/// longer than [`HISTORY_HOLE_S`].
fn has_hole(samples: &[MeasurementPoint], from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
    let hole = chrono::Duration::seconds(HISTORY_HOLE_S);
    let start = samples.partition_point(|m| m.timestamp < from);
    let mut covered_until = from;
    for m in samples[start..].iter().take_while(|m| m.timestamp <= to) {
        if m.timestamp - covered_until > hole {
            return true;
        }
        covered_until = m.timestamp;
    }
    to - covered_until > hole
}

/// What a background command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Start,
    Stop,
}

/// Automatic reconnect attempts after the link is lost.
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    /// The link was lost; automatic reconnect attempt N is running.
    Reconnecting(u32),
    Error(String),
}

/// Subscription identity for one connection: a new connection restarts the stream.
struct SessionSubscription(Session);

impl Hash for SessionSubscription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "mc5000_session".hash(state);
        self.0.generation().hash(state);
    }
}

/// Turns the session's events into app messages. Only decoded slot statuses and the end
/// of the connection are forwarded; polling itself runs inside the session.
fn session_stream(
    subscription: &SessionSubscription,
) -> impl futures::stream::Stream<Item = AppMessage> {
    use tokio::sync::broadcast::error::RecvError;

    futures::stream::unfold(Some(subscription.0.subscribe()), |events| async move {
        let mut events = events?;
        loop {
            match events.recv().await {
                Ok(SessionEvent::Frame(frame)) => {
                    if let Some(status) = frame.status() {
                        return Some((AppMessage::StatusReceived(status), Some(events)));
                    }
                }
                Ok(SessionEvent::Disconnected) | Err(RecvError::Closed) => {
                    return Some((AppMessage::ConnectionLost, None));
                }
                Err(RecvError::Lagged(_)) => {}
            }
        }
    })
}

/// Label shown in the device list and stored as the last device.
/// Format: `MC5000 BT: <name> (ID:<peripheral id>)`.
fn device_label(device: &DiscoveredCharger) -> String {
    format!("MC5000 BT: {} (ID:{})", device.name, device.id)
}

/// The peripheral ID inside a [`device_label`].
fn label_peripheral_id(label: &str) -> Option<String> {
    let start = label.find("ID:")? + 3;
    let rest = &label[start..];
    Some(rest.find(')').map_or(rest, |end| &rest[..end]).to_string())
}

fn full_scan_task() -> Task<AppMessage> {
    Task::perform(
        async { Session::scan(5).await.map_err(|e| e.to_string()) },
        AppMessage::BluetoothScanComplete,
    )
}

impl ChargerApp {
    #[cfg(test)]
    pub fn new() -> (Self, Task<AppMessage>) {
        Self::new_with(None)
    }

    /// Creates the app; `control` enables the text control channel and JSON feedback.
    pub fn new_with(control: Option<crate::control::ControlOptions>) -> (Self, Task<AppMessage>) {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        if verbose {
            println!("[APP VERBOSE] Initializing ChargerApp...");
        }
        
        let settings = crate::settings::load();
        crate::i18n::set_language(&settings.language);
        
        if verbose {
            println!("[APP VERBOSE] Creating device manager and initializing slots...");
        }
        
        let profile_store = crate::profiles::ProfileStore::load();
        let slot_config_store = crate::slot_persist::SlotConfigStore::load();

        // Reconstruct saved dialog states from persisted disk configs
        let saved_slot_configs = {
            use crate::ui::components::config_dialog::ConfigDialogState;
            let mut arr: [Option<ConfigDialogState>; 4] = [None, None, None, None];
            for (i, slot_arr) in arr.iter_mut().enumerate() {
                if let Some(persisted) = slot_config_store.get(i) {
                    let mut state = ConfigDialogState::from_persisted(persisted);
                    state.resolve_profile(&profile_store);
                    *slot_arr = Some(state);
                }
            }
            arr
        };

        let graph_window_input = settings.graph_window_s.to_string();
        let app = ChargerApp {
            devices: Vec::new(),
            session: None,
            slots: [
                Slot::new(SlotId(0)),
                Slot::new(SlotId(1)),
                Slot::new(SlotId(2)),
                Slot::new(SlotId(3)),
            ],
            data_logger: DataLogger::new(),
            csv_exporter: CsvExporter::default(),
            last_update: Instant::now(),
            update_counter: 0,
            selected_device: None,
            connection_status: ConnectionStatus::Disconnected,
            scanning: true,  // Start with scanning active
            slot_configs: [None, None, None, None],
            configuring_slot: None,
            selected_slot: Some(0),  // Default to first slot selected
            config_dialog_state: None,
            pending_slot: None,
            saved_slot_configs,
            smart_probes: [None, None, None, None],
            last_device: None,
            pending_stop: 0,
            commands_in_flight: 0,
            control_seq: 0,
            restarts: [None; 4],
            quit_deadline: None,
            slot_notices: [None, None, None, None],
            slot_error_codes: [0; 4],
            show_detailed_stats: false,  // Default: hide detailed per-slot stream
            settings,
            show_settings: false,
            profile_store,
            slot_config_store,
            buttons_locked_until: None,
            slot_command_pending: [None, None, None, None],
            status_seen: [false; 4],
            reported_modes: [None, None, None, None],
            control: control.map(|options| Control {
                options,
                snapshots: tokio::sync::watch::channel(crate::control::Snapshot::default()).0,
            }),
            program_starts: [None; 4],
            history_checked: [None; 4],
            history_samples: [0; 4],
            graph_window_input,
            graph_epoch: None,
        };

        if verbose {
            println!("[APP VERBOSE] Starting initial Bluetooth device scan (async)...");
            println!("[APP VERBOSE] ChargerApp initialization complete\n");
        }

        // If we have a saved device and "save_last_device" is on, do a quick targeted
        // scan instead of a full 5-second blind scan.
        if let Some(control) = &app.control {
            control.snapshots.send_replace(app.snapshot());
        }

        let saved_peripheral = app
            .settings
            .last_device_id
            .as_deref()
            .filter(|_| app.settings.save_last_device)
            .and_then(label_peripheral_id);
        let startup_task = match saved_peripheral {
            Some(pid) => {
                if verbose {
                    println!("[APP VERBOSE] Quick scan for saved peripheral: {}", pid);
                }
                Task::perform(
                    async move {
                        skyrc_mc5000::Protocol::scan_for_device(&pid, 5)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    AppMessage::QuickScanResult,
                )
            }
            None => full_scan_task(),
        };

        (app, startup_task)
    }

    /// Mask of the slots the GUI tracks as active, excluding the slots in `except`.
    fn active_mask_except(&self, except: u8) -> u8 {
        self.slots
            .iter()
            .enumerate()
            .filter(|(i, s)| s.is_active() && except & (1 << i) == 0)
            .fold(0, |mask, (i, _)| mask | 1 << i)
    }

    /// Auto (`smart == false`) and Smart (`smart == true`) buttons: detect each idle cell's
    /// chemistry and start charging it. Auto charges at a fixed 500 mA; Smart
    /// starts a low-current probe and picks the current once the resistance is known
    /// (see [`Self::smart_step`]). Cells that cannot be identified are skipped.
    fn auto_charge(&mut self, smart: bool) -> Task<AppMessage> {
        use crate::auto_charge::{charge_plan, detect_chemistry, probe_current_ma};
        let label = if smart { "smart" } else { "auto" };
        self.buttons_locked_until = Some(Instant::now() + Duration::from_secs(5));
        if self.session.is_none() {
            return Task::none();
        }

        let mut configs = Vec::new();
        for idx in 0..4 {
            let slot = &self.slots[idx];
            if !slot.is_idle() || slot.current_voltage < crate::auto_charge::EMPTY_V {
                continue;
            }
            let voltage = slot.current_voltage;
            let chemistry = match detect_chemistry(voltage) {
                Ok(chemistry) => chemistry,
                Err(reason) => {
                    log::info!("{label}: slot {} skipped: {reason}", idx + 1);
                    if let crate::auto_charge::SkipReason::Ambiguous { voltage } = reason {
                        self.slot_notices[idx] = Some(
                            crate::i18n::t!("notice.ambiguous_voltage", voltage = format!("{voltage:.2}"))
                                .to_string(),
                        );
                    }
                    self.emit("event", serde_json::json!({
                        "event": "auto_skipped", "slot": idx + 1, "reason": reason.to_string(),
                    }));
                    continue;
                }
            };
            let current = Some(if smart {
                probe_current_ma(chemistry)
            } else {
                crate::auto_charge::AUTO_CURRENT_MA
            });
            let (config, task) = charge_plan(idx, chemistry, current);
            log::info!(
                "{label}: slot {} {:.3} V → {:?}, {} mA{}",
                idx + 1,
                voltage,
                chemistry,
                config.charge_current_ma,
                if smart { " (probe)" } else { "" }
            );
            self.emit("event", serde_json::json!({
                "event": if smart { "smart_probe" } else { "auto_start" },
                "slot": idx + 1,
                "voltage": voltage,
                "chemistry": format!("{chemistry:?}"),
                "current_ma": config.charge_current_ma,
            }));
            // The state changes when the charger reports the slot running.
            self.slot_command_pending[idx] = Some((Instant::now(), self.slots[idx].state.clone()));
            let slot = &mut self.slots[idx];
            slot.current_task = Some(task.clone());
            slot.start_time = Some(Instant::now());
            self.slot_configs[idx] = Some(task);
            self.slot_notices[idx] = None;
            self.smart_probes[idx] = smart.then(|| SmartProbe { chemistry, started: Instant::now() });
            configs.push(config);
        }

        if configs.is_empty() {
            return Task::none();
        }
        let start_mask = configs.iter().fold(0u8, |m, c| m | c.channel_bitmask);
        let keep_running = self.active_mask_except(start_mask);
        self.run_command(start_mask, move |session| async move {
            session.start_slots(&configs, keep_running).await
        })
    }

    /// One Smart step for a slot with a running probe: when the charger has measured the
    /// internal resistance, restart the charge at the current chosen for it, or stop the
    /// slot if the cell should not be charged.
    fn smart_step(&mut self, idx: usize, status: &SlotStatus) -> Task<AppMessage> {
        use crate::auto_charge::{charge_plan, smart_current_ma};
        let Some(probe) = self.smart_probes[idx] else {
            return Task::none();
        };
        let measured = status.is_running()
            && status.resistance_milliohm > 0
            && status.elapsed_seconds >= SMART_PROBE_SETTLE_S;
        if !measured {
            if probe.started.elapsed() > SMART_PROBE_TIMEOUT {
                // No reading (for example a nearly full cell): keep the probe current.
                self.smart_probes[idx] = None;
                self.slot_notices[idx] = Some(crate::i18n::t!("notice.no_resistance").to_string());
                self.emit("event", serde_json::json!({
                    "event": "smart_no_reading", "slot": idx + 1,
                    "running": status.is_running(),
                }));
            }
            return Task::none();
        }

        self.smart_probes[idx] = None;
        let slot_bit = 1u8 << idx;
        match smart_current_ma(probe.chemistry, status.resistance_milliohm) {
            Ok(crate::auto_charge::SmartCurrent { current_ma: current, worn }) => {
                let (config, task) = charge_plan(idx, probe.chemistry, Some(current));
                if worn {
                    self.slot_notices[idx] = Some(
                        crate::i18n::t!(
                            "notice.worn_cell",
                            resistance = status.resistance_milliohm,
                            current = current
                        )
                        .to_string(),
                    );
                }
                log::info!(
                    "smart: slot {} R={} mΩ → {:?} at {} mA",
                    idx + 1, status.resistance_milliohm, probe.chemistry, current
                );
                self.emit("event", serde_json::json!({
                    "event": "smart_charge", "slot": idx + 1,
                    "chemistry": format!("{:?}", probe.chemistry),
                    "resistance_mohm": status.resistance_milliohm,
                    "current_ma": current,
                    "worn": worn,
                }));
                self.slots[idx].current_task = Some(task.clone());
                self.slot_configs[idx] = Some(task);
                let keep_running = self.active_mask_except(slot_bit);
                self.restarts[idx] = Some(Restart { since: Instant::now(), sent: false });
                self.run_command(slot_bit, move |session| async move {
                    session.start_slots(&[config], keep_running).await
                })
            }
            Err(reason) => {
                log::warn!("smart: slot {} not charged: {reason}", idx + 1);
                self.emit("event", serde_json::json!({
                    "event": "smart_rejected", "slot": idx + 1,
                    "resistance_mohm": status.resistance_milliohm, "reason": reason.to_string(),
                }));
                self.slot_notices[idx] = Some(match reason {
                    crate::auto_charge::SmartRefusal::NotMeasured => {
                        crate::i18n::t!("notice.no_resistance").to_string()
                    }
                    crate::auto_charge::SmartRefusal::TooResistive { resistance_mohm } => {
                        crate::i18n::t!("notice.resistance_too_high", resistance = resistance_mohm)
                            .to_string()
                    }
                });
                self.slots[idx].stop();
                self.slot_configs[idx] = None;
                self.stop_command(slot_bit)
            }
        }
    }

    /// Stops the slots in `mask`. Without a working link the stop is queued and sent first
    /// after the next (re)connect; a stop that fails is queued the same way.
    fn stop_command(&mut self, mask: u8) -> Task<AppMessage> {
        match self.session.as_ref().filter(|s| !s.is_lost()) {
            Some(session) => {
                let session = session.clone();
                self.commands_in_flight += 1;
                Task::perform(async move { session.stop_slots(mask).await }, move |result| {
                    AppMessage::CommandFinished(CommandKind::Stop, mask, result.map_err(|e| e.to_string()))
                })
            }
            None => {
                self.queue_stop(mask);
                Task::none()
            }
        }
    }

    fn queue_stop(&mut self, mask: u8) {
        self.pending_stop |= mask;
        log::warn!("Stop for slot mask 0x{mask:02X} queued until the charger is reconnected");
        self.emit("event", serde_json::json!({ "event": "stop_queued", "slots": self.pending_stop }));
    }

    /// Sends queued stops on a fresh connection.
    fn send_pending_stop(&mut self) -> Task<AppMessage> {
        match std::mem::take(&mut self.pending_stop) {
            0 => Task::none(),
            mask => self.stop_command(mask),
        }
    }

    /// One automatic reconnect attempt to the last charger, after a growing delay.
    fn reconnect_task(&self, attempt: u32) -> Task<AppMessage> {
        let Some(device) = self.last_device.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                // First attempt at once; then 2 s, 4 s, ...
                tokio::time::sleep(Duration::from_secs(2 * u64::from(attempt - 1))).await;
                // Session::connect finds the charger again if the old handle is stale.
                let session = Session::connect(&device, ConnectMode::Control)
                    .await
                    .map_err(|e| e.to_string())?;
                session.start_polling(POLL_INTERVAL);
                Ok(session)
            },
            move |result| AppMessage::ReconnectResult(attempt, result),
        )
    }

    /// Runs a command sequence on the shared session in the background. The UI never waits
    /// for the charger; the outcome arrives as [`AppMessage::CommandFinished`].
    fn run_command<F, Fut>(&mut self, slots: u8, command: F) -> Task<AppMessage>
    where
        F: FnOnce(Session) -> Fut,
        Fut: std::future::Future<Output = Result<(), ChargerError>> + Send + 'static,
    {
        match &self.session {
            Some(session) => {
                let future = command(session.clone());
                self.commands_in_flight += 1;
                Task::perform(future, move |result| {
                    AppMessage::CommandFinished(CommandKind::Start, slots, result.map_err(|e| e.to_string()))
                })
            }
            None => Task::none(),
        }
    }

    /// How long a quit waits for commands in flight before exiting anyway.
    const QUIT_WAIT: Duration = Duration::from_secs(20);

    /// Exits now when nothing is in flight; otherwise once the last command finishes (see
    /// `CommandFinished`) or at the deadline (see `Tick`).
    fn request_quit(&mut self) -> Task<AppMessage> {
        if self.commands_in_flight == 0 {
            return self.quit_now();
        }
        if self.quit_deadline.is_none() {
            self.quit_deadline = Some(Instant::now() + Self::QUIT_WAIT);
            log::info!("Quit: waiting for {} command(s) to reach the charger", self.commands_in_flight);
            self.emit("event", serde_json::json!({ "event": "quit_waiting", "commands": self.commands_in_flight }));
        }
        Task::none()
    }

    /// Disconnects and exits. Warns loudly when a stop never reached the charger.
    fn quit_now(&mut self) -> Task<AppMessage> {
        self.quit_deadline = None;
        if self.pending_stop != 0 || self.commands_in_flight != 0 {
            log::error!(
                "Exiting with stop mask 0x{:02X} undelivered and {} command(s) unconfirmed: \
                 check the charger, programs may still be running",
                self.pending_stop,
                self.commands_in_flight
            );
            self.emit("event", serde_json::json!({
                "event": "stop_not_delivered",
                "slots": self.pending_stop,
                "commands_in_flight": self.commands_in_flight,
            }));
        }
        match self.session.take() {
            Some(session) => Task::future(async move {
                let _ = session.disconnect().await;
            })
            .discard()
            .chain(iced::exit()),
            None => iced::exit(),
        }
    }

    pub fn title(&self) -> String {
        format!("{} v{}", crate::i18n::t!("app.title"), env!("CARGO_PKG_VERSION"))
    }

    pub fn update(&mut self, message: AppMessage) -> Task<AppMessage> {
        let task = self.handle(message);
        if let Some(control) = &self.control {
            let snapshot = self.snapshot();
            control.snapshots.send_if_modified(|current| {
                let changed = *current != snapshot;
                *current = snapshot;
                changed
            });
        }
        task
    }

    /// What the control channel reports and waits on.
    /// Notes when the slot's program started and, once per connection and program, asks
    /// the charger for its voltage history if the app's own data of the program has a hole:
    /// the app connected after the start, or the link was down. Read only.
    fn history_task(&mut self, slot: usize, status: &SlotStatus) -> Task<AppMessage> {
        let now = Utc::now();
        if status.is_running() {
            let estimate = now - chrono::Duration::seconds(i64::from(status.elapsed_seconds));
            let same_program = self.program_starts[slot].is_some_and(|start| {
                (start - estimate).num_seconds().abs() <= PROGRAM_START_TOLERANCE_S
            });
            if !same_program {
                self.program_starts[slot] = Some(estimate);
            }
        }
        let (Some(session), Some(program_start)) = (&self.session, self.program_starts[slot]) else {
            return Task::none();
        };
        // The charger keeps the history after the program ends, and a finished program's
        // start is still known if the app saw it run. It runs again from 0 after a restart.
        if status.elapsed_seconds == 0 || status.voltage_mv == 0 {
            return Task::none();
        }
        let key = (session.generation(), program_start);
        if self.history_checked[slot] == Some(key) {
            return Task::none();
        }
        self.history_checked[slot] = Some(key);
        if !has_hole(
            self.data_logger.get_measurements_for_slot(SlotId(slot)),
            program_start,
            now.min(program_start + chrono::Duration::seconds(i64::from(status.elapsed_seconds))),
        ) {
            return Task::none();
        }
        let request = HistoryRequest { slot, program_start, elapsed_s: status.elapsed_seconds };
        let session = session.clone();
        let slot_id = slot as u8 + 1;
        Task::perform(
            async move { session.read_history(slot_id, HISTORY_TIMEOUT).await.map_err(|e| e.to_string()) },
            move |result| AppMessage::HistoryReceived(request, result),
        )
    }

    /// Adds a history the charger sent to the slot's data, if it fits the program it was
    /// asked for.
    fn backfill_history(&mut self, request: HistoryRequest, result: Result<VoltageHistory, String>) {
        let slot = request.slot;
        let history = match result {
            Ok(history) => history,
            Err(e) => {
                log::info!("Slot {}: no voltage history from the charger: {e}", slot + 1);
                return;
            }
        };
        let span = history.samples_mv.len() as i64 * i64::from(history.interval_s);
        let slack = 2 * i64::from(history.interval_s) + 30;
        if usize::from(history.slot_id) != slot + 1 || (span - i64::from(request.elapsed_s)).abs() > slack {
            log::warn!(
                "Slot {}: history of {span} s does not fit the program's {} s; not used",
                slot + 1,
                request.elapsed_s
            );
            return;
        }
        let added = self.data_logger.backfill_history(
            SlotId(slot),
            request.program_start,
            history.interval_s,
            &history.samples_mv,
        );
        self.history_samples[slot] = added;
        log::info!(
            "Slot {}: {added} of {} history samples ({} s apart) filled in",
            slot + 1,
            history.samples_mv.len(),
            history.interval_s
        );
        self.emit(
            "event",
            serde_json::json!({
                "event": "history",
                "slot": slot + 1,
                "samples": history.samples_mv.len(),
                "interval_s": history.interval_s,
                "added": added,
            }),
        );
    }

    fn snapshot(&self) -> crate::control::Snapshot {
        use crate::control::{Snapshot, SlotSnapshot};
        let connection = match &self.connection_status {
            ConnectionStatus::Disconnected => "disconnected",
            ConnectionStatus::Connecting => "connecting",
            ConnectionStatus::Connected => "connected",
            ConnectionStatus::Reconnecting(_) => "reconnecting",
            ConnectionStatus::Error(_) => "error",
        };
        Snapshot {
            scanning: self.scanning,
            connection: connection.to_string(),
            error: match &self.connection_status {
                ConnectionStatus::Error(e) => Some(e.clone()),
                _ => None,
            },
            devices: self.devices.iter().map(device_label).collect(),
            selected_device: self.selected_device.clone(),
            connected_to: self.session.as_ref().map(|s| s.name().to_string()),
            dialog_slot: self
                .config_dialog_state
                .as_ref()
                .and(self.pending_slot)
                .map(|s| s.0 + 1),
            commands_in_flight: self.commands_in_flight,
            control_seq: self.control_seq,
            slots: self
                .slots
                .iter()
                .enumerate()
                .map(|(i, slot)| SlotSnapshot {
                    slot: i + 1,
                    state: match &slot.state {
                        SlotState::Idle => "idle",
                        SlotState::Charging => "charging",
                        SlotState::Discharging => "discharging",
                        SlotState::Completed => "completed",
                        SlotState::Error(_) => "error",
                        SlotState::Paused => "paused",
                    }
                    .to_string(),
                    seen: self.status_seen[i],
                    mode: self.reported_modes[i].clone(),
                    voltage: slot.current_voltage,
                    current_ma: slot.current_current,
                    capacity_mah: slot.capacity_mah,
                    resistance_mohm: slot.resistance_milliohm,
                    elapsed_s: slot.elapsed_seconds,
                    awaiting_charger: self.restarts[i].is_some()
                        || self.slot_command_pending[i]
                            .as_ref()
                            .is_some_and(|(at, _)| at.elapsed() < COMMAND_CONFIRM_WAIT),
                    temperature_c: slot.temperature_c,
                    charger_error: Some(self.slot_error_codes[i]).filter(|&code| code != 0),
                    task: slot.current_task.as_ref().map(|t| t.task_type.to_string()),
                    error: match &slot.state {
                        SlotState::Error(e) => Some(e.clone()),
                        _ => None,
                    },
                    notice: self.slot_notices[i].clone(),
                    history_samples: self.history_samples[i],
                })
                .collect(),
        }
    }

    /// Prints a control-channel event, when the channel is enabled.
    fn emit(&self, kind: &str, fields: serde_json::Value) {
        if self.control.is_some() {
            crate::control::emit(kind, fields);
        }
    }

    /// Executes one control-channel command by sending the same messages the UI sends.
    fn handle_control(&mut self, line: &str) -> Task<AppMessage> {
        use crate::control::{dialog_field, emit_error, parse_slot};
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or_default();
        let args: Vec<&str> = words.collect();
        let fail = |message: String| {
            emit_error(&format!("{line}: {message}"));
            Task::none()
        };

        let task = match command {
            "scan" => self.update(AppMessage::RefreshDevices),
            "devices" => {
                let devices: Vec<String> = self.devices.iter().map(device_label).collect();
                self.emit("devices", serde_json::json!({ "devices": devices }));
                Task::none()
            }
            "select" => {
                let labels: Vec<String> = self.devices.iter().map(device_label).collect();
                let wanted = args.join(" ");
                let found = wanted
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| labels.get(n.wrapping_sub(1)).cloned())
                    .or_else(|| labels.iter().find(|l| !wanted.is_empty() && l.contains(&wanted)).cloned());
                match found {
                    Some(label) => self.update(AppMessage::DeviceSelected(label)),
                    None => return fail(format!("no charger matches '{wanted}'")),
                }
            }
            "connect" => self.update(AppMessage::ConnectDevice),
            "disconnect" => self.update(AppMessage::DisconnectDevice),
            "configure" | "start" => {
                let slot = match parse_slot(args.first().copied()) {
                    Ok(slot) => slot,
                    Err(e) => return fail(e),
                };
                let voltage = self.slots[slot].current_voltage;
                let mut tasks = vec![self.update(AppMessage::ShowConfigDialog(SlotId(slot), voltage))];
                if command == "start" {
                    // Chemistry and mode reset the other fields to their defaults (as in the
                    // dialog), so they are applied first, whatever order they were given in.
                    let mut settings = args[1..].to_vec();
                    settings.sort_by_key(|s| match s.split_once('=').map(|(key, _)| key) {
                        Some("chemistry") => 0,
                        Some("mode") => 1,
                        _ => 2,
                    });
                    for setting in &settings {
                        let Some((key, value)) = setting.split_once('=') else {
                            let _ = self.update(AppMessage::ConfigDialogCancel);
                            return fail(format!("expected key=value, got '{setting}'"));
                        };
                        match dialog_field(key, value) {
                            Ok(message) => tasks.push(self.update(message)),
                            Err(e) => {
                                let _ = self.update(AppMessage::ConfigDialogCancel);
                                return fail(e);
                            }
                        }
                    }
                    // Only modes the dialog offers for this chemistry, as for a user.
                    if let Some(state) = &self.config_dialog_state {
                        let offered = crate::config_dialog::ChargeMode::available_for_chemistry(state.chemistry);
                        if !offered.contains(&state.mode) {
                            let message = format!("{:?} does not offer {:?}", state.chemistry, state.mode);
                            let _ = self.update(AppMessage::ConfigDialogCancel);
                            return fail(message);
                        }
                    }
                    tasks.push(self.update(AppMessage::ConfigDialogConfirm));
                    // A refused Confirm leaves the dialog open with the reason.
                    if let Some(error) = self.config_dialog_state.as_ref().and_then(|d| d.error.clone()) {
                        let _ = self.update(AppMessage::ConfigDialogCancel);
                        return fail(error);
                    }
                }
                Task::batch(tasks)
            }
            "set" => {
                if self.config_dialog_state.is_none() {
                    return fail("no dialog is open (use configure <slot>)".into());
                }
                match (args.first(), args.get(1)) {
                    (Some(key), Some(value)) => match dialog_field(key, value) {
                        Ok(message) => self.update(message),
                        Err(e) => return fail(e),
                    },
                    _ => return fail("usage: set <key> <value>".into()),
                }
            }
            "confirm" => self.update(AppMessage::ConfigDialogConfirm),
            "cancel" => self.update(AppMessage::ConfigDialogCancel),
            "stop" => match args.first() {
                None => self.update(AppMessage::StopAllSlots),
                Some(slot) => match parse_slot(Some(slot)) {
                    Ok(slot) => self.update(AppMessage::StopTask(SlotId(slot))),
                    Err(e) => return fail(e),
                },
            },
            "auto" => self.update(AppMessage::SimpleAutoCharge),
            "smart" => self.update(AppMessage::SmartChargeAll),
            "select-slot" => match parse_slot(args.first().copied()) {
                Ok(slot) => self.update(AppMessage::SlotSelected(slot)),
                Err(e) => return fail(e),
            },
            "state" => {
                crate::control::emit_state(&self.snapshot());
                Task::none()
            }
            "help" => {
                self.emit("help", serde_json::json!({ "text": crate::control::HELP }));
                Task::none()
            }
            "quit" | "exit" => {
                self.emit("ack", serde_json::json!({ "command": line }));
                return self.update(AppMessage::QuitRequested);
            }
            _ => return fail("unknown command (try 'help')".into()),
        };
        self.emit("ack", serde_json::json!({ "command": line }));
        task
    }

    /// Shows the charger's error code for a slot as its notice when the code changes, and
    /// removes that notice once the charger clears the code.
    fn note_charger_error(&mut self, idx: usize, code: u8) {
        let previous = self.slot_error_codes[idx];
        if code == previous {
            return;
        }
        self.slot_error_codes[idx] = code;
        if code != 0 {
            self.slot_notices[idx] = Some(charger_error_notice(code));
        } else if self.slot_notices[idx].as_deref() == Some(charger_error_notice(previous).as_str()) {
            self.slot_notices[idx] = None;
        }
    }

    fn handle(&mut self, message: AppMessage) -> Task<AppMessage> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        match message {
            AppMessage::StatusReceived(status) => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                // Set when auto-charge adjusts the current; runs in the background.
                let mut command_task = Task::none();

                // The session only delivers checksum-valid frames for slots 1-4.
                let slot_idx = usize::from(status.slot_id.saturating_sub(1));
                if (1..=4).contains(&status.slot_id) {
                    self.status_seen[slot_idx] = true;
                    if status.voltage_mv == 0 {
                        // Cell removed: its notice no longer applies.
                        self.slot_notices[slot_idx] = None;
                    }
                    self.note_charger_error(slot_idx, status.error.unwrap_or(0));
                    self.reported_modes[slot_idx] = match (status.chemistry, status.mode) {
                        (Some(chemistry), Some(mode)) => Some(format!("{} {}", chemistry.name(), mode.name())),
                        _ => None,
                    };
                    if verbose {
                        use std::io::Write;
                        println!("[GUI VERBOSE] Status for slot {}", slot_idx + 1);
                        let _ = std::io::stdout().flush();
                    }

                    // (command-pending flag is cleared below, after we know the new state)
                    {
                        let voltage = status.voltage();
                        // current_current is stored and displayed in mA throughout the codebase.
                        // status.current() returns amps (divides by 1000) which is wrong here.
                        let current = status.current_ma as f32;
                        
                        if verbose {
                            use std::io::Write;
                            println!("[GUI VERBOSE]   State: {:?}, {:.3}V, {}mA, {}mAh", 
                                status.state, voltage, status.current_ma, status.capacity_mah);
                            let _ = std::io::stdout().flush();
                        }
                        
                        // Update slot data
                        if let Some(slot) = self.slots.get_mut(slot_idx) {
                            slot.update_full_status(
                                voltage,
                                current,
                                status.capacity_mah,
                                status.resistance_milliohm,
                                status.elapsed_seconds
                            );
                            slot.temperature_c = status.temperature_c;
                            // The parser now returns Charging for any active slot (current>0).
                            // Restore Discharging when the task type calls for it.
                            let mut new_state = map_bt_state(&status.state);
                            if new_state == SlotState::Charging {
                                if let Some(ref task) = slot.current_task {
                                    if task.task_type == TaskType::Discharge {
                                        new_state = SlotState::Discharging;
                                    }
                                }
                            }
                            // The charger's phase byte is authoritative: a program it ends at once
                            // (e.g. battery type error) shows as idle at once. Only the idle in
                            // the middle of a restart is not shown.
                            let running = matches!(
                                new_state,
                                SlotState::Charging | SlotState::Discharging | SlotState::Paused
                            );
                            match self.restarts[slot_idx] {
                                Some(restart) if restart.since.elapsed() >= COMMAND_CONFIRM_WAIT => {
                                    self.restarts[slot_idx] = None;
                                    slot.set_state(new_state);
                                }
                                Some(restart) if restart.sent && running => {
                                    self.restarts[slot_idx] = None;
                                    slot.set_state(new_state);
                                }
                                Some(_) if !running => {}
                                _ => slot.set_state(new_state),
                            }
                        }

                        // Unlock button for this slot when its state changed from what was
                        // recorded at button-press time (10s fallback is handled in the view).
                        if let Some((_, ref prev_state)) = self.slot_command_pending[slot_idx].clone() {
                            if self.slots[slot_idx].state != *prev_state {
                                self.slot_command_pending[slot_idx] = None;
                            }
                        }

                        // Smart charge: once the probe reports the cell's resistance, charge
                        // at the current chosen for it.
                        command_task = self.smart_step(slot_idx, &status);

                        // Force UI update
                        self.update_counter = self.update_counter.wrapping_add(1);
                        self.last_update = Instant::now();
                        
                        // Log measurement with full data
                        let state_str = format!("{:?}", status.state);
                        let task_mode = if let Some(slot) = self.slots.get(slot_idx) {
                            if let Some(ref task) = slot.current_task {
                                format!("{:?}", task.task_type)
                            } else {
                                "None".to_string()
                            }
                        } else {
                            "Unknown".to_string()
                        };
                        let measurement = MeasurementPoint::new_full(
                            SlotId(slot_idx),
                            voltage,
                            current,
                            state_str,
                            task_mode,
                            status.resistance_milliohm,
                            status.elapsed_seconds
                        );
                        self.data_logger.add_measurement(measurement);
                        let history = self.history_task(slot_idx, &status);
                        command_task = Task::batch([command_task, history]);
                    }
                }
                command_task
            }
            
            AppMessage::Tick => {
                // Status polling runs inside the shared session; the timer only refreshes
                // time-based UI (elapsed times, button cooldowns) and ends an overdue quit.
                if self.quit_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return self.quit_now();
                }
                Task::none()
            }

            AppMessage::DeviceSelected(device_name) => {
                self.selected_device = Some(device_name);
                Task::none()
            }

            AppMessage::QuickScanResult(result) => {
                match result {
                    Ok(Some(device)) => {
                        if verbose {
                            println!("[APP VERBOSE] Quick scan found device: {}", device.name);
                        }
                        self.scanning = false;
                        self.selected_device = Some(device_label(&device));
                        self.devices = vec![device];
                        self.update(AppMessage::ConnectDevice)
                    }
                    Ok(None) => {
                        if verbose {
                            println!("[APP VERBOSE] Quick scan: device not found, falling back to full scan");
                        }
                        full_scan_task()
                    }
                    Err(e) => {
                        if verbose {
                            println!("[APP VERBOSE] Quick scan error: {}, falling back to full scan", e);
                        }
                        full_scan_task()
                    }
                }
            }

            AppMessage::BluetoothScanComplete(result) => {
                self.scanning = false;
                match result {
                    Ok(devices) => {
                        if verbose {
                            println!("[APP VERBOSE] ✓ Bluetooth scan found {} charger(s)", devices.len());
                        }
                        self.devices = devices;
                        let labels: Vec<String> = self.devices.iter().map(device_label).collect();
                        self.emit("event", serde_json::json!({ "event": "scanned", "devices": labels }));
                        // Keep the current selection if it is still there, else pick the first.
                        if !self.selected_device.as_ref().is_some_and(|s| labels.contains(s)) {
                            self.selected_device = labels.into_iter().next();
                        }
                    }
                    Err(e) => {
                        if verbose {
                            println!("[APP VERBOSE] ✗ Bluetooth scan failed: {}", e);
                        }
                        log::error!("Bluetooth scan failed: {}", e);
                        self.emit("event", serde_json::json!({ "event": "scan_failed", "error": e }));
                    }
                }
                Task::none()
            }

            AppMessage::ConnectDevice => {
                if self.session.is_some()
                    || matches!(
                        self.connection_status,
                        ConnectionStatus::Connecting | ConnectionStatus::Reconnecting(_)
                    )
                {
                    return Task::none();
                }
                let selected = self.selected_device.clone();
                let device = self
                    .devices
                    .iter()
                    .find(|d| Some(device_label(d)) == selected)
                    .cloned();
                let Some(device) = device else {
                    if verbose {
                        println!("[GUI VERBOSE] Selected device not in the scan results, re-scanning...");
                    }
                    self.scanning = true;
                    return full_scan_task();
                };

                if verbose {
                    println!("[GUI VERBOSE] Connecting to {}", device.name);
                }
                self.connection_status = ConnectionStatus::Connecting;
                Task::perform(
                    async move {
                        let session = Session::connect(&device, ConnectMode::Control)
                            .await
                            .map_err(|e| e.to_string())?;
                        session.start_polling(POLL_INTERVAL);
                        Ok(session)
                    },
                    AppMessage::Connected,
                )
            }

            AppMessage::Connected(result) => {
                match result {
                    Ok(session) => {
                        if verbose {
                            println!("[GUI VERBOSE] ✓ Connected to {}", session.name());
                        }
                        self.connection_status = ConnectionStatus::Connected;
                        self.status_seen = [false; 4];
                        self.emit("event", serde_json::json!({ "event": "connected", "device": session.name() }));
                        if self.settings.save_last_device {
                            self.settings.last_device_id = self.selected_device.clone();
                            let _ = crate::settings::save(&self.settings);
                        }
                        self.last_device = self.devices.iter().find(|d| d.id == session.id()).cloned();
                        self.session = Some(session);
                        return self.send_pending_stop();
                    }
                    Err(e) => {
                        if verbose {
                            println!("[GUI VERBOSE] ✗ Connection failed: {}", e);
                        }
                        self.emit("event", serde_json::json!({ "event": "connect_failed", "error": e }));
                        self.connection_status = ConnectionStatus::Error(e);
                        self.session = None;
                    }
                }
                Task::none()
            }

            AppMessage::ConnectionLost => {
                self.restarts = [None; 4];
                if self.session.take().is_none() {
                    return Task::none();
                }
                log::warn!("Connection to the charger was lost; reconnecting");
                self.emit("event", serde_json::json!({ "event": "connection_lost" }));
                if self.last_device.is_none() {
                    self.connection_status = ConnectionStatus::Error("Connection lost".to_string());
                    return Task::none();
                }
                self.connection_status = ConnectionStatus::Reconnecting(1);
                self.reconnect_task(1)
            }

            AppMessage::ReconnectResult(attempt, result) => {
                // The user disconnected (or connected by hand) meanwhile: drop this result.
                if self.connection_status != ConnectionStatus::Reconnecting(attempt) {
                    if let Ok(session) = result {
                        return Task::future(async move {
                            let _ = session.disconnect().await;
                        })
                        .discard();
                    }
                    return Task::none();
                }
                match result {
                    Ok(session) => {
                        log::info!("Reconnected to {} (attempt {attempt})", session.name());
                        self.emit("event", serde_json::json!({
                            "event": "reconnected", "device": session.name(), "attempt": attempt,
                        }));
                        self.connection_status = ConnectionStatus::Connected;
                        self.status_seen = [false; 4];
                        self.session = Some(session);
                        self.send_pending_stop()
                    }
                    Err(e) if attempt < MAX_RECONNECT_ATTEMPTS => {
                        log::warn!("Reconnect attempt {attempt} failed: {e}");
                        self.connection_status = ConnectionStatus::Reconnecting(attempt + 1);
                        self.reconnect_task(attempt + 1)
                    }
                    Err(e) => {
                        log::error!("Reconnecting failed: {e}");
                        self.emit("event", serde_json::json!({ "event": "reconnect_failed", "error": e }));
                        self.connection_status =
                            ConnectionStatus::Error(format!("Connection lost; reconnecting failed: {e}"));
                        Task::none()
                    }
                }
            }

            AppMessage::CommandFinished(kind, slots, result) => {
                self.commands_in_flight = self.commands_in_flight.saturating_sub(1);
                if kind == CommandKind::Start {
                    for (i, restart) in self.restarts.iter_mut().enumerate() {
                        if slots & (1 << i) != 0 {
                            *restart = match (restart.take(), result.is_ok()) {
                                (Some(r), true) => Some(Restart { sent: true, ..r }),
                                _ => None,
                            };
                        }
                    }
                }
                self.emit("event", serde_json::json!({
                    "event": "command_finished",
                    "command": format!("{kind:?}").to_lowercase(),
                    "slots": slots,
                    "error": result.as_ref().err(),
                }));
                if kind == CommandKind::Stop && result.is_err() {
                    // A stop must not get lost: deliver it on the next working connection.
                    self.queue_stop(slots);
                }
                if let Err(e) = result {
                    log::error!("Command for slot mask 0x{:02X} failed: {}", slots, e);
                    if verbose {
                        println!("[GUI VERBOSE] ✗ Command for slot mask 0x{:02X} failed: {}", slots, e);
                    }
                    for (i, slot) in self.slots.iter_mut().enumerate() {
                        if slots & (1 << i) != 0 {
                            slot.set_state(SlotState::Error(e.clone()));
                            self.slot_command_pending[i] = None;
                            self.slot_notices[i] =
                                Some(crate::i18n::t!("notice.command_failed", error = e.clone()).to_string());
                        }
                    }
                }
                if self.quit_deadline.is_some() && self.commands_in_flight == 0 {
                    return self.quit_now();
                }
                Task::none()
            }

            AppMessage::QuitRequested => self.request_quit(),

            AppMessage::RefreshDevices => {
                if verbose {
                    println!("[GUI VERBOSE] Refreshing device list (scanning Bluetooth...)...");
                }
                self.scanning = true;
                full_scan_task()
            }

            AppMessage::DisconnectDevice => {
                self.restarts = [None; 4];
                self.connection_status = ConnectionStatus::Disconnected;
                self.status_seen = [false; 4];
                for slot in &mut self.slots {
                    slot.stop();
                }
                match self.session.take() {
                    Some(session) => Task::future(async move {
                        let _ = session.disconnect().await;
                    })
                    .discard(),
                    None => Task::none(),
                }
            }

            AppMessage::SlotMessage(slot_id, slot_msg) => {
                if let Some(slot) = self.slots.get_mut(slot_id.0) {
                    match slot_msg {
                        SlotMessage::UpdateState(state) => {
                            slot.set_state(state);
                        }
                        SlotMessage::UpdateMeasurement(voltage, current) => {
                            slot.update_measurement(voltage, current);
                        }
                    }
                }
                Task::none()
            }
            
            AppMessage::StartTask(slot_id, config) => {
                if verbose {
                    println!("[START-SLOT] Starting task on slot {}", slot_id.0 + 1);
                }
                let slot_bit = 1u8 << slot_id.0;
                let charge_config = task_to_charge_config(slot_id.0, &config);
                let keep_running = self.active_mask_except(slot_bit);
                let task = self.run_command(slot_bit, move |session| async move {
                    session.start_slots(&[charge_config], keep_running).await
                });

                if let Some(slot) = self.slots.get_mut(slot_id.0) {
                    slot.start_task(config.clone());
                }
                self.slot_notices[slot_id.0] = None;
                self.slot_configs[slot_id.0] = Some(config);
                task
            }

            AppMessage::StopTask(slot_id) => {
                self.slot_command_pending[slot_id.0] = Some((Instant::now(), self.slots[slot_id.0].state.clone()));
                let slot_bit = 1u8 << slot_id.0;

                // FE(slot) + 0x93 0x00 stops only this slot; the others keep running.
                if verbose {
                    println!("[STOP-SLOT] Stopping slot {}", slot_id.0 + 1);
                }
                let task = self.stop_command(slot_bit);
                self.restarts[slot_id.0] = None;

                // The charger decides when the slot is idle: keep its state until a status
                // says so (the Stop button stays disabled meanwhile).
                if let Some(slot) = self.slots.get_mut(slot_id.0) {
                    slot.forget_task();
                }
                self.slot_configs[slot_id.0] = None;
                self.smart_probes[slot_id.0] = None;
                task
            }

            AppMessage::StopAllSlots => {
                // Every slot, not only those the GUI tracks: a program may have been started
                // from the charger's front panel.
                if verbose {
                    println!("[STOP-ALL] Stopping all slots");
                }
                let task = self.stop_command(0x0F);
                self.restarts = [None; 4];

                // States change when the charger reports the slots idle.
                for (i, slot) in self.slots.iter_mut().enumerate() {
                    if slot.is_active() || slot.state == SlotState::Paused {
                        self.slot_command_pending[i] = Some((Instant::now(), slot.state.clone()));
                    }
                    slot.forget_task();
                }
                self.smart_probes = [None; 4];
                task
            }

            AppMessage::ConfigureSlot(slot_id) => {
                self.configuring_slot = Some(slot_id);
                // Initialize with default or existing config
                if self.slot_configs[slot_id.0].is_none() {
                    self.slot_configs[slot_id.0] = Some(TaskConfig::default());
                }
                Task::none()
            }

            AppMessage::UpdateSlotChemistry(slot_id, chemistry) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.battery_chemistry = chemistry;
                    config.target_voltage = chemistry.target_voltage();
                    config.cutoff_voltage = Some(chemistry.cutoff_voltage());
                }
                Task::none()
            }

            AppMessage::UpdateSlotTaskType(slot_id, task_type) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.task_type = task_type;
                }
                Task::none()
            }

            AppMessage::UpdateSlotCapacity(slot_id, capacity) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.capacity_limit = Some(capacity);
                }
                Task::none()
            }

            AppMessage::UpdateSlotChargeCurrent(slot_id, current_ma) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.charge_current_ma = current_ma;
                    config.discharge_current_ma = current_ma.min(2000);
                }
                Task::none()
            }

            AppMessage::CancelSlotConfig(slot_id) => {
                self.configuring_slot = None;
                self.slot_configs[slot_id.0] = None;
                Task::none()
            }
            
            AppMessage::ApplySlotConfig(slot_id) => {
                if let Some(config) = self.slot_configs[slot_id.0].clone() {
                    self.configuring_slot = None;
                    return self.update(AppMessage::StartTask(slot_id, config));
                }
                Task::none()
            }
            
            AppMessage::SlotSelected(slot_index) => {
                self.selected_slot = Some(slot_index);
                Task::none()
            }
            
            AppMessage::ShowConfigDialog(slot_index, voltage) => {
                self.pending_slot = Some(slot_index);
                // Restore saved config for this slot, or create a new one
                let mut state = if let Some(saved) = self.saved_slot_configs[slot_index.0].clone() {
                    saved
                } else {
                    crate::ui::components::config_dialog::ConfigDialogState::new(voltage)
                };
                // Re-resolve selected_profile index from last_profile_name (in case profile list changed)
                state.resolve_profile(&self.profile_store);
                self.config_dialog_state = Some(state);
                Task::none()
            }
            
            AppMessage::ConfigChemistryChanged(chemistry) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.update_chemistry(chemistry);
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigEneloopTypeChanged(eneloop) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.update_eneloop_type(eneloop);
                    state.config_modified = true;
                }
                Task::none()
            }

            AppMessage::ConfigModeChanged(mode) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.update_mode(mode);
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigCapacityChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.capacity_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigChargeCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.charge_current_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigDischargeCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.discharge_current_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigTargetVoltageChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.target_voltage_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigCutoffVoltageChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.cutoff_voltage_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigStorageVoltageChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.storage_voltage_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigDeltaPeakChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.delta_peak_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigTrickleChargeChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.trickle_charge_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigCutoffTimerChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.cutoff_timer_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigChargeCutoffCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.charge_cutoff_current_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigDischargeCutoffCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.discharge_cutoff_current_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigChargeRestingChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.charge_resting_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigDischargeRestingChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.discharge_resting_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigCycleCountChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.cycle_count_input = value;
                    state.config_modified = true;
                }
                Task::none()
            }
            
            AppMessage::ConfigDialogCancel => {
                // Save current state for this slot before closing
                if let (Some(state), Some(slot_id)) = (&self.config_dialog_state, self.pending_slot) {
                    self.saved_slot_configs[slot_id.0] = Some(state.clone());
                }
                self.config_dialog_state = None;
                self.pending_slot = None;
                Task::none()
            }

            AppMessage::ConfigDialogDefault => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.reset_to_defaults();
                }
                Task::none()
            }

            AppMessage::ConfigProfileNameChanged(name) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.profile_name_input = name;
                    if state.selected_profile.is_some() {
                        state.config_modified = true;
                    }
                }
                Task::none()
            }

            AppMessage::ConfigSaveProfile => {
                if let Some(state) = &self.config_dialog_state {
                    let name = crate::profiles::Profile::generate_name(
                        state.chemistry,
                        state.mode,
                        &state.get_config(),
                    );
                    let profile = crate::profiles::Profile {
                        name,
                        chemistry: state.chemistry,
                        mode: state.mode,
                        config: state.get_config(),
                    };
                    self.profile_store.add_profile(profile);
                }
                Task::none()
            }

            AppMessage::ConfigDeleteProfile => {
                if let Some(state) = &self.config_dialog_state {
                    if let Some(idx) = state.selected_profile {
                        // Store for undo
                        if let Some(profile) = self.profile_store.profiles.get(idx) {
                            if let Some(s) = &mut self.config_dialog_state {
                                s.deleted_profile = Some(profile.clone());
                            }
                        }
                        self.profile_store.delete_profile(idx);
                        if let Some(s) = &mut self.config_dialog_state {
                            s.selected_profile = None;
                        }
                    }
                }
                Task::none()
            }

            AppMessage::ConfigUndoDelete => {
                if let Some(state) = &mut self.config_dialog_state {
                    if let Some(profile) = state.deleted_profile.take() {
                        self.profile_store.add_profile(profile);
                    }
                }
                Task::none()
            }

            AppMessage::ConfigUpdateProfile => {
                if let Some(state) = &self.config_dialog_state {
                    if let Some(idx) = state.selected_profile {
                        let config = state.get_config();
                        let name = if state.profile_name_input.trim().is_empty() {
                            crate::profiles::Profile::generate_name(state.chemistry, state.mode, &config)
                        } else {
                            state.profile_name_input.trim().to_string()
                        };
                        if let Some(profile) = self.profile_store.profiles.get_mut(idx) {
                            profile.name = name;
                            profile.chemistry = state.chemistry;
                            profile.mode = state.mode;
                            profile.config = config;
                        }
                        let _ = self.profile_store.save();
                        if let Some(s) = &mut self.config_dialog_state {
                            s.config_modified = false;
                        }
                    }
                }
                Task::none()
            }

            AppMessage::ConfigSelectProfile(idx) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.selected_profile = Some(idx);
                    // Apply profile to dialog and remember its name for persistence
                    if let Some(profile) = self.profile_store.profiles.get(idx) {
                        state.apply_profile(profile);
                        state.last_profile_name = Some(profile.name.clone());
                        state.config_modified = false;
                    }
                }
                Task::none()
            }

            AppMessage::ConfigExportProfiles => {
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Export Profiles")
                            .set_file_name("mc5000_profiles.json")
                            .add_filter("JSON", &["json"])
                            .save_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    AppMessage::ConfigProfilesExported,
                )
            }

            AppMessage::ConfigImportProfiles => {
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import Profiles")
                            .add_filter("JSON", &["json"])
                            .pick_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    AppMessage::ConfigProfilesImported,
                )
            }

            AppMessage::ConfigProfilesExported(path) => {
                if let Some(path) = path {
                    if let Err(e) = self.profile_store.export_to_file(&path) {
                        log::error!("Failed to export profiles: {}", e);
                    }
                }
                Task::none()
            }

            AppMessage::ConfigProfilesImported(path) => {
                if let Some(path) = path {
                    match crate::profiles::ProfileStore::import_from_file(&path) {
                        Ok(profiles) => {
                            for p in profiles {
                                self.profile_store.add_profile(p);
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to import profiles: {}", e);
                        }
                    }
                }
                Task::none()
            }
            
            AppMessage::ConfigDialogConfirm => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                // Refuse settings whose chemistry cannot match the cell in the slot.
                if let (Some(state), Some(slot_id)) = (&mut self.config_dialog_state, self.pending_slot) {
                    let voltage = self.slots[slot_id.0].current_voltage;
                    if let Err(mismatch) = crate::auto_charge::check_voltage_fits(state.chemistry, voltage) {
                        let chemistry = state.chemistry.to_string();
                        let message = match mismatch {
                            crate::auto_charge::VoltageMismatch::TooHigh { voltage, limit } => crate::i18n::t!(
                                "config.voltage_too_high",
                                voltage = format!("{voltage:.2}"),
                                chemistry = chemistry,
                                limit = format!("{limit:.2}")
                            ),
                            crate::auto_charge::VoltageMismatch::TooLow { voltage, min } => crate::i18n::t!(
                                "config.voltage_too_low",
                                voltage = format!("{voltage:.2}"),
                                chemistry = chemistry,
                                min = format!("{min:.2}")
                            ),
                        }
                        .to_string();
                        log::warn!("Slot {}: start refused: {mismatch}", slot_id.0 + 1);
                        state.error = Some(message);
                        return Task::none();
                    }
                }
                if let (Some(state), Some(slot_id)) = 
                    (&self.config_dialog_state, self.pending_slot) 
                {
                    let config = state.get_config();
                    let chemistry = state.chemistry;
                    let mode = state.mode;
                    
                    // Mark slot command pending + auto-select this slot
                    self.slot_command_pending[slot_id.0] = Some((Instant::now(), self.slots[slot_id.0].state.clone()));
                    self.selected_slot = Some(slot_id.0);
                    
                    // Save config for this slot, then close dialog
                    // Persist to disk so it survives app restarts
                    let persisted = crate::slot_persist::PersistedSlotConfig {
                        chemistry: state.chemistry,
                        mode: state.mode,
                        config: state.get_config(),
                        last_profile_name: state.last_profile_name.clone(),
                    };
                    self.slot_config_store.set(slot_id.0, Some(persisted));
                    self.slot_config_store.save();
                    self.saved_slot_configs[slot_id.0] = Some(state.clone());
                    self.config_dialog_state = None;
                    self.pending_slot = None;
                    
                    // Send configuration to device if connected via Bluetooth
                    if self.session.is_some() {
                        {
                            if verbose {
                                println!("[GUI VERBOSE] Starting charge from config dialog on slot {}", slot_id.0 + 1);
                                println!("[GUI VERBOSE]   Chemistry: {:?}", chemistry);
                                println!("[GUI VERBOSE]   Mode: {:?}", mode);
                                println!("[GUI VERBOSE]   Capacity: {}mAh", config.capacity_mah);
                                println!("[GUI VERBOSE]   Charge current: {}mA", config.charge_current_ma);
                                println!("[GUI VERBOSE]   Target voltage: {}mV", config.target_voltage_mv);
                            }
                            
                            let bt_config = crate::auto_charge::to_protocol_config(
                                slot_id.0, chemistry, mode, &config,
                            );

                            // The start mask keeps every slot that is already active.
                            let slot_bit = 1u8 << slot_id.0;
                            let keep_running = self.active_mask_except(slot_bit);
                            if verbose {
                                println!("[GUI VERBOSE] Sending config + start (mask 0x{:02X})", keep_running | slot_bit);
                            }
                            // The slot shows running when the charger reports it (Configure stays
                            // disabled until then); a failed command sets the slot to Error.
                            self.slot_notices[slot_id.0] = None;
                            return self.run_command(slot_bit, move |session| async move {
                                session.start_slots(&[bt_config], keep_running).await
                            });
                        }
                    }
                } else {
                    self.config_dialog_state = None;
                    self.pending_slot = None;
                }
                
                Task::none()
            }
            
            AppMessage::SimpleAutoCharge => self.auto_charge(false),

            AppMessage::SmartChargeAll => self.auto_charge(true),

            AppMessage::ExportData => {
                // Default export: time-aligned format
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_file_name("charger_data_aligned.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    AppMessage::FileSelectedTimeAligned,
                )
            }
            
            AppMessage::ExportTimeAligned => {
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_file_name("charger_data_aligned.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    AppMessage::FileSelectedTimeAligned,
                )
            }
            
            AppMessage::ExportAllSamples => {
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_file_name("charger_data_all_samples.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    AppMessage::FileSelectedAllSamples,
                )
            }
            
            AppMessage::FileSelected(path) => {
                // Legacy handler - redirect to time-aligned
                if let Some(path) = path {
                    let aligned_data = self.data_logger.get_time_aligned_data();
                    if let Err(e) = crate::export::export_time_aligned(&path, &aligned_data) {
                        log::error!("Failed to export data: {}", e);
                        eprintln!("Failed to export data: {}", e);
                    } else {
                        println!("✓ Exported {} time-aligned rows to {:?}", aligned_data.len(), path);
                    }
                }
                Task::none()
            }
            
            AppMessage::FileSelectedTimeAligned(path) => {
                if let Some(path) = path {
                    let aligned_data = self.data_logger.get_time_aligned_data();
                    if let Err(e) = crate::export::export_time_aligned(&path, &aligned_data) {
                        log::error!("Failed to export time-aligned data: {}", e);
                        eprintln!("Failed to export time-aligned data: {}", e);
                    } else {
                        println!("✓ Exported {} time-aligned rows to {:?}", aligned_data.len(), path);
                    }
                }
                Task::none()
            }
            
            AppMessage::FileSelectedAllSamples(path) => {
                if let Some(path) = path {
                    let measurements = self.data_logger.get_all_measurements();
                    if let Err(e) = self.csv_exporter.export_to_file(&path, measurements.iter().copied()) {
                        log::error!("Failed to export all samples: {}", e);
                        eprintln!("Failed to export all samples: {}", e);
                    } else {
                        println!("✓ Exported {} individual samples to {:?}", measurements.len(), path);
                    }
                }
                Task::none()
            }
            
            AppMessage::ToggleDetailedStats => {
                self.show_detailed_stats = !self.show_detailed_stats;
                Task::none()
            }
            
            AppMessage::HistoryReceived(request, result) => {
                self.backfill_history(request, result);
                Task::none()
            }

            AppMessage::GraphToggleMode => {
                use crate::settings::GraphTimeMode;
                self.settings.graph_time_mode = match self.settings.graph_time_mode {
                    GraphTimeMode::Roll => GraphTimeMode::Infinite,
                    GraphTimeMode::Infinite => GraphTimeMode::Roll,
                };
                let _ = crate::settings::save(&self.settings);
                Task::none()
            }

            AppMessage::GraphWindowInput(input) => {
                self.graph_window_input = input;
                Task::none()
            }

            AppMessage::GraphWindowApply => {
                match self.graph_window_input.trim().parse::<u32>() {
                    Ok(seconds) if seconds >= 10 => {
                        self.settings.graph_window_s = seconds;
                        let _ = crate::settings::save(&self.settings);
                    }
                    _ => {}
                }
                self.graph_window_input = self.settings.graph_window_s.to_string();
                Task::none()
            }

            AppMessage::GraphClear => {
                self.graph_epoch = Some(Utc::now());
                Task::none()
            }

            AppMessage::ClearData => {
                self.data_logger.clear();
                self.graph_epoch = None;
                Task::none()
            }
            
            AppMessage::SettingsOpen => {
                self.show_settings = true;
                Task::none()
            }
            
            AppMessage::SettingsClose => {
                self.show_settings = false;
                Task::none()
            }
            
            AppMessage::SettingsChangeTheme(theme) => {
                self.settings.theme = theme;
                let _ = crate::settings::save(&self.settings);
                Task::none()
            }
            
            AppMessage::SettingsChangeLanguage(lang) => {
                self.settings.language = lang.clone();
                crate::i18n::set_language(&lang);
                let _ = crate::settings::save(&self.settings);
                Task::none()
            }
            
            AppMessage::SettingsToggleSaveDevice => {
                self.settings.save_last_device = !self.settings.save_last_device;
                if !self.settings.save_last_device {
                    self.settings.last_device_id = None;
                }
                let _ = crate::settings::save(&self.settings);
                Task::none()
            }
            
            AppMessage::SettingsOpenRepo => {
                let _ = open::that("https://github.com/rssdev10/skyrc-mc-rs");
                Task::none()
            }

            AppMessage::Control(line) => {
                self.control_seq += 1;
                self.handle_control(&line)
            }
        }
    }

    pub fn view(&self) -> Element<'_, AppMessage> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        if verbose && self.update_counter.is_multiple_of(10) {
            use std::io::Write;
            println!("[GUI VERBOSE] View rendered, update_counter: {}", self.update_counter);
            let _ = std::io::stdout().flush();
        }
        
        if self.show_settings {
            return crate::ui::components::settings_dialog::view(&self.settings);
        }
        
        ui::main_view(
            self.devices.iter().map(device_label).collect(),
            self.session.as_ref().map(Session::name),
            &self.slots,
            &self.data_logger,
            &self.connection_status,
            &self.selected_device,
            &self.slot_configs,
            &self.configuring_slot,
            self.scanning,
            self.selected_slot,
            &self.config_dialog_state,
            self.show_detailed_stats,
            &self.profile_store,
            self.buttons_locked(),
            &self.slot_command_pending,
            &self.slot_notices,
            ui::components::graph_panel::GraphOptions {
                mode: self.settings.graph_time_mode,
                window_s: self.settings.graph_window_s,
                window_input: &self.graph_window_input,
                epoch: self.graph_epoch,
            },
        )
    }

    pub fn subscription(&self) -> Subscription<AppMessage> {
        let tick = iced::time::every(Duration::from_millis(1000))
            .map(|_| AppMessage::Tick);
        
        // Decoded slot statuses from the shared session, while connected.
        let session_events = match &self.session {
            Some(session) => Subscription::run_with(
                SessionSubscription(session.clone()),
                session_stream,
            ),
            None => Subscription::none(),
        };

        // Text control channel (headless runs and agents).
        let control = match &self.control {
            Some(control) => Subscription::run_with(
                crate::control::ControlSubscription {
                    options: control.options.clone(),
                    snapshots: control.snapshots.subscribe(),
                },
                crate::control::control_stream,
            ),
            None => Subscription::none(),
        };

        let close = iced::window::close_requests().map(|_| AppMessage::QuitRequested);

        Subscription::batch([tick, session_events, control, close])
    }

    pub fn theme(&self) -> Theme {
        match self.settings.theme {
            crate::settings::AppTheme::Light => Theme::Light,
            crate::settings::AppTheme::Dark => Theme::Dark,
        }
    }

    /// Returns true if Auto/SmartCharge buttons should be disabled (5s cooldown)
    fn buttons_locked(&self) -> bool {
        self.buttons_locked_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }
}

/// The notice for a charger error code (status byte 20), in the user's language.
fn charger_error_notice(code: u8) -> String {
    use crate::i18n::t;
    let error = match code {
        1 => t!("charger_error.input_low"),
        2 => t!("charger_error.input_high"),
        3 | 9 => t!("charger_error.connection"),
        4 => t!("charger_error.capacity_limit"),
        5 => t!("charger_error.time_limit"),
        6 => t!("charger_error.internal_temp"),
        7 => t!("charger_error.calibration"),
        8 => t!("charger_error.resistance"),
        10 => t!("charger_error.battery_type"),
        11 => t!("charger_error.overload"),
        12 => t!("charger_error.reversed"),
        13 => t!("charger_error.full"),
        _ => t!("charger_error.unknown", code = code),
    };
    t!("notice.charger_error", error = error).to_string()
}

fn map_bt_state(state: &BtSlotState) -> SlotState {
    match state {
        BtSlotState::Empty | BtSlotState::Idle => SlotState::Idle,
        BtSlotState::Charging | BtSlotState::ChargingCC | BtSlotState::ChargingCV => SlotState::Charging,
        BtSlotState::Discharging => SlotState::Discharging,
        BtSlotState::Completed => SlotState::Completed,
        BtSlotState::Paused => SlotState::Paused,
        BtSlotState::Error => SlotState::Error("Error".to_string()),
    }
}

/// Convert a TaskConfig + slot index to a ChargeConfig for the BLE protocol.
fn task_to_charge_config(slot_idx: usize, config: &TaskConfig) -> ChargeConfig {
    let bt_chem = config.battery_chemistry.to_bluetooth_chemistry();
    let mode = match config.task_type {
        TaskType::Charge => OperationMode::Charge,
        TaskType::Discharge => OperationMode::Discharge,
        TaskType::Storage => OperationMode::Storage,
        TaskType::Cycle { .. } => OperationMode::Cycle,
        _ => OperationMode::Charge,
    };
    ChargeConfig::new(
        (slot_idx + 1) as u8,
        bt_chem,
        mode,
        config.capacity_limit.unwrap_or(3000) as u16,
        config.charge_current_ma,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_app() -> ChargerApp {
        let (app, _task) = ChargerApp::new();
        app
    }

    #[test]
    fn test_initial_state() {
        let app = create_test_app();
        assert_eq!(app.connection_status, ConnectionStatus::Disconnected);
        assert!(app.scanning); // starts with scan active
        assert!(app.selected_device.is_none());
        assert!(app.session.is_none());
        assert!(app.config_dialog_state.is_none());
        assert!(!app.show_detailed_stats);
        for slot in &app.slots {
            assert_eq!(slot.state, SlotState::Idle);
        }
    }

    #[test]
    fn test_toggle_detailed_stats() {
        let mut app = create_test_app();
        assert!(!app.show_detailed_stats);
        let _ = app.update(AppMessage::ToggleDetailedStats);
        assert!(app.show_detailed_stats);
        let _ = app.update(AppMessage::ToggleDetailedStats);
        assert!(!app.show_detailed_stats);
    }

    #[test]
    fn test_slot_selected() {
        let mut app = create_test_app();
        assert_eq!(app.selected_slot, Some(0)); // default
        let _ = app.update(AppMessage::SlotSelected(2));
        assert_eq!(app.selected_slot, Some(2));
    }

    #[test]
    fn test_device_selected() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::DeviceSelected("test-device".to_string()));
        assert_eq!(app.selected_device, Some("test-device".to_string()));
    }

    #[test]
    fn test_bluetooth_scan_complete_success() {
        let mut app = create_test_app();
        app.scanning = true;
        app.selected_device = Some("MC5000 BT: gone (ID:1)".to_string());
        let _ = app.update(AppMessage::BluetoothScanComplete(Ok(Vec::new())));
        assert!(!app.scanning);
        // A selection that is no longer in the scan results is cleared.
        assert!(app.selected_device.is_none());
    }

    #[test]
    fn test_bluetooth_scan_complete_error() {
        let mut app = create_test_app();
        assert!(app.scanning);
        let _ = app.update(AppMessage::BluetoothScanComplete(Err("scan failed".to_string())));
        assert!(!app.scanning);
        // Error is logged but connection status stays disconnected
        assert_eq!(app.connection_status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn test_config_dialog_cancel() {
        let mut app = create_test_app();
        app.config_dialog_state = Some(
            crate::ui::components::config_dialog::ConfigDialogState::new(3.7),
        );
        app.pending_slot = Some(SlotId(0));
        let _ = app.update(AppMessage::ConfigDialogCancel);
        assert!(app.config_dialog_state.is_none());
    }

    fn status(slot_id: u8, voltage_mv: u16, current_ma: u16) -> SlotStatus {
        SlotStatus {
            slot_id,
            state: if current_ma > 0 { BtSlotState::Charging } else { BtSlotState::Idle },
            voltage_mv,
            current_ma,
            capacity_mah: 0,
            elapsed_seconds: 0,
            resistance_milliohm: 0,
            temperature_c: 0,
            phase: Some(if current_ma > 0 { 2 } else { 0 }),
            mode: None,
            chemistry: None,
            error: Some(0),
        }
    }

    #[test]
    fn eneloop_type_sets_defaults_survives_a_mode_change_and_reaches_the_charger() {
        use skyrc_mc5000::EneloopType;
        let mut app = create_test_app();
        let _ = app.update(AppMessage::ConfigureSlot(SlotId(1)));
        app.config_dialog_state = Some(crate::ui::components::config_dialog::ConfigDialogState::new(1.3));
        let _ = app.update(AppMessage::ConfigChemistryChanged(BatteryChemistry::Eneloop));
        let _ = app.update(AppMessage::ConfigEneloopTypeChanged(EneloopType::LiteAA));
        let state = app.config_dialog_state.as_ref().unwrap();
        assert_eq!((state.capacity_input.as_str(), state.charge_current_input.as_str()), ("1000", "500"));

        let _ = app.update(AppMessage::ConfigModeChanged(crate::config_dialog::ChargeMode::Discharge));
        let state = app.config_dialog_state.as_ref().unwrap();
        assert_eq!(state.eneloop_type(), EneloopType::LiteAA, "kept across the mode change");
        assert_eq!(state.discharge_current_input, "250");

        let config = crate::auto_charge::to_protocol_config(
            1,
            BatteryChemistry::Eneloop,
            crate::config_dialog::ChargeMode::Discharge,
            &state.get_config(),
        );
        assert_eq!(config.eneloop_type, EneloopType::LiteAA);
    }

    #[test]
    fn charger_error_code_shows_a_notice_until_the_charger_clears_it() {
        let mut app = create_test_app();
        let mut s = status(2, 1300, 0);
        s.error = Some(10);
        let _ = app.update(AppMessage::StatusReceived(s.clone()));
        assert_eq!(app.slot_notices[1], Some(super::charger_error_notice(10)));
        assert_eq!(app.snapshot().slots[1].charger_error, Some(10));

        // The same code again leaves a newer notice alone.
        app.slot_notices[1] = Some("other".into());
        let _ = app.update(AppMessage::StatusReceived(s.clone()));
        assert_eq!(app.slot_notices[1].as_deref(), Some("other"));

        // Cleared code: only its own notice is removed.
        app.slot_notices[1] = Some(super::charger_error_notice(10));
        s.error = Some(0);
        let _ = app.update(AppMessage::StatusReceived(s));
        assert_eq!(app.slot_notices[1], None);
        assert_eq!(app.snapshot().slots[1].charger_error, None);
    }

    #[test]
    fn test_status_received_updates_the_slot() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(4, 4128, 0)));
        assert!((app.slots[3].current_voltage - 4.128).abs() < 1e-3);
        assert_eq!(app.slots[3].state, SlotState::Idle);
        assert_eq!(app.slots[0].current_voltage, 0.0);
    }

    #[test]
    fn test_discharging_status_shows_as_discharging() {
        let mut app = create_test_app();
        let mut discharging = status(4, 4094, 504);
        discharging.state = BtSlotState::Discharging;
        discharging.mode = Some(OperationMode::Discharge);
        discharging.chemistry = Some(skyrc_mc5000::BatteryChemistry::LiIon);
        let _ = app.update(AppMessage::StatusReceived(discharging));
        assert_eq!(app.slots[3].state, SlotState::Discharging);
        assert_eq!(app.reported_modes[3].as_deref(), Some("Li-Ion Discharge"));
    }

    fn probe_status(slot_id: u8, resistance_milliohm: u16) -> SlotStatus {
        let mut s = status(slot_id, 1300, 200);
        s.resistance_milliohm = resistance_milliohm;
        s.elapsed_seconds = 8;
        s
    }

    #[test]
    fn test_smart_step_charges_at_the_current_for_the_measured_resistance() {
        let mut app = create_test_app();
        app.smart_probes[1] = Some(SmartProbe { chemistry: BatteryChemistry::NiMH, started: Instant::now() });
        let _ = app.update(AppMessage::StatusReceived(probe_status(2, 230)));
        assert!(app.smart_probes[1].is_none(), "probe finished");
        let task = app.slots[1].current_task.as_ref().expect("task recorded");
        assert_eq!(task.charge_current_ma, 400);
        assert_eq!(task.battery_chemistry, BatteryChemistry::NiMH);
    }

    #[test]
    fn test_smart_step_waits_for_a_settled_reading() {
        let mut app = create_test_app();
        app.smart_probes[0] = Some(SmartProbe { chemistry: BatteryChemistry::NiMH, started: Instant::now() });
        let mut early = probe_status(1, 60);
        early.elapsed_seconds = 2;
        let _ = app.update(AppMessage::StatusReceived(early));
        assert!(app.smart_probes[0].is_some(), "still probing");
    }

    #[test]
    fn test_smart_charges_a_worn_nickel_cell_gently_with_a_notice() {
        let mut app = create_test_app();
        app.smart_probes[3] = Some(SmartProbe { chemistry: BatteryChemistry::NiMH, started: Instant::now() });
        // The AA that read 548 mΩ on a live charger.
        let _ = app.update(AppMessage::StatusReceived(probe_status(4, 548)));
        let task = app.slots[3].current_task.as_ref().expect("charged");
        assert_eq!(task.charge_current_ma, 200);
        assert!(app.slot_notices[3].is_some(), "worn-cell notice shown");
    }

    #[test]
    fn test_smart_refusal_notice_survives_the_next_status_update() {
        let mut app = create_test_app();
        app.smart_probes[0] = Some(SmartProbe { chemistry: BatteryChemistry::NiMH, started: Instant::now() });
        let _ = app.update(AppMessage::StatusReceived(probe_status(1, 1500)));
        assert!(app.smart_probes[0].is_none());
        assert!(app.slot_notices[0].is_some());
        // The charger then reports the slot idle; the reason must stay visible.
        let _ = app.update(AppMessage::StatusReceived(status(1, 1290, 0)));
        assert_eq!(app.slots[0].state, SlotState::Idle);
        assert!(app.slot_notices[0].is_some(), "notice kept");
        // Removing the cell clears it.
        let _ = app.update(AppMessage::StatusReceived(status(1, 0, 0)));
        assert!(app.slot_notices[0].is_none());
    }

    #[test]
    fn test_confirm_refuses_a_chemistry_that_cannot_match_the_cell() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(1, 4110, 0))); // a Li-Ion cell
        let _ = app.update(AppMessage::ShowConfigDialog(SlotId(0), 4.11));
        let _ = app.update(AppMessage::ConfigChemistryChanged(BatteryChemistry::NiMH));
        let _ = app.update(AppMessage::ConfigDialogConfirm);
        let dialog = app.config_dialog_state.as_ref().expect("dialog stays open");
        assert!(dialog.error.is_some(), "reason is shown");
        assert_eq!(app.slots[0].state, SlotState::Idle, "nothing started");

        // Choosing a matching chemistry clears the reason.
        let _ = app.update(AppMessage::ConfigChemistryChanged(BatteryChemistry::LiIon));
        assert!(app.config_dialog_state.as_ref().unwrap().error.is_none());
    }

    /// A transport that accepts every write and never answers.
    struct NullTransport;

    impl skyrc_mc5000::Transport for NullTransport {
        fn write<'a>(&'a self, _: &'a [u8]) -> futures::future::BoxFuture<'a, Result<(), ChargerError>> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> futures::future::BoxFuture<'_, bool> {
            Box::pin(async { true })
        }
        fn disconnect(&self) -> futures::future::BoxFuture<'_, Result<(), ChargerError>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn test_session() -> Session {
        use futures::StreamExt;
        Session::open(NullTransport, futures::stream::pending().boxed(), "#Charger-TEST", "test")
    }

    #[test]
    fn test_stop_without_a_link_is_queued() {
        let mut app = create_test_app();
        app.connection_status = ConnectionStatus::Reconnecting(1);
        app.slots[1].state = SlotState::Charging;
        let _ = app.update(AppMessage::StopTask(SlotId(1)));
        let _ = app.update(AppMessage::StopAllSlots);
        assert_eq!(app.pending_stop, 0x0F);
    }

    #[test]
    fn test_a_failed_stop_is_queued() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::CommandFinished(CommandKind::Stop, 0x02, Err("timeout".into())));
        assert_eq!(app.pending_stop, 0x02);
        let _ = app.update(AppMessage::CommandFinished(CommandKind::Start, 0x04, Err("timeout".into())));
        assert_eq!(app.pending_stop, 0x02, "starts are not retried");
    }

    #[tokio::test]
    async fn test_reconnect_restores_the_session_and_sends_queued_stops() {
        let mut app = create_test_app();
        app.connection_status = ConnectionStatus::Reconnecting(2);
        app.pending_stop = 0x08;
        let _ = app.update(AppMessage::ReconnectResult(2, Ok(test_session())));
        assert_eq!(app.connection_status, ConnectionStatus::Connected);
        assert!(app.session.is_some());
        assert_eq!(app.pending_stop, 0, "queued stop handed to the new session");
    }

    #[test]
    fn test_failed_reconnects_retry_then_give_up() {
        let mut app = create_test_app();
        app.connection_status = ConnectionStatus::Reconnecting(1);
        let _ = app.update(AppMessage::ReconnectResult(1, Err("no charger".into())));
        assert_eq!(app.connection_status, ConnectionStatus::Reconnecting(2));

        app.connection_status = ConnectionStatus::Reconnecting(MAX_RECONNECT_ATTEMPTS);
        let _ = app.update(AppMessage::ReconnectResult(MAX_RECONNECT_ATTEMPTS, Err("no charger".into())));
        assert!(matches!(app.connection_status, ConnectionStatus::Error(_)));
    }

    #[tokio::test]
    async fn test_reconnect_after_the_user_disconnected_is_dropped() {
        let mut app = create_test_app();
        app.connection_status = ConnectionStatus::Disconnected;
        let _ = app.update(AppMessage::ReconnectResult(1, Ok(test_session())));
        assert!(app.session.is_none());
        assert_eq!(app.connection_status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn test_control_channel_sees_the_real_initial_state() {
        let options = crate::control::ControlOptions {
            source: crate::control::Source::Script(std::sync::Arc::new(Vec::new())),
            quit_at_end: true,
        };
        let (app, _task) = ChargerApp::new_with(Some(options));
        let snapshots = app.control.as_ref().unwrap().snapshots.subscribe();
        // Before any message: the startup scan is running, so `wait-until scanned` must wait.
        assert!(snapshots.borrow().scanning);
        assert_eq!(snapshots.borrow().slots.len(), 4);
    }

    #[test]
    fn test_status_for_unknown_slot_is_ignored() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(0, 4128, 500)));
        let _ = app.update(AppMessage::StatusReceived(status(7, 4128, 500)));
        for slot in &app.slots {
            assert_eq!(slot.state, SlotState::Idle);
            assert_eq!(slot.current_voltage, 0.0);
        }
    }

    #[test]
    fn test_label_round_trips_the_peripheral_id() {
        assert_eq!(
            label_peripheral_id("MC5000 BT: #Charger-FF34 (ID:ad46-1dff)").as_deref(),
            Some("ad46-1dff")
        );
        assert_eq!(label_peripheral_id("no id here"), None);
    }

    #[test]
    fn test_active_mask_except_skips_excluded_slots() {
        let mut app = create_test_app();
        app.slots[0].state = SlotState::Charging;
        app.slots[2].state = SlotState::Discharging;
        assert_eq!(app.active_mask_except(0), 0b0101);
        assert_eq!(app.active_mask_except(0b0001), 0b0100);
    }

    #[test]
    fn test_stop_without_connection_is_queued_and_keeps_the_state() {
        let mut app = create_test_app();
        app.slots[1].state = SlotState::Charging;
        let _ = app.update(AppMessage::StopAllSlots);
        assert_eq!(app.pending_stop, 0x0F, "delivered on the next connection");
        assert_eq!(app.slots[1].state, SlotState::Charging, "not idle until the charger says so");
        assert!(app.slot_command_pending[1].is_some(), "Stop stays disabled meanwhile");
    }

    #[test]
    fn test_started_slot_shows_running_only_when_the_charger_reports_it() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        let task = TaskConfig::default();
        app.slot_command_pending[1] = Some((Instant::now(), app.slots[1].state.clone()));
        let _ = app.update(AppMessage::StartTask(SlotId(1), task));
        assert_eq!(app.slots[1].state, SlotState::Idle, "not running until the charger says so");
        assert!(app.slots[1].current_task.is_some());
        assert!(app.snapshot().slots[1].awaiting_charger);

        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 200)));
        assert_eq!(app.slots[1].state, SlotState::Charging);
        assert!(!app.snapshot().slots[1].awaiting_charger);

        // A program the charger ends at once shows as idle at once (no grace period).
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        assert_eq!(app.slots[1].state, SlotState::Idle);
    }

    /// A charger that accepts every write and never answers: enough for code paths that
    /// need a session but not a reply.
    struct SilentCharger;

    impl skyrc_mc5000::Transport for SilentCharger {
        fn write<'a>(
            &'a self,
            _packet: &'a [u8],
        ) -> futures::future::BoxFuture<'a, Result<(), skyrc_mc5000::Error>> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> futures::future::BoxFuture<'_, bool> {
            Box::pin(async { true })
        }
        fn disconnect(&self) -> futures::future::BoxFuture<'_, Result<(), skyrc_mc5000::Error>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn silent_session() -> Session {
        use futures::StreamExt;
        Session::open(SilentCharger, futures::stream::pending().boxed(), "#Charger-TEST", "test")
    }

    #[tokio::test]
    async fn test_confirm_starts_without_marking_the_slot_running() {
        let mut app = create_test_app();
        app.session = Some(silent_session());
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        app.pending_slot = Some(SlotId(1));
        app.config_dialog_state = Some(crate::ui::components::config_dialog::ConfigDialogState::new(1.3));
        let _ = app.update(AppMessage::ConfigChemistryChanged(BatteryChemistry::NiMH));
        let _ = app.update(AppMessage::ConfigDialogConfirm);
        assert!(app.config_dialog_state.is_none(), "dialog confirmed");
        assert_eq!(app.slots[1].state, SlotState::Idle, "not running until the charger says so");
        assert!(app.slot_command_pending[1].is_some());
    }

    #[test]
    fn test_smart_restart_does_not_show_the_idle_in_between() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 200)));
        assert_eq!(app.slots[1].state, SlotState::Charging);
        app.restarts[1] = Some(Restart { since: Instant::now(), sent: false });
        app.commands_in_flight = 1;
        assert!(app.snapshot().slots[1].awaiting_charger);

        // The charger stops the probe program before starting the chosen one.
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        assert_eq!(app.slots[1].state, SlotState::Charging, "restart gap hidden");

        let _ = app.update(AppMessage::CommandFinished(CommandKind::Start, 0x02, Ok(())));
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        assert_eq!(app.slots[1].state, SlotState::Charging, "still between programs");

        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 250)));
        assert!(app.restarts[1].is_none(), "restart done");
        assert!(!app.snapshot().slots[1].awaiting_charger);

        // Afterwards an idle report shows at once.
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        assert_eq!(app.slots[1].state, SlotState::Idle);
    }

    #[test]
    fn test_restart_gives_up_after_the_wait_and_on_stop() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 200)));
        app.restarts[1] = Some(Restart { since: Instant::now() - COMMAND_CONFIRM_WAIT, sent: true });
        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 0)));
        assert_eq!(app.slots[1].state, SlotState::Idle, "charger did not restart: shown");

        let _ = app.update(AppMessage::StatusReceived(status(2, 1300, 200)));
        app.restarts[1] = Some(Restart { since: Instant::now(), sent: false });
        let _ = app.update(AppMessage::StopTask(SlotId(1)));
        assert!(app.restarts[1].is_none(), "a stop is never hidden");
    }

    #[test]
    fn test_stopped_slot_turns_idle_only_when_the_charger_reports_it() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::StatusReceived(status(3, 1300, 400)));
        assert_eq!(app.slots[2].state, SlotState::Charging);

        let _ = app.update(AppMessage::StopTask(SlotId(2)));
        assert_eq!(app.slots[2].state, SlotState::Charging);
        assert!(app.slots[2].current_task.is_none());

        // Still charging: the stop has not reached the charger yet.
        let _ = app.update(AppMessage::StatusReceived(status(3, 1300, 400)));
        assert_eq!(app.slots[2].state, SlotState::Charging);
        assert!(app.slot_command_pending[2].is_some());

        let _ = app.update(AppMessage::StatusReceived(status(3, 1300, 0)));
        assert_eq!(app.slots[2].state, SlotState::Idle);
        assert!(app.slot_command_pending[2].is_none(), "Stop/Configure usable again");
    }

    #[test]
    fn test_quit_waits_for_commands_in_flight() {
        let mut app = create_test_app();
        app.commands_in_flight = 1; // e.g. a stop on its way to the charger
        let _ = app.update(AppMessage::QuitRequested);
        assert!(app.quit_deadline.is_some(), "waits instead of exiting");

        let _ = app.update(AppMessage::Tick);
        assert!(app.quit_deadline.is_some(), "deadline not reached");

        let _ = app.update(AppMessage::CommandFinished(CommandKind::Stop, 0x08, Ok(())));
        assert_eq!(app.commands_in_flight, 0);
        assert!(app.quit_deadline.is_none(), "exits once the stop went out");
    }

    #[test]
    fn test_quit_exits_at_the_deadline_and_at_once_when_idle() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::QuitRequested);
        assert!(app.quit_deadline.is_none(), "nothing in flight: exits at once");

        app.commands_in_flight = 1;
        let _ = app.update(AppMessage::QuitRequested);
        app.quit_deadline = Some(Instant::now() - Duration::from_secs(1));
        let _ = app.update(AppMessage::Tick);
        assert!(app.quit_deadline.is_none(), "gave up waiting");
    }

    #[test]
    fn test_connection_lost_clears_status() {
        let mut app = create_test_app();
        app.connection_status = ConnectionStatus::Connected;
        let _ = app.update(AppMessage::ConnectionLost);
        // Without a session there was nothing to lose: status is unchanged.
        assert_eq!(app.connection_status, ConnectionStatus::Connected);
    }

    #[test]
    fn test_tick_does_not_panic_when_disconnected() {
        let mut app = create_test_app();
        // Tick should not panic when no device is connected
        let _ = app.update(AppMessage::Tick);
    }

    #[test]
    fn test_clear_data() {
        let mut app = create_test_app();
        let _ = app.update(AppMessage::ClearData);
        // Should not panic; data logger should be cleared
    }

    #[test]
    fn test_connection_status_variants() {
        assert_eq!(ConnectionStatus::Disconnected, ConnectionStatus::Disconnected);
        assert_ne!(ConnectionStatus::Connected, ConnectionStatus::Disconnected);
        assert_eq!(
            ConnectionStatus::Error("test".to_string()),
            ConnectionStatus::Error("test".to_string())
        );
    }

    #[test]
    fn test_task_to_charge_config() {
        let config = TaskConfig {
            task_type: TaskType::Charge,
            battery_chemistry: BatteryChemistry::LiIon,
            target_voltage: 4.2,
            target_current: 1.0,
            cutoff_voltage: None,
            capacity_limit: Some(2000),
            time_limit: None,
            temperature_limit: None,
            charge_current_ma: 500,
            discharge_current_ma: 300,
        };
        let charge_config = task_to_charge_config(0, &config);
        assert_eq!(charge_config.charge_current_ma, 500);
        assert_eq!(charge_config.capacity_mah, 2000);
    }

    #[test]
    fn test_bt_state_to_slot_state() {
        assert_eq!(super::map_bt_state(&BtSlotState::Idle), SlotState::Idle);
        assert_eq!(super::map_bt_state(&BtSlotState::Charging), SlotState::Charging);
        assert_eq!(super::map_bt_state(&BtSlotState::Discharging), SlotState::Discharging);
        assert_eq!(super::map_bt_state(&BtSlotState::Completed), SlotState::Completed);
        assert_eq!(super::map_bt_state(&BtSlotState::Paused), SlotState::Paused);
        assert_eq!(
            super::map_bt_state(&BtSlotState::Error),
            SlotState::Error("Error".to_string())
        );
    }

    #[test]
    fn a_hole_in_the_programs_live_data_is_found() {
        let start = chrono::Utc::now() - chrono::Duration::seconds(600);
        let point = |s: i64| MeasurementPoint {
            timestamp: start + chrono::Duration::seconds(s),
            ..MeasurementPoint::new(SlotId(0), 4.2, 500.0)
        };
        let end = start + chrono::Duration::seconds(60);
        let every_second: Vec<_> = (0..=60).map(point).collect();
        assert!(!super::has_hole(&every_second, start, end), "watched from the start");
        let late: Vec<_> = (30..=60).map(point).collect();
        assert!(super::has_hole(&late, start, end), "connected 30 s late");
        let gap: Vec<_> = (0..=20).chain(40..=60).map(point).collect();
        assert!(super::has_hole(&gap, start, end), "20 s without data");
        let short_gap: Vec<_> = (0..=20).chain(30..=60).map(point).collect();
        assert!(!super::has_hole(&short_gap, start, end), "10 s is not a hole");
        assert!(super::has_hole(&[], start, end));
    }

    fn running(slot_id: u8, elapsed_seconds: u32) -> SlotStatus {
        SlotStatus { elapsed_seconds, ..status(slot_id, 4198, 500) }
    }

    #[tokio::test]
    async fn history_is_requested_once_per_connection_and_program() {
        let mut app = create_test_app();
        app.session = Some(test_session());
        app.connection_status = ConnectionStatus::Connected;
        // Connected 10 minutes into a program: the start is a hole.
        let _ = app.update(AppMessage::StatusReceived(running(4, 600)));
        let first = app.history_checked[3];
        assert!(first.is_some());
        let start = app.program_starts[3].unwrap();
        assert!(((chrono::Utc::now() - start).num_seconds() - 600).abs() <= 1);
        // Further statuses of the same program do not ask again.
        let _ = app.update(AppMessage::StatusReceived(running(4, 601)));
        assert_eq!(app.history_checked[3], first);
        assert_eq!(app.program_starts[3], Some(start));
        // A new program does.
        let _ = app.update(AppMessage::StatusReceived(running(4, 3)));
        assert_ne!(app.program_starts[3], Some(start));
    }

    #[test]
    fn history_that_does_not_fit_the_program_is_not_used() {
        let mut app = create_test_app();
        let request = super::HistoryRequest {
            slot: 3,
            program_start: chrono::Utc::now() - chrono::Duration::seconds(892),
            elapsed_s: 892,
        };
        let history = |samples: usize| skyrc_mc5000::VoltageHistory {
            slot_id: 4,
            interval_s: 4,
            samples_mv: vec![4198; samples],
        };
        let _ = app.update(AppMessage::HistoryReceived(request, Ok(history(50))));
        assert_eq!(app.data_logger.len(), 0, "200 s of history for an 892 s program");
        let _ = app.update(AppMessage::HistoryReceived(request, Ok(history(223))));
        assert_eq!(app.data_logger.len(), 223);
        assert_eq!(app.history_samples[3], 223);
        let _ = app.update(AppMessage::HistoryReceived(request, Err("timeout".into())));
        assert_eq!(app.data_logger.len(), 223);
    }

    #[test]
    fn graph_window_accepts_only_sensible_values() {
        let mut app = create_test_app();
        let before = app.settings.graph_window_s;
        let _ = app.update(AppMessage::GraphWindowInput("abc".into()));
        let _ = app.update(AppMessage::GraphWindowApply);
        assert_eq!(app.settings.graph_window_s, before);
        assert_eq!(app.graph_window_input, before.to_string());
        let _ = app.update(AppMessage::GraphClear);
        assert!(app.graph_epoch.is_some());
        let _ = app.update(AppMessage::ClearData);
        assert!(app.graph_epoch.is_none());
    }
}
