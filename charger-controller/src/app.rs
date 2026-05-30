use iced::{Element, Subscription, Task, Theme};

use std::time::{Duration, Instant};
use std::hash::{Hash, Hasher};
use futures::stream::StreamExt;

use mc5000_protocol::{
    Device, DeviceManager, DeviceError,
    MC5000SlotState as BtSlotState, ChargeConfig, OperationMode, 
    BatteryChemistry as BtChemistry, MC5000Protocol, StartStopAction
};
use crate::slot::{Slot, SlotId, SlotState, TaskConfig, BatteryChemistry, TaskType};
use crate::data::{DataLogger, MeasurementPoint};
use crate::export::CsvExporter;
use crate::ui;
use crate::config_dialog::{ChargeMode};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AppMessage {
    Tick,
    NotificationReceived(Vec<u8>),
    DeviceSelected(String),
    BluetoothScanComplete(Result<Vec<String>, String>),
    /// Result of a quick targeted scan for a previously known device.
    /// `Ok(Some(display_name))` → found, ready to connect.
    /// `Ok(None)` → not found, fall back to full scan.
    QuickScanResult(Result<Option<String>, String>),
    ConnectDevice,
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
    // CLI debug mode
    CliCommand(String),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SlotMessage {
    UpdateState(SlotState),
    UpdateMeasurement(f32, f32), // voltage, current
}

pub struct ChargerApp {
    device_manager: DeviceManager,
    connected_device: Option<Device>,
    slots: [Slot; 4],
    data_logger: DataLogger,
    csv_exporter: CsvExporter,
    last_update: Instant,
    update_counter: u64,  // Force UI redraw
    selected_device: Option<String>,
    connection_status: ConnectionStatus,
    scanning: bool,  // Track if Bluetooth scan is in progress
    rt: tokio::runtime::Runtime,
    slot_configs: [Option<TaskConfig>; 4],
    configuring_slot: Option<SlotId>,
    selected_slot: Option<usize>,  // Track selected slot for graph display
    config_dialog_state: Option<crate::ui::components::config_dialog::ConfigDialogState>,
    pending_slot: Option<SlotId>,
    saved_slot_configs: [Option<crate::ui::components::config_dialog::ConfigDialogState>; 4],  // Persist config per slot
    slot_config_store: crate::slot_persist::SlotConfigStore,  // Disk-persisted per-slot configs
    auto_charge_pending: Vec<SlotId>,  // Slots waiting for resistance measurement in auto-charge mode
    show_detailed_stats: bool,  // Toggle for detailed per-slot sample stream
    settings: crate::settings::Settings,
    show_settings: bool,
    profile_store: crate::profiles::ProfileStore,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

/// Wrapper for BLE peripheral that implements Hash for subscription identity
struct NotificationData {
    peripheral: btleplug::platform::Peripheral,
    verbose: bool,
}

impl Hash for NotificationData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "mc5000_notifications".hash(state);
    }
}

fn notification_stream(
    data: &NotificationData,
) -> impl futures::stream::Stream<Item = AppMessage> {
    let peripheral = data.peripheral.clone();
    let verbose = data.verbose;

    futures::stream::unfold(
        (peripheral, None),
        move |(peripheral, mut stream_opt)| async move {
            use btleplug::api::Peripheral as _;

            if stream_opt.is_none() {
                if verbose {
                    use std::io::Write;
                    println!("[SUBSCRIPTION VERBOSE] Creating new notifications stream...");
                    let _ = std::io::stdout().flush();
                }
                match peripheral.notifications().await {
                    Ok(stream) => {
                        if verbose {
                            use std::io::Write;
                            println!("[SUBSCRIPTION VERBOSE] ✓ Stream created");
                            let _ = std::io::stdout().flush();
                        }
                        stream_opt = Some(stream);
                    }
                    Err(e) => {
                        if verbose {
                            use std::io::Write;
                            println!("[SUBSCRIPTION VERBOSE] ✗ Error creating stream: {}", e);
                            let _ = std::io::stdout().flush();
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        return Some((AppMessage::Tick, (peripheral, None)));
                    }
                }
            }

            if let Some(stream) = stream_opt.as_mut() {
                match tokio::time::timeout(Duration::from_millis(50), stream.next()).await {
                    Ok(Some(notif)) => {
                        if verbose {
                            use std::io::Write;
                            println!(
                                "[SUBSCRIPTION VERBOSE] ✓ Got notification: {} bytes",
                                notif.value.len()
                            );
                            let _ = std::io::stdout().flush();
                        }
                        return Some((
                            AppMessage::NotificationReceived(notif.value),
                            (peripheral, stream_opt),
                        ));
                    }
                    Ok(None) => {
                        if verbose {
                            use std::io::Write;
                            println!("[SUBSCRIPTION VERBOSE] Stream ended, will recreate");
                            let _ = std::io::stdout().flush();
                        }
                        return Some((AppMessage::Tick, (peripheral, None)));
                    }
                    Err(_) => {
                        return Some((AppMessage::Tick, (peripheral, stream_opt)));
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
            Some((AppMessage::Tick, (peripheral, stream_opt)))
        },
    )
}

/// Subscription that reads stdin lines and emits CliCommand messages (debug mode only)
#[cfg(debug_assertions)]
fn cli_debug_stream() -> impl futures::stream::Stream<Item = AppMessage> {
    futures::stream::unfold((), |()| async {
        let line = tokio::task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok()?;
            Some(buf.trim().to_string())
        })
        .await
        .ok()
        .flatten();

        match line {
            Some(cmd) if !cmd.is_empty() => Some((AppMessage::CliCommand(cmd), ())),
            _ => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Some((AppMessage::Tick, ()))
            }
        }
    })
}

impl ChargerApp {
    pub fn new() -> (Self, Task<AppMessage>) {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        if verbose {
            println!("[APP VERBOSE] Initializing ChargerApp...");
        }
        
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
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

        let app = ChargerApp {
            device_manager: DeviceManager::new(),
            connected_device: None,
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
            rt,
            slot_configs: [None, None, None, None],
            configuring_slot: None,
            selected_slot: Some(0),  // Default to first slot selected
            config_dialog_state: None,
            pending_slot: None,
            saved_slot_configs,
            auto_charge_pending: Vec::new(),
            show_detailed_stats: false,  // Default: hide detailed per-slot stream
            settings,
            show_settings: false,
            profile_store,
            slot_config_store,
        };

        if verbose {
            println!("[APP VERBOSE] Starting initial Bluetooth device scan (async)...");
            println!("[APP VERBOSE] ChargerApp initialization complete\n");
        }

        // If we have a saved device and "save_last_device" is on, do a quick targeted
        // scan instead of a full 5-second blind scan.
        let startup_task = if app.settings.save_last_device {
            if let Some(ref saved) = app.settings.last_device_id {
                // Extract the peripheral ID from the stored display name
                // Format: "MC5000 BT: <name> (ID:<peripheral_id>)"
                let peripheral_id: Option<String> = saved.find("ID:").map(|start| {
                    let id_start = start + 3;
                    saved[id_start..]
                        .find(')')
                        .map(|end| saved[id_start..id_start + end].to_string())
                        .unwrap_or_else(|| saved[id_start..].to_string())
                });

                if let Some(pid) = peripheral_id {
                    if verbose {
                        println!("[APP VERBOSE] Quick scan for saved peripheral: {}", pid);
                    }
                    Task::perform(
                        async move {
                            let mut dm = mc5000_protocol::DeviceManager::new();
                            dm.quick_scan_for_device(&pid, 5).await
                                .map_err(|e| e.to_string())
                        },
                        AppMessage::QuickScanResult,
                    )
                } else {
                    // Saved name has no parseable ID — fall back to full scan
                    Self::full_scan_task()
                }
            } else {
                Self::full_scan_task()
            }
        } else {
            Self::full_scan_task()
        };

        (app, startup_task)
    }

    fn full_scan_task() -> Task<AppMessage> {
        Task::perform(
            async {
                let mut device_manager = mc5000_protocol::DeviceManager::new();
                match device_manager.scan_bluetooth_devices().await {
                    Ok(_) => {
                        let devices = device_manager.get_available_devices().to_vec();
                        Ok(devices)
                    }
                    Err(e) => Err(e.to_string()),
                }
            },
            AppMessage::BluetoothScanComplete,
        )
    }

    pub fn title(&self) -> String {
        format!("{} v{}", crate::i18n::t!("app.title"), env!("CARGO_PKG_VERSION"))
    }

    pub fn update(&mut self, message: AppMessage) -> Task<AppMessage> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        match message {
            AppMessage::NotificationReceived(data) => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                // Parse notification data (23-byte status response)
                if data.len() >= 23 && data[0] == 0x0F && data[1] == 0x15 && data[2] == 0x91 {
                    let channel_mask = data[3];
                    
                    // Determine slot index from channel mask
                    let slot_idx = match channel_mask {
                        0x01 => 0,
                        0x02 => 1,
                        0x04 => 2,
                        0x08 => 3,
                        _ => return Task::none(),
                    };
                    
                    if verbose {
                        use std::io::Write;
                        println!("[GUI VERBOSE] Notification for slot {} (mask 0x{:02X})", slot_idx + 1, channel_mask);
                        let _ = std::io::stdout().flush();
                    }
                    
                    // Parse status using the MC5000SlotStatus parser
                    if let Ok(status) = mc5000_protocol::MC5000SlotStatus::parse_from_response(&data) {
                        let voltage = status.voltage();
                        let current = status.current();
                        
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
                            let new_state = map_bt_state(&status.state);
                            // Don't downgrade from Charging to Idle within 10s of starting.
                            // The MC5000 reports 0x00 (Idle) during NiMH charge startup
                            // until current actually starts flowing.
                            let should_protect = slot.is_active()
                                && new_state == SlotState::Idle
                                && slot.start_time.map(|t| t.elapsed().as_secs() < 10).unwrap_or(false);
                            if !should_protect {
                                slot.set_state(new_state);
                            }
                        }
                        
                        // Check if this slot is in auto-charge mode and has valid resistance
                        let slot_id = SlotId(slot_idx);
                        let elapsed_enough = self.slots.get(slot_idx)
                            .and_then(|s| s.start_time)
                            .map(|t| t.elapsed().as_secs() >= 5)
                            .unwrap_or(false);
                        let slot_is_active = self.slots.get(slot_idx).map(|s| s.is_active()).unwrap_or(false);
                        
                        if self.auto_charge_pending.contains(&slot_id) && 
                           status.resistance_milliohm > 10 && // Valid resistance measurement
                           elapsed_enough && // Wait for device to stabilize
                           slot_is_active {
                            
                            // Calculate optimal charge current based on resistance
                            let resistance_ohm = status.resistance_milliohm as f32 / 1000.0;
                            let capacity_mah = 3000.0; // Default capacity for auto-charge
                            let base_current_1c = capacity_mah; // 1C current in mA
                            
                            // Calculate safe current: limit voltage drop to ~0.3V max
                            let max_current_from_resistance = (300.0 / resistance_ohm) as u16; // in mA
                            
                            // Determine chemistry-based max current
                            let chemistry_max: u16 = self.slots.get(slot_idx)
                                .and_then(|s| s.current_task.as_ref())
                                .map(|task| match task.battery_chemistry {
                                    BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop => 500,
                                    _ => 3000, // Li-Ion variants can go higher
                                })
                                .unwrap_or(500);
                            
                            // Use conservative rate: 0.5C or resistance-limited or chemistry-limited, whichever is lowest
                            let target_current = ((base_current_1c * 0.5) as u16)
                                .min(max_current_from_resistance)
                                .min(chemistry_max)
                                .max(300);
                            
                            if verbose {
                                println!("[AUTO-CHARGE] Slot {}: R={}mΩ, adjusting current to {}mA (max from R: {}mA)",
                                    slot_idx + 1, status.resistance_milliohm, target_current, max_current_from_resistance);
                            }
                            
                            // Extract task data needed for config
                            let task_data = self.slots.get(slot_idx)
                                .and_then(|s| s.current_task.as_ref())
                                .map(|task| (task.battery_chemistry, task.target_voltage, task.cutoff_voltage));
                            
                            if let Some((chemistry, target_voltage, cutoff_opt)) = task_data {
                                let cutoff_voltage = cutoff_opt.unwrap_or(chemistry.cutoff_voltage());
                                
                                let bt_chem = match chemistry {
                                    BatteryChemistry::LiIon => BtChemistry::LiIon,
                                    BatteryChemistry::LiIonHV => BtChemistry::LiIonHV,
                                    BatteryChemistry::LiFePO4 => BtChemistry::LiFePO4,
                                    BatteryChemistry::NiMH => BtChemistry::NiMH,
                                    BatteryChemistry::NiCd => BtChemistry::NiCd,
                                    BatteryChemistry::Eneloop => BtChemistry::Eneloop,
                                    BatteryChemistry::NiZn => BtChemistry::NiZn,
                                    BatteryChemistry::RAM => BtChemistry::RAM,
                                    BatteryChemistry::LTO => BtChemistry::LTO,
                                    BatteryChemistry::NaIon => BtChemistry::NaIon,
                                };
                                let config = ChargeConfig {
                                    channel_bitmask: 1 << slot_idx,
                                    mode: OperationMode::Charge,
                                    chemistry: bt_chem,
                                    charge_current_ma: target_current,
                                    discharge_current_ma: 0,
                                    capacity_mah: capacity_mah as u16,
                                    target_voltage_mv: (target_voltage * 1000.0) as u16,
                                    cutoff_voltage_mv: (cutoff_voltage * 1000.0) as u16,
                                    charge_cutoff_current_ma: 100,
                                    discharge_cutoff_current_ma: 100,
                                    trickle_charge_ma: if matches!(chemistry, BatteryChemistry::NiCd) { 50 } else { 0 },
                                    keep_voltage_mv: 0,
                                    delta_peak_mv: if matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop) { 6 } else { 0 },
                                    cutoff_timer_min: 0,
                                    max_time_min: 300,
                                    cycle_direction: 0x00,
                                    charge_resting_min: 10,
                                    discharge_resting_min: 10,
                                    cycle_count: 1,
                                };
                                
                                // Compute combined active mask
                                let mut active_mask: u8 = 0;
                                for i in 0..4 {
                                    if self.slots[i].is_active() {
                                        active_mask |= 1 << i;
                                    }
                                }
                                
                                if let Some(ref mut device) = self.connected_device {
                                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                                        let cmd = mc5000_protocol::MC5000Protocol::build_charge_config_command(&config);
                                        
                                        match self.rt.block_on(proto.send_command(&cmd)) {
                                            Ok(_) => {
                                                let start_cmd = mc5000_protocol::MC5000Protocol::build_start_stop_command(
                                                    mc5000_protocol::StartStopAction::ChannelMask(active_mask)
                                                );
                                                let _ = self.rt.block_on(proto.send_command(&start_cmd));
                                                
                                                if verbose {
                                                    println!("[AUTO-CHARGE] Slot {}: Updated to {}mA based on {}mΩ resistance",
                                                        slot_idx + 1, target_current, status.resistance_milliohm);
                                                }
                                                
                                                // Update task current
                                                if let Some(slot) = self.slots.get_mut(slot_idx) {
                                                    if let Some(ref mut task) = slot.current_task {
                                                        task.charge_current_ma = target_current;
                                                        task.target_current = target_current as f32 / 1000.0;
                                                    }
                                                }
                                                
                                                // Remove from pending list
                                                self.auto_charge_pending.retain(|&id| id != slot_id);
                                            }
                                            Err(e) => {
                                                if verbose {
                                                    println!("[AUTO-CHARGE] ✗ Slot {}: Failed to update current: {}",
                                                        slot_idx + 1, e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else if self.auto_charge_pending.contains(&slot_id) && !slot_is_active && elapsed_enough {
                            // Device didn't start charging — remove from pending
                            if verbose {
                                println!("[AUTO-CHARGE] Slot {}: Device did not start, removing from pending", slot_idx + 1);
                            }
                            self.auto_charge_pending.retain(|&id| id != slot_id);
                        }
                        
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
                    }
                }
                Task::none()
            }
            
            AppMessage::Tick => {
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        if verbose {
                            use std::io::Write;
                            println!("[GUI VERBOSE] Polling all 4 slots for status updates...");
                            let _ = std::io::stdout().flush();
                        }
                        
                        // Poll all four slots
                        for (idx, channel) in [1u8,2,3,4].iter().enumerate() {
                            if verbose {
                                use std::io::Write;
                                println!("[GUI VERBOSE] Requesting status for slot {} (channel: {})...", idx + 1, channel);
                                let _ = std::io::stdout().flush();
                            }
                            
                            // Just send the request - response will come via notification
                            let _res = self.rt.block_on(proto.request_slot_status(*channel));
                        }
                    }
                }
                Task::none()
            }
            
            AppMessage::DeviceSelected(device_name) => {
                self.selected_device = Some(device_name);
                Task::none()
            }
            
            AppMessage::QuickScanResult(result) => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                match result {
                    Ok(Some(display_name)) => {
                        // Quick scan succeeded — populate our device manager with the found
                        // peripheral (it is already in the temporary DeviceManager used in the
                        // task, but we need it in self.device_manager for connect() to work).
                        // Re-use the quick scan result: do a short re-scan on self.device_manager.
                        // Extract peripheral ID from display_name and quick-scan again.
                        let peripheral_id: Option<String> = display_name.find("ID:").map(|start| {
                            let id_start = start + 3;
                            display_name[id_start..]
                                .find(')')
                                .map(|end| display_name[id_start..id_start + end].to_string())
                                .unwrap_or_else(|| display_name[id_start..].to_string())
                        });

                        if let Some(pid) = peripheral_id {
                            let _ = self.rt.block_on(self.device_manager.quick_scan_for_device(&pid, 5));
                        }

                        if verbose {
                            println!("[APP VERBOSE] Quick scan found device: {}", display_name);
                        }
                        self.scanning = false;
                        self.selected_device = Some(display_name);
                        self.update(AppMessage::ConnectDevice)
                    }
                    Ok(None) => {
                        // Device not nearby — fall back to a full scan
                        if verbose {
                            println!("[APP VERBOSE] Quick scan: device not found, falling back to full scan");
                        }
                        // scanning stays true; fire off full scan
                        Task::perform(
                            async {
                                let mut device_manager = mc5000_protocol::DeviceManager::new();
                                match device_manager.scan_bluetooth_devices().await {
                                    Ok(_) => Ok(device_manager.get_available_devices().to_vec()),
                                    Err(e) => Err(e.to_string()),
                                }
                            },
                            AppMessage::BluetoothScanComplete,
                        )
                    }
                    Err(e) => {
                        if verbose {
                            println!("[APP VERBOSE] Quick scan error: {}", e);
                        }
                        // Also fall back to full scan on error
                        Task::perform(
                            async {
                                let mut device_manager = mc5000_protocol::DeviceManager::new();
                                match device_manager.scan_bluetooth_devices().await {
                                    Ok(_) => Ok(device_manager.get_available_devices().to_vec()),
                                    Err(e) => Err(e.to_string()),
                                }
                            },
                            AppMessage::BluetoothScanComplete,
                        )
                    }
                }
            }

            AppMessage::BluetoothScanComplete(result) => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                self.scanning = false;  // Scan complete, enable UI
                
                match result {
                    Ok(devices) => {
                        if verbose {
                            println!("[APP VERBOSE] ✓ Bluetooth scan completed");
                            println!("[APP VERBOSE] Found {} devices", devices.len());
                        }
                        
                        // Re-scan to populate device_manager with live peripheral handles
                        let _ = self.rt.block_on(self.device_manager.scan_bluetooth_devices());
                        
                        // Select the first MC5000 device (no auto-connect on full scan —
                        // the user explicitly gets to choose or this is the fallback path)
                        let device_to_select = self.device_manager.get_available_devices()
                            .iter()
                            .find(|d| d.starts_with("MC5000"))
                            .cloned();
                        
                        if let Some(device) = device_to_select {
                            if verbose {
                                println!("[APP VERBOSE] Auto-selecting device: {}", device);
                            }
                            self.selected_device = Some(device);
                        }
                    }
                    Err(e) => {
                        if verbose {
                            println!("[APP VERBOSE] ✗ Bluetooth scan failed: {}", e);
                        }
                    }
                }
                Task::none()
            }
            
            AppMessage::ConnectDevice => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if let Some(ref device_name) = self.selected_device {
                    if verbose {
                        println!("[GUI VERBOSE] Attempting to connect to device: {}", device_name);
                    }
                    
                    // Check if device is still in the available list
                    let device_available = self.device_manager.get_available_devices()
                        .iter()
                        .any(|d| d == device_name);
                    
                    if !device_available {
                        if verbose {
                            println!("[GUI VERBOSE] Device not in list, re-scanning...");
                        }
                        // Device not found — trigger a re-scan which will repopulate the list
                        self.scanning = true;
                        return Task::perform(
                            async {
                                let mut device_manager = DeviceManager::new();
                                match device_manager.scan_bluetooth_devices().await {
                                    Ok(_) => {
                                        let devices = device_manager.get_available_devices().to_vec();
                                        Ok(devices)
                                    }
                                    Err(e) => Err(e.to_string())
                                }
                            },
                            AppMessage::BluetoothScanComplete,
                        );
                    }
                    
                    self.connection_status = ConnectionStatus::Connecting;
                    let device_name_clone = device_name.clone();
                    
                    // Do the connection synchronously but with timeout to avoid freezing
                    // TODO: Make this fully async to avoid blocking
                    let result = self.rt.block_on(async {
                        tokio::time::timeout(
                            Duration::from_secs(15),
                            self.device_manager.connect(device_name_clone)
                        ).await
                            .map_err(|_| DeviceError::ConnectionFailed("Connection timeout".to_string()))?
                    });
                    
                    match result {
                        Ok(device) => {
                            if verbose {
                                println!("[GUI VERBOSE] ✓ Successfully connected to: {}", device.name);
                                println!("[GUI VERBOSE] Device type: {:?}", device.device_type);
                            }
                            self.connection_status = ConnectionStatus::Connected;
                            // Persist last connected device if enabled
                            if self.settings.save_last_device {
                                self.settings.last_device_id = self.selected_device.clone();
                                let _ = crate::settings::save(&self.settings);
                            }
                            self.connected_device = Some(device);
                        }
                        Err(e) => {
                            if verbose {
                                println!("[GUI VERBOSE] ✗ Connection failed: {}", e);
                            }
                            self.connection_status = ConnectionStatus::Error(e.to_string());
                            self.connected_device = None;
                        }
                    }
                }
                Task::none()
            }

            AppMessage::RefreshDevices => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                if verbose {
                    println!("[GUI VERBOSE] Refreshing device list (scanning Bluetooth...)...");
                }
                
                self.scanning = true;  // Start scanning, disable UI
                
                // Start async scan
                Task::perform(
                    async {
                        let mut device_manager = DeviceManager::new();
                        match device_manager.scan_bluetooth_devices().await {
                            Ok(_) => {
                                let devices = device_manager.get_available_devices().to_vec();
                                Ok(devices)
                            }
                            Err(e) => Err(e.to_string())
                        }
                    },
                    AppMessage::BluetoothScanComplete
                )
            }
            
            AppMessage::DisconnectDevice => {
                self.connected_device = None;
                self.connection_status = ConnectionStatus::Disconnected;
                // Stop all active slots
                for slot in &mut self.slots {
                    slot.stop();
                }
                Task::none()
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
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                // Send configuration to device if connected via Bluetooth
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        if verbose {
                            println!("[START-SLOT] Starting task on slot {}", slot_id.0 + 1);
                        }

                        let charge_config = task_to_charge_config(slot_id.0, &config);
                        let cmd = MC5000Protocol::build_charge_config_command(&charge_config);
                        
                        if let Err(e) = self.rt.block_on(proto.send_command(&cmd)) {
                            log::error!("Failed to send charge config: {}", e);
                        }

                        // Start this slot with its channel bitmask
                        let start_cmd = MC5000Protocol::build_start_stop_command(
                            StartStopAction::ChannelMask(1 << slot_id.0)
                        );
                        if verbose {
                            println!("[START-SLOT] Slot {} start mask=0x{:02X}", slot_id.0 + 1, 1u8 << slot_id.0);
                        }
                        
                        if let Err(e) = self.rt.block_on(proto.send_command(&start_cmd)) {
                            log::error!("Failed to start charging: {}", e);
                        }
                    }
                }

                if let Some(slot) = self.slots.get_mut(slot_id.0) {
                    slot.start_task(config.clone());
                }
                self.slot_configs[slot_id.0] = Some(config);
                Task::none()
            }
            
            AppMessage::StopTask(slot_id) => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if verbose {
                    println!("[STOP-SLOT] Stopping slot {} only", slot_id.0 + 1);
                }
                
                // Collect slots that should REMAIN active (all active except the one being stopped)
                let remaining_active: Vec<(usize, Option<TaskConfig>)> = self.slots.iter()
                    .enumerate()
                    .filter(|(i, s)| *i != slot_id.0 && s.is_active())
                    .map(|(i, _)| (i, self.slot_configs[i].clone()))
                    .collect();
                
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        // Protocol (from btsnoop capture): FE query → StopAll → [FE → Config → Start] per remaining slot
                        // The FE query "selects" the slot context; device ignores 0x93 without it.
                        let slot_mask: u8 = 1 << slot_id.0;
                        let fe_cmd = MC5000Protocol::build_fe_query_command_for(slot_mask);
                        let _ = self.rt.block_on(proto.send_command(&fe_cmd));
                        self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));

                        let stop_cmd = MC5000Protocol::build_start_stop_command(
                            StartStopAction::StopAll
                        );
                        let _ = self.rt.block_on(proto.send_command(&stop_cmd));
                        
                        if verbose {
                            println!("[STOP-SLOT] Sent FE(0x{:02X}) + StopAll, will re-start {} remaining slots", slot_mask, remaining_active.len());
                        }
                        
                        // Re-start remaining active slots (protocol: FE → Config → Start for each)
                        if !remaining_active.is_empty() {
                            self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                            
                            for (idx, config_opt) in &remaining_active {
                                if let Some(config) = config_opt {
                                    let remaining_mask: u8 = 1 << *idx;
                                    
                                    // FE query for this slot (required before config/start)
                                    let fe_cmd = MC5000Protocol::build_fe_query_command_for(remaining_mask);
                                    let _ = self.rt.block_on(proto.send_command(&fe_cmd));
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));

                                    let charge_config = task_to_charge_config(*idx, config);
                                    let cmd = MC5000Protocol::build_charge_config_command(&charge_config);
                                    let _ = self.rt.block_on(proto.send_command(&cmd));
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                                    
                                    let start_cmd = MC5000Protocol::build_start_stop_command(
                                        StartStopAction::ChannelMask(remaining_mask)
                                    );
                                    let _ = self.rt.block_on(proto.send_command(&start_cmd));
                                    
                                    if verbose {
                                        println!("[STOP-SLOT] Re-started slot {} (FE+Config+Start mask 0x{:02X})", idx + 1, remaining_mask);
                                    }
                                    
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                                } else if verbose {
                                    println!("[STOP-SLOT] Slot {} was active but has no saved config, cannot re-start", idx + 1);
                                }
                            }
                        }
                    }
                }

                // Only mark the stopped slot as stopped in UI
                if let Some(slot) = self.slots.get_mut(slot_id.0) {
                    slot.stop();
                }
                self.slot_configs[slot_id.0] = None;
                Task::none()
            }
            
            AppMessage::StopAllSlots => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if verbose {
                    println!("[STOP-ALL] Stopping all slots");
                }
                
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        // Protocol: FE query required before StopAll (0x93 0x00)
                        // Use bitmask of all active slots, or general query if none tracked
                        let active_mask: u8 = self.slots.iter()
                            .enumerate()
                            .filter(|(_, s)| s.is_active())
                            .fold(0u8, |acc, (i, _)| acc | (1 << i));
                        let fe_mask = if active_mask != 0 { active_mask } else { 0x0F };
                        
                        let fe_cmd = MC5000Protocol::build_fe_query_command_for(fe_mask);
                        let _ = self.rt.block_on(proto.send_command(&fe_cmd));
                        self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));

                        let stop_cmd = MC5000Protocol::build_start_stop_command(
                            StartStopAction::StopAll
                        );
                        let _ = self.rt.block_on(proto.send_command(&stop_cmd));
                        
                        if verbose {
                            println!("[STOP-ALL] Sent FE(0x{:02X}) + StopAll", fe_mask);
                        }
                    }
                }

                for slot in &mut self.slots {
                    slot.stop();
                }
                Task::none()
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
                
                if let (Some(state), Some(slot_id)) = 
                    (&self.config_dialog_state, self.pending_slot) 
                {
                    let config = state.get_config();
                    let chemistry = state.chemistry;
                    let mode = state.mode;
                    
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
                    if let Some(ref mut device) = self.connected_device {
                        if let Some(proto) = device.bluetooth_protocol.as_mut() {
                            if verbose {
                                println!("[GUI VERBOSE] Starting charge from config dialog on slot {}", slot_id.0 + 1);
                                println!("[GUI VERBOSE]   Chemistry: {:?}", chemistry);
                                println!("[GUI VERBOSE]   Mode: {:?}", mode);
                                println!("[GUI VERBOSE]   Capacity: {}mAh", config.capacity_mah);
                                println!("[GUI VERBOSE]   Charge current: {}mA", config.charge_current_ma);
                                println!("[GUI VERBOSE]   Target voltage: {}mV", config.target_voltage_mv);
                            }
                            
                            // Convert mode
                            // NOTE: Break-In (0x05) causes device to abort after ~42s.
                            // The official app sends Discharge (0x02) with break-in parameters.
                            let bt_mode = match mode {
                                ChargeMode::Charge => OperationMode::Charge,
                                ChargeMode::Storage => OperationMode::Storage,
                                ChargeMode::Discharge => OperationMode::Discharge,
                                ChargeMode::Cycle => OperationMode::Cycle,
                                ChargeMode::Refresh => OperationMode::Refresh,
                                ChargeMode::BreakIn => OperationMode::Discharge,
                            };
                            
                            // Convert chemistry
                            let bt_chem = chemistry.to_bluetooth_chemistry();
                            
                            // Build charge config
                            let bt_config = ChargeConfig {
                                channel_bitmask: 1 << slot_id.0,
                                mode: bt_mode,
                                chemistry: bt_chem,
                                charge_current_ma: config.charge_current_ma,
                                discharge_current_ma: config.discharge_current_ma,
                                capacity_mah: config.capacity_mah,
                                target_voltage_mv: config.target_voltage_mv,
                                cutoff_voltage_mv: config.cutoff_voltage_mv,
                                charge_cutoff_current_ma: config.charge_cutoff_current_ma,
                                discharge_cutoff_current_ma: config.discharge_cutoff_current_ma,
                                trickle_charge_ma: config.trickle_charge_ma,
                                keep_voltage_mv: config.keep_voltage_mv.unwrap_or(0),
                                delta_peak_mv: config.delta_peak_mv,
                                cutoff_timer_min: config.cutoff_timer_min,
                                max_time_min: 300, // Default max time
                                cycle_direction: match config.cycle_mode.as_deref() {
                                    Some("D>C") => 0x01,
                                    Some("C>D>C") => 0x02,
                                    Some("D>C>D") => 0x03,
                                    _ => 0x00, // C>D or default
                                },
                                charge_resting_min: config.charge_resting_min,
                                discharge_resting_min: config.discharge_resting_min,
                                cycle_count: config.cycle_count as u8,
                            };
                            
                            let cmd = MC5000Protocol::build_charge_config_command(&bt_config);
                            if verbose {
                                println!("[GUI VERBOSE] Sending config command: {:02X?}", cmd);
                            }
                            
                            match self.rt.block_on(proto.send_command(&cmd)) {
                                Ok(_) => {
                                    if verbose {
                                        println!("[GUI VERBOSE] ✓ Config command sent successfully");
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to send charge config: {}", e);
                                }
                            }
                            
                            // Start with combined bitmask: this slot + all already-active slots
                            // Protocol requires: start command sets the COMPLETE active slot set
                            let mut active_mask: u8 = 1 << slot_id.0;
                            for (i, s) in self.slots.iter().enumerate() {
                                if i != slot_id.0 && s.is_active() {
                                    active_mask |= 1 << i;
                                }
                            }
                            let start_cmd = MC5000Protocol::build_start_stop_command(
                                StartStopAction::ChannelMask(active_mask)
                            );
                            if verbose {
                                println!("[GUI VERBOSE] Sending start command (mask 0x{:02X})", active_mask);
                            }
                            
                            match self.rt.block_on(proto.send_command(&start_cmd)) {
                                Ok(_) => {
                                    if verbose {
                                        println!("[GUI VERBOSE] ✓ Start command sent successfully");
                                    }
                                    // Update slot state
                                    if let Some(slot) = self.slots.get_mut(slot_id.0) {
                                        slot.state = SlotState::Charging;
                                    }
                                }
                                Err(e) => {
                                    if verbose {
                                        println!("[GUI VERBOSE] ✗ Start command failed: {}", e);
                                    }
                                    log::error!("Failed to send start command: {}", e);
                                }
                            }
                        }
                    }
                } else {
                    self.config_dialog_state = None;
                    self.pending_slot = None;
                }
                
                Task::none()
            }
            
            AppMessage::SimpleAutoCharge => {
                // Simple auto-charge: detect Li-Ion/NiMH and charge at 500mA
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if verbose {
                    println!("[SIMPLE-AUTO] Starting simple auto-charge for all idle slots at 500mA...");
                }

                // Collect idle slot indices first (can't borrow self.slots mutably in loop with self.connected_device)
                let idle_slots: Vec<(usize, f32)> = self.slots.iter()
                    .enumerate()
                    .filter(|(_, s)| s.is_idle() && s.current_voltage > 0.1)
                    .map(|(i, s)| (i, s.current_voltage))
                    .collect();
                
                if verbose {
                    println!("[SIMPLE-AUTO] Found {} idle slots to start", idle_slots.len());
                }
                
                // For each idle slot: send config, then start for that single slot
                for (slot_idx, voltage) in &idle_slots {
                    let slot_id = self.slots[*slot_idx].id;
                    let chemistry = self.slots[*slot_idx].estimate_chemistry_from_voltage();
                    
                    if verbose {
                        println!("[SIMPLE-AUTO] Slot {}: Detected chemistry: {:?} ({:.3}V)", 
                            slot_id.0 + 1, chemistry, voltage);
                    }
                    
                    let charge_current_ma = 500;
                    let target_voltage = chemistry.target_voltage();
                    let cutoff_voltage = chemistry.cutoff_voltage();
                    
                    let bt_chem = match chemistry {
                        BatteryChemistry::LiIon => BtChemistry::LiIon,
                        BatteryChemistry::LiIonHV => BtChemistry::LiIonHV,
                        BatteryChemistry::LiFePO4 => BtChemistry::LiFePO4,
                        BatteryChemistry::NiMH => BtChemistry::NiMH,
                        BatteryChemistry::NiCd => BtChemistry::NiCd,
                        BatteryChemistry::Eneloop => BtChemistry::Eneloop,
                        BatteryChemistry::NiZn => BtChemistry::NiZn,
                        BatteryChemistry::RAM => BtChemistry::RAM,
                        BatteryChemistry::LTO => BtChemistry::LTO,
                        BatteryChemistry::NaIon => BtChemistry::NaIon,
                    };
                    
                    let is_nickel = matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop);
                    
                    let config = ChargeConfig {
                        channel_bitmask: 1 << slot_id.0,
                        mode: OperationMode::Charge,
                        chemistry: bt_chem,
                        charge_current_ma,
                        discharge_current_ma: 0,
                        capacity_mah: 3000,
                        target_voltage_mv: (target_voltage * 1000.0) as u16,
                        cutoff_voltage_mv: (cutoff_voltage * 1000.0) as u16,
                        charge_cutoff_current_ma: 100,
                        discharge_cutoff_current_ma: 100,
                        trickle_charge_ma: if is_nickel { 50 } else { 0 },
                        keep_voltage_mv: if is_nickel { 1300 } else { 0 },
                        delta_peak_mv: if is_nickel { 6 } else { 0 },
                        cutoff_timer_min: if is_nickel { 90 } else { 0 },
                        max_time_min: 300,
                        cycle_direction: 0x00,
                        charge_resting_min: 10,
                        discharge_resting_min: 10,
                        cycle_count: 1,
                    };
                    
                    let mut slot_started = false;
                    if let Some(ref mut device) = self.connected_device {
                        if let Some(proto) = device.bluetooth_protocol.as_mut() {
                            let cmd = mc5000_protocol::MC5000Protocol::build_charge_config_command(&config);
                            
                            match self.rt.block_on(proto.send_command(&cmd)) {
                                Ok(_) => {
                                    if verbose {
                                        println!("[SIMPLE-AUTO] Slot {}: Config sent ({}mA)", 
                                            slot_id.0 + 1, charge_current_ma);
                                    }
                                    
                                    // Add this slot's bit for single-slot start
                                    let slot_mask: u8 = 1 << slot_id.0;
                                    
                                    // Wait for device to process config before start
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                                    
                                    // Send start for this single slot only
                                    let start_cmd = mc5000_protocol::MC5000Protocol::build_start_stop_command(
                                        mc5000_protocol::StartStopAction::ChannelMask(slot_mask)
                                    );
                                    
                                    if verbose {
                                        println!("[SIMPLE-AUTO] Slot {}: Sending start (mask 0x{:02X})", 
                                            slot_id.0 + 1, slot_mask);
                                    }
                                    
                                    match self.rt.block_on(proto.send_command(&start_cmd)) {
                                        Ok(_) => {
                                            if verbose {
                                                println!("[SIMPLE-AUTO] Slot {}: ✓ Started", slot_id.0 + 1);
                                            }
                                            slot_started = true;
                                        }
                                        Err(e) => {
                                            if verbose {
                                                println!("[SIMPLE-AUTO] Slot {}: ✗ Start failed: {}", slot_id.0 + 1, e);
                                            }
                                        }
                                    }
                                    
                                    // Delay before next slot
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                                }
                                Err(e) => {
                                    if verbose {
                                        println!("[SIMPLE-AUTO] ✗ Slot {}: Failed to send config: {}", slot_id.0 + 1, e);
                                    }
                                }
                            }
                        }
                    }
                    
                    if slot_started {
                        let task = TaskConfig {
                            task_type: TaskType::Charge,
                            battery_chemistry: chemistry,
                            target_voltage,
                            target_current: charge_current_ma as f32 / 1000.0,
                            cutoff_voltage: Some(cutoff_voltage),
                            capacity_limit: None,
                            time_limit: None,
                            temperature_limit: None,
                            charge_current_ma,
                            discharge_current_ma: 0,
                        };
                        
                        self.slots[*slot_idx].current_task = Some(task.clone());
                        self.slots[*slot_idx].start_time = Some(Instant::now());
                        self.slots[*slot_idx].state = SlotState::Charging;
                        self.slot_configs[slot_id.0] = Some(task);
                    }
                }
                
                Task::none()
            }
            
            AppMessage::SmartChargeAll => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if verbose {
                    println!("[AUTO-CHARGE] Starting auto-charge for all idle slots...");
                }

                // Collect idle slot indices first
                let idle_slots: Vec<(usize, f32)> = self.slots.iter()
                    .enumerate()
                    .filter(|(_, s)| s.is_idle() && s.current_voltage > 0.1)
                    .map(|(i, s)| (i, s.current_voltage))
                    .collect();
                
                if verbose {
                    println!("[AUTO-CHARGE] Found {} idle slots to start", idle_slots.len());
                }
                
                // For each idle slot: send config, then start for that single slot
                for (slot_idx, voltage) in &idle_slots {
                    let slot_id = self.slots[*slot_idx].id;
                    let chemistry = self.slots[*slot_idx].estimate_chemistry_from_voltage();
                    
                    if verbose {
                        println!("[AUTO-CHARGE] Slot {}: Detected chemistry: {:?} ({:.3}V)", 
                            slot_id.0 + 1, chemistry, voltage);
                    }
                    
                    let initial_current_ma = 500;
                    let target_voltage = chemistry.target_voltage();
                    let cutoff_voltage = chemistry.cutoff_voltage();
                    
                    let bt_chem = match chemistry {
                        BatteryChemistry::LiIon => BtChemistry::LiIon,
                        BatteryChemistry::LiIonHV => BtChemistry::LiIonHV,
                        BatteryChemistry::LiFePO4 => BtChemistry::LiFePO4,
                        BatteryChemistry::NiMH => BtChemistry::NiMH,
                        BatteryChemistry::NiCd => BtChemistry::NiCd,
                        BatteryChemistry::Eneloop => BtChemistry::Eneloop,
                        BatteryChemistry::NiZn => BtChemistry::NiZn,
                        BatteryChemistry::RAM => BtChemistry::RAM,
                        BatteryChemistry::LTO => BtChemistry::LTO,
                        BatteryChemistry::NaIon => BtChemistry::NaIon,
                    };
                    
                    let is_nickel = matches!(chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop);
                    
                    let config = ChargeConfig {
                        channel_bitmask: 1 << slot_id.0,
                        mode: OperationMode::Charge,
                        chemistry: bt_chem,
                        charge_current_ma: initial_current_ma,
                        discharge_current_ma: 0,
                        capacity_mah: 3000,
                        target_voltage_mv: (target_voltage * 1000.0) as u16,
                        cutoff_voltage_mv: (cutoff_voltage * 1000.0) as u16,
                        charge_cutoff_current_ma: 100,
                        discharge_cutoff_current_ma: 100,
                        trickle_charge_ma: if is_nickel { 50 } else { 0 },
                        keep_voltage_mv: if is_nickel { 1300 } else { 0 },
                        delta_peak_mv: if is_nickel { 6 } else { 0 },
                        cutoff_timer_min: if is_nickel { 90 } else { 0 },
                        max_time_min: 300,
                        cycle_direction: 0x00,
                        charge_resting_min: 10,
                        discharge_resting_min: 10,
                        cycle_count: 1,
                    };
                    
                    let mut slot_started = false;
                    if let Some(ref mut device) = self.connected_device {
                        if let Some(proto) = device.bluetooth_protocol.as_mut() {
                            let cmd = mc5000_protocol::MC5000Protocol::build_charge_config_command(&config);
                            
                            match self.rt.block_on(proto.send_command(&cmd)) {
                                Ok(_) => {
                                    if verbose {
                                        println!("[AUTO-CHARGE] Slot {}: Config sent ({}mA)", 
                                            slot_id.0 + 1, initial_current_ma);
                                    }
                                    
                                    // Single-slot start
                                    let slot_mask: u8 = 1 << slot_id.0;
                                    
                                    // Wait for device to process config before start
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                                    
                                    // Send start for this single slot only
                                    let start_cmd = mc5000_protocol::MC5000Protocol::build_start_stop_command(
                                        mc5000_protocol::StartStopAction::ChannelMask(slot_mask)
                                    );
                                    
                                    if verbose {
                                        println!("[AUTO-CHARGE] Slot {}: Sending start (mask 0x{:02X})", 
                                            slot_id.0 + 1, slot_mask);
                                    }
                                    
                                    match self.rt.block_on(proto.send_command(&start_cmd)) {
                                        Ok(_) => {
                                            if verbose {
                                                println!("[AUTO-CHARGE] Slot {}: ✓ Started", slot_id.0 + 1);
                                            }
                                            slot_started = true;
                                        }
                                        Err(e) => {
                                            if verbose {
                                                println!("[AUTO-CHARGE] Slot {}: ✗ Start failed: {}", slot_id.0 + 1, e);
                                            }
                                        }
                                    }
                                    
                                    // Delay before next slot
                                    self.rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(200)));
                                }
                                Err(e) => {
                                    if verbose {
                                        println!("[AUTO-CHARGE] ✗ Slot {}: Failed to send config: {}", 
                                            slot_id.0 + 1, e);
                                    }
                                    log::error!("Auto-charge failed for slot {}: {}", slot_id.0 + 1, e);
                                }
                            }
                        }
                    }
                    
                    if slot_started {
                        let task = TaskConfig {
                            task_type: TaskType::Charge,
                            battery_chemistry: chemistry,
                            target_voltage,
                            target_current: initial_current_ma as f32 / 1000.0,
                            cutoff_voltage: Some(cutoff_voltage),
                            capacity_limit: Some(3000),
                            time_limit: None,
                            temperature_limit: None,
                            charge_current_ma: initial_current_ma,
                            discharge_current_ma: 0,
                        };
                        
                        self.slots[*slot_idx].current_task = Some(task.clone());
                        self.slots[*slot_idx].start_time = Some(Instant::now());
                        self.slots[*slot_idx].state = SlotState::Charging;
                        self.slot_configs[slot_id.0] = Some(task);
                        self.auto_charge_pending.push(slot_id);
                    }
                }
                
                Task::none()
            }
            
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
                    if let Err(e) = self.csv_exporter.export_to_file(&path, measurements) {
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
            
            AppMessage::ClearData => {
                self.data_logger.clear();
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

            AppMessage::CliCommand(cmd) => {
                use std::io::Write;
                println!("[CLI DEBUG] Received command: '{}'", cmd);
                let _ = std::io::stdout().flush();
                
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                match parts.first().copied() {
                    Some("auto") => {
                        println!("[CLI DEBUG] Triggering SimpleAutoCharge");
                        let _ = std::io::stdout().flush();
                        return self.update(AppMessage::SimpleAutoCharge);
                    }
                    Some("smart") => {
                        println!("[CLI DEBUG] Triggering SmartChargeAll");
                        let _ = std::io::stdout().flush();
                        return self.update(AppMessage::SmartChargeAll);
                    }
                    Some("stop") => {
                        println!("[CLI DEBUG] Triggering StopAllSlots");
                        let _ = std::io::stdout().flush();
                        return self.update(AppMessage::StopAllSlots);
                    }
                    Some("status") => {
                        println!("[CLI DEBUG] Slot status:");
                        for (i, slot) in self.slots.iter().enumerate() {
                            println!("  Slot {}: state={:?}, voltage={:.3}V, current={:.0}mA, resistance={}mΩ",
                                i + 1, slot.state, slot.current_voltage, slot.current_current, slot.resistance_milliohm);
                        }
                        let _ = std::io::stdout().flush();
                    }
                    Some("connect") => {
                        println!("[CLI DEBUG] Triggering ConnectDevice");
                        let _ = std::io::stdout().flush();
                        return self.update(AppMessage::ConnectDevice);
                    }
                    Some("disconnect") => {
                        println!("[CLI DEBUG] Triggering DisconnectDevice");
                        let _ = std::io::stdout().flush();
                        return self.update(AppMessage::DisconnectDevice);
                    }
                    Some("scan") => {
                        println!("[CLI DEBUG] Triggering RefreshDevices");
                        let _ = std::io::stdout().flush();
                        return self.update(AppMessage::RefreshDevices);
                    }
                    Some("help") => {
                        println!("[CLI DEBUG] Available commands:");
                        println!("  auto        - Trigger Auto (500mA) charge on idle slots");
                        println!("  smart       - Trigger SmartCharge on idle slots");
                        println!("  stop        - Stop all slots");
                        println!("  status      - Show current slot status");
                        println!("  connect     - Connect to selected device");
                        println!("  disconnect  - Disconnect from device");
                        println!("  scan        - Scan for devices");
                        println!("  help        - Show this help");
                        let _ = std::io::stdout().flush();
                    }
                    _ => {
                        println!("[CLI DEBUG] Unknown command: '{}'. Type 'help' for available commands.", cmd);
                        let _ = std::io::stdout().flush();
                    }
                }
                Task::none()
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
            &self.device_manager,
            &self.connected_device,
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
        )
    }

    pub fn subscription(&self) -> Subscription<AppMessage> {
        let tick = iced::time::every(Duration::from_millis(1000))
            .map(|_| AppMessage::Tick);
        
        // Add notification listener if connected
        let notifications = if let Some(ref device) = self.connected_device {
            if let Some(ref proto) = device.bluetooth_protocol {
                if let Some(peripheral) = proto.get_peripheral() {
                    let peripheral_clone = peripheral.clone();
                    let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                    
                    Subscription::run_with(
                        NotificationData {
                            peripheral: peripheral_clone,
                            verbose,
                        },
                        notification_stream,
                    )
                } else {
                    Subscription::none()
                }
            } else {
                Subscription::none()
            }
        } else {
            Subscription::none()
        };
        
        // CLI debug input subscription (only in verbose debug builds)
        #[cfg(debug_assertions)]
        let cli_input = if std::env::var("MC5000_VERBOSE").is_ok() {
            Subscription::run(cli_debug_stream)
        } else {
            Subscription::none()
        };
        #[cfg(not(debug_assertions))]
        let cli_input = Subscription::none();
        
        Subscription::batch([tick, notifications, cli_input])
    }

    pub fn theme(&self) -> Theme {
        match self.settings.theme {
            crate::settings::AppTheme::Light => Theme::Light,
            crate::settings::AppTheme::Dark => Theme::Dark,
        }
    }
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
        assert!(app.connected_device.is_none());
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
        app.update(AppMessage::ToggleDetailedStats);
        assert!(app.show_detailed_stats);
        app.update(AppMessage::ToggleDetailedStats);
        assert!(!app.show_detailed_stats);
    }

    #[test]
    fn test_slot_selected() {
        let mut app = create_test_app();
        assert_eq!(app.selected_slot, Some(0)); // default
        app.update(AppMessage::SlotSelected(2));
        assert_eq!(app.selected_slot, Some(2));
    }

    #[test]
    fn test_device_selected() {
        let mut app = create_test_app();
        app.update(AppMessage::DeviceSelected("test-device".to_string()));
        assert_eq!(app.selected_device, Some("test-device".to_string()));
    }

    #[test]
    fn test_bluetooth_scan_complete_success() {
        let mut app = create_test_app();
        app.scanning = true;
        let devices = vec!["device1".to_string(), "device2".to_string()];
        app.update(AppMessage::BluetoothScanComplete(Ok(devices)));
        assert!(!app.scanning);
    }

    #[test]
    fn test_bluetooth_scan_complete_error() {
        let mut app = create_test_app();
        assert!(app.scanning);
        app.update(AppMessage::BluetoothScanComplete(Err("scan failed".to_string())));
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
        app.update(AppMessage::ConfigDialogCancel);
        assert!(app.config_dialog_state.is_none());
    }

    #[test]
    fn test_notification_received_invalid_data() {
        let mut app = create_test_app();
        // Too short - should be ignored
        app.update(AppMessage::NotificationReceived(vec![0x01, 0x02]));
        // Wrong header - should be ignored
        app.update(AppMessage::NotificationReceived(vec![0xFF; 23]));
        // All slots should remain idle
        for slot in &app.slots {
            assert_eq!(slot.state, SlotState::Idle);
        }
    }

    #[test]
    fn test_notification_received_valid_status() {
        let mut app = create_test_app();
        // Construct a minimal valid notification:
        // Header: 0F 15 91, channel_mask 0x01 (slot 0)
        let mut data = vec![0x0F, 0x15, 0x91, 0x01];
        // Pad to 23 bytes
        data.resize(23, 0x00);
        // Set state byte (byte index varies by parser, but let's exercise the path)
        app.update(AppMessage::NotificationReceived(data));
        // The parser may or may not succeed with zeroed data, 
        // but the app should not panic
    }

    #[test]
    fn test_tick_does_not_panic_when_disconnected() {
        let mut app = create_test_app();
        // Tick should not panic when no device is connected
        app.update(AppMessage::Tick);
    }

    #[test]
    fn test_clear_data() {
        let mut app = create_test_app();
        app.update(AppMessage::ClearData);
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
}
