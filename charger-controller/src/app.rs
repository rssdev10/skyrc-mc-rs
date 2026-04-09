use iced::{executor, Application, Command, Element, Subscription, Theme};
use iced::window;
use std::time::{Duration, Instant};
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
use crate::config_dialog::{ChargeMode, ChargeConfig as DialogChargeConfig};

#[derive(Debug, Clone)]
pub enum AppMessage {
    Tick,
    NotificationReceived(Vec<u8>),
    DeviceSelected(String),
    BluetoothScanComplete(Result<Vec<String>, String>),
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
    SimpleAutoCharge,  // Simple auto: detect Li-Ion/NiMH, charge at 500mA
    SmartChargeAll,    // Smart charge: detect chemistry, measure resistance, optimize current
    StopAllSlots,      // Stop all active slots
    // Data statistics messages
    ToggleDetailedStats,  // Toggle detailed per-slot sample stream visibility
    ExportTimeAligned,    // Export time-aligned CSV (default)
    ExportAllSamples,     // Export all individual samples CSV
    FileSelectedTimeAligned(Option<std::path::PathBuf>),
    FileSelectedAllSamples(Option<std::path::PathBuf>),
}

#[derive(Debug, Clone)]
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
    auto_charge_pending: Vec<SlotId>,  // Slots waiting for resistance measurement in auto-charge mode
    show_detailed_stats: bool,  // Toggle for detailed per-slot sample stream
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl Application for ChargerApp {
    type Executor = executor::Default;
    type Message = AppMessage;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        if verbose {
            println!("[APP VERBOSE] Initializing ChargerApp...");
        }
        
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        
        if verbose {
            println!("[APP VERBOSE] Creating device manager and initializing slots...");
        }
        
        let mut app = ChargerApp {
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
            auto_charge_pending: Vec::new(),
            show_detailed_stats: false,  // Default: hide detailed per-slot stream
        };

        if verbose {
            println!("[APP VERBOSE] Starting initial Bluetooth device scan (async)...");
            println!("[APP VERBOSE] ChargerApp initialization complete\n");
        }

        // Start async Bluetooth scan
        let scan_command = Command::perform(
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
        );

        (app, scan_command)
    }

    fn title(&self) -> String {
        "MC5000 Charger Controller".to_string()
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
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
                        _ => return Command::none(),
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
                            slot.set_state(map_bt_state(&status.state));
                            
                            // Check if this slot is in auto-charge mode and has valid resistance
                            let slot_id = SlotId(slot_idx);
                            if self.auto_charge_pending.contains(&slot_id) && 
                               status.resistance_milliohm > 10 && // Valid resistance measurement
                               slot.is_active() {
                                
                                // Calculate optimal charge current based on resistance
                                // Target: ~0.5C to 1C rate, capped by resistance
                                // Higher resistance = lower safe current
                                let resistance_ohm = status.resistance_milliohm as f32 / 1000.0;
                                let capacity_mah = 3000.0; // Default capacity for auto-charge
                                let base_current_1c = capacity_mah; // 1C current in mA
                                
                                // Calculate safe current: limit voltage drop to ~0.3V max
                                // I = V_drop / R, using 0.3V as safe limit
                                let max_current_from_resistance = (300.0 / resistance_ohm) as u16; // in mA
                                
                                // Use conservative rate: 0.5C or resistance-limited, whichever is lower
                                let target_current = ((base_current_1c * 0.5) as u16).min(max_current_from_resistance).max(300);
                                
                                if verbose {
                                    println!("[AUTO-CHARGE] Slot {}: R={}mΩ, adjusting current to {}mA (max from R: {}mA)",
                                        slot_idx + 1, status.resistance_milliohm, target_current, max_current_from_resistance);
                                }
                                
                                // Update charge current
                                if let Some(ref task) = slot.current_task {
                                    let chemistry = task.battery_chemistry;
                                    let target_voltage = task.target_voltage;
                                    let cutoff_voltage = task.cutoff_voltage.unwrap_or(chemistry.cutoff_voltage());
                                    
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
                                    
                                    if let Some(ref mut device) = self.connected_device {
                                        if let Some(proto) = device.bluetooth_protocol.as_mut() {
                                            let cmd = mc5000_protocol::MC5000Protocol::build_charge_config_command(&config);
                                            
                                            match self.rt.block_on(proto.send_command(&cmd)) {
                                                Ok(_) => {
                                                    // Re-send start command so updated config takes effect
                                                    let start_cmd = mc5000_protocol::MC5000Protocol::build_start_stop_command(
                                                        mc5000_protocol::StartStopAction::ChannelMask(1 << slot_idx)
                                                    );
                                                    let _ = self.rt.block_on(proto.send_command(&start_cmd));
                                                    
                                                    if verbose {
                                                        println!("[AUTO-CHARGE] Slot {}: Updated to {}mA based on {}mΩ resistance",
                                                            slot_idx + 1, target_current, status.resistance_milliohm);
                                                    }
                                                    
                                                    // Update task current
                                                    if let Some(ref mut task) = slot.current_task {
                                                        task.charge_current_ma = target_current;
                                                        task.target_current = target_current as f32 / 1000.0;
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
                            }
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
                Command::none()
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
                Command::none()
            }
            
            AppMessage::DeviceSelected(device_name) => {
                self.selected_device = Some(device_name);
                Command::none()
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
                        
                        // Re-scan to populate device_manager properly
                        let _ = self.rt.block_on(self.device_manager.scan_bluetooth_devices());
                        
                        // Auto-select first MC5000 device
                        let mc5000_device = self.device_manager.get_available_devices()
                            .iter()
                            .find(|d| d.starts_with("MC5000"))
                            .cloned();
                        
                        if let Some(device) = mc5000_device {
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
                Command::none()
            }
            
            AppMessage::ConnectDevice => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if let Some(ref device_name) = self.selected_device {
                    if verbose {
                        println!("[GUI VERBOSE] Attempting to connect to device: {}", device_name);
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
                Command::none()
            }

            AppMessage::RefreshDevices => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                if verbose {
                    println!("[GUI VERBOSE] Refreshing device list (scanning Bluetooth...)...");
                }
                
                self.scanning = true;  // Start scanning, disable UI
                
                // Start async scan
                return Command::perform(
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
                );
            }
            
            AppMessage::DisconnectDevice => {
                self.connected_device = None;
                self.connection_status = ConnectionStatus::Disconnected;
                // Stop all active slots
                for slot in &mut self.slots {
                    slot.stop();
                }
                Command::none()
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
                Command::none()
            }
            
            AppMessage::StartTask(slot_id, config) => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                // Send configuration to device if connected via Bluetooth
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        if verbose {
                            println!("[GUI VERBOSE] Starting task on slot {}", slot_id.0 + 1);
                        }

                        // Init sequence required before config commands
                        if let Err(e) = self.rt.block_on(proto.send_init_sequence()) {
                            log::error!("Failed to send init sequence: {}", e);
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
                        eprintln!("[START] Slot {} start mask=0x{:02X}", slot_id.0 + 1, 1u8 << slot_id.0);
                        
                        if let Err(e) = self.rt.block_on(proto.send_command(&start_cmd)) {
                            log::error!("Failed to start charging: {}", e);
                        }
                    }
                }

                if let Some(slot) = self.slots.get_mut(slot_id.0) {
                    slot.start_task(config);
                }
                Command::none()
            }
            
            // StopTask and StopAllSlots both perform stop-all on the device.
            // The MC5000 protocol requires an init sequence (0x74, 0x65, 0xFE)
            // before stop-all commands are accepted. Per-slot stop is not supported
            // because the init sequence clears device config state.
            AppMessage::StopTask(_) | AppMessage::StopAllSlots => {
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        // Init sequence required for device to accept stop command
                        let _ = self.rt.block_on(proto.send_init_sequence());
                        let stop_cmd = MC5000Protocol::build_start_stop_command(
                            StartStopAction::StopAll
                        );
                        let _ = self.rt.block_on(proto.send_command(&stop_cmd));
                    }
                }

                // Stop all slots in UI (stop-all affects all)
                for slot in &mut self.slots {
                    slot.stop();
                }
                Command::none()
            }
            
            AppMessage::ConfigureSlot(slot_id) => {
                self.configuring_slot = Some(slot_id);
                // Initialize with default or existing config
                if self.slot_configs[slot_id.0].is_none() {
                    self.slot_configs[slot_id.0] = Some(TaskConfig::default());
                }
                Command::none()
            }

            AppMessage::UpdateSlotChemistry(slot_id, chemistry) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.battery_chemistry = chemistry;
                    config.target_voltage = chemistry.target_voltage();
                    config.cutoff_voltage = Some(chemistry.cutoff_voltage());
                }
                Command::none()
            }

            AppMessage::UpdateSlotTaskType(slot_id, task_type) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.task_type = task_type;
                }
                Command::none()
            }

            AppMessage::UpdateSlotCapacity(slot_id, capacity) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.capacity_limit = Some(capacity);
                }
                Command::none()
            }

            AppMessage::UpdateSlotChargeCurrent(slot_id, current_ma) => {
                if let Some(config) = self.slot_configs[slot_id.0].as_mut() {
                    config.charge_current_ma = current_ma;
                    config.discharge_current_ma = current_ma.min(2000);
                }
                Command::none()
            }

            AppMessage::CancelSlotConfig(slot_id) => {
                self.configuring_slot = None;
                self.slot_configs[slot_id.0] = None;
                Command::none()
            }
            
            AppMessage::ApplySlotConfig(slot_id) => {
                if let Some(config) = self.slot_configs[slot_id.0].clone() {
                    self.configuring_slot = None;
                    return self.update(AppMessage::StartTask(slot_id, config));
                }
                Command::none()
            }
            
            AppMessage::SlotSelected(slot_index) => {
                self.selected_slot = Some(slot_index);
                Command::none()
            }
            
            AppMessage::ShowConfigDialog(slot_index, voltage) => {
                self.pending_slot = Some(slot_index);
                self.config_dialog_state = Some(crate::ui::components::config_dialog::ConfigDialogState::new(
                    voltage,  // Voltage-based chemistry detection
                ));
                Command::none()
            }
            
            AppMessage::ConfigChemistryChanged(chemistry) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.update_chemistry(chemistry);
                }
                Command::none()
            }
            
            AppMessage::ConfigModeChanged(mode) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.update_mode(mode);
                }
                Command::none()
            }
            
            AppMessage::ConfigCapacityChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.capacity_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigChargeCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.charge_current_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigDischargeCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.discharge_current_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigTargetVoltageChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.target_voltage_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigCutoffVoltageChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.cutoff_voltage_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigStorageVoltageChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.storage_voltage_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigDeltaPeakChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.delta_peak_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigTrickleChargeChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.trickle_charge_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigCutoffTimerChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.cutoff_timer_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigChargeCutoffCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.charge_cutoff_current_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigDischargeCutoffCurrentChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.discharge_cutoff_current_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigChargeRestingChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.charge_resting_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigDischargeRestingChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.discharge_resting_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigCycleCountChanged(value) => {
                if let Some(state) = &mut self.config_dialog_state {
                    state.cycle_count_input = value;
                }
                Command::none()
            }
            
            AppMessage::ConfigDialogCancel => {
                self.config_dialog_state = None;
                self.pending_slot = None;
                Command::none()
            }
            
            AppMessage::ConfigDialogConfirm => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if let (Some(state), Some(slot_id)) = 
                    (&self.config_dialog_state, self.pending_slot) 
                {
                    let config = state.get_config();
                    let chemistry = state.chemistry;
                    let mode = state.mode;
                    
                    // Close dialog
                    self.config_dialog_state = None;
                    self.pending_slot = None;
                    
                    // Send configuration to device if connected via Bluetooth
                    if let Some(ref mut device) = self.connected_device {
                        if let Some(proto) = device.bluetooth_protocol.as_mut() {
                            // Init sequence required before config commands
                            if let Err(e) = self.rt.block_on(proto.send_init_sequence()) {
                                log::error!("Failed to send init sequence: {}", e);
                            }

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
                            
                            // Start with channel mask for this slot only
                            let start_cmd = MC5000Protocol::build_start_stop_command(
                                StartStopAction::ChannelMask(1 << slot_id.0)
                            );
                            if verbose {
                                println!("[GUI VERBOSE] Sending start command (mask 0x{:02X})", 1u8 << slot_id.0);
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
                
                Command::none()
            }
            
            AppMessage::SimpleAutoCharge => {
                // Simple auto-charge: detect Li-Ion/NiMH and charge at 500mA
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if verbose {
                    println!("[SIMPLE-AUTO] Starting simple auto-charge for all idle slots at 500mA...");
                }
                
                // Init sequence required before config commands
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        if let Err(e) = self.rt.block_on(proto.send_init_sequence()) {
                            log::error!("Failed to send init sequence: {}", e);
                        }
                    }
                }

                for slot in &mut self.slots {
                    if slot.is_idle() && slot.current_voltage > 0.1 {
                        let slot_id = slot.id;
                        let voltage = slot.current_voltage;
                        
                        // Auto-detect chemistry from voltage
                        let chemistry = slot.estimate_chemistry_from_voltage();
                        
                        if verbose {
                            println!("[SIMPLE-AUTO] Slot {}: Detected chemistry: {:?} ({:.3}V)", 
                                slot_id.0 + 1, chemistry, voltage);
                        }
                        
                        // Use 500mA charging current
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
                        
                        if let Some(ref mut device) = self.connected_device {
                            if let Some(proto) = device.bluetooth_protocol.as_mut() {
                                let cmd = mc5000_protocol::MC5000Protocol::build_charge_config_command(&config);
                                
                                match self.rt.block_on(proto.send_command(&cmd)) {
                                    Ok(_) => {
                                        if verbose {
                                            println!("[SIMPLE-AUTO] Slot {}: Config sent, starting charge at {}mA", 
                                                slot_id.0 + 1, charge_current_ma);
                                        }
                                        
                                        let start_cmd = mc5000_protocol::MC5000Protocol::build_start_stop_command(
                                            mc5000_protocol::StartStopAction::ChannelMask(1 << slot_id.0)
                                        );
                                        
                                        if let Err(e) = self.rt.block_on(proto.send_command(&start_cmd)) {
                                            if verbose {
                                                println!("[SIMPLE-AUTO] ✗ Slot {}: Failed to start: {}", slot_id.0 + 1, e);
                                            }
                                            continue;
                                        }
                                        
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
                                        
                                        self.slot_configs[slot_id.0] = Some(task);
                                        
                                        if verbose {
                                            println!("[SIMPLE-AUTO] ✓ Slot {}: Charging at {}mA", slot_id.0 + 1, charge_current_ma);
                                        }
                                    }
                                    Err(e) => {
                                        if verbose {
                                            println!("[SIMPLE-AUTO] ✗ Slot {}: Failed to send config: {}", slot_id.0 + 1, e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                
                Command::none()
            }
            
            AppMessage::SmartChargeAll => {
                let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                
                if verbose {
                    println!("[AUTO-CHARGE] Starting auto-charge for all idle slots...");
                }
                
                // Init sequence required before config commands
                if let Some(ref mut device) = self.connected_device {
                    if let Some(proto) = device.bluetooth_protocol.as_mut() {
                        if let Err(e) = self.rt.block_on(proto.send_init_sequence()) {
                            log::error!("Failed to send init sequence: {}", e);
                        }
                    }
                }

                // Find all idle slots with batteries present
                for slot in &mut self.slots {
                    if slot.is_idle() && slot.current_voltage > 0.1 {
                        let slot_id = slot.id;
                        let voltage = slot.current_voltage;
                        
                        // Auto-detect chemistry from voltage
                        let chemistry = slot.estimate_chemistry_from_voltage();
                        
                        if verbose {
                            println!("[AUTO-CHARGE] Slot {}: Detected chemistry: {:?} ({:.3}V)", 
                                slot_id.0 + 1, chemistry, voltage);
                        }
                        
                        // Start at 500mA — enough for device to measure resistance.
                        // Will be adjusted based on resistance reading later.
                        let initial_current_ma = 500;
                        
                        // Create charge configuration using chemistry defaults
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
                        
                        // Send charge command with minimal current
                        if let Some(ref mut device) = self.connected_device {
                            if let Some(proto) = device.bluetooth_protocol.as_mut() {
                                let cmd = mc5000_protocol::MC5000Protocol::build_charge_config_command(&config);
                                
                                match self.rt.block_on(proto.send_command(&cmd)) {
                                    Ok(_) => {
                                        if verbose {
                                            println!("[AUTO-CHARGE] Slot {}: Config sent, starting charge at {}mA", 
                                                slot_id.0 + 1, initial_current_ma);
                                        }
                                        
                                        // Send start command
                                        let start_cmd = mc5000_protocol::MC5000Protocol::build_start_stop_command(
                                            mc5000_protocol::StartStopAction::ChannelMask(1 << slot_id.0)
                                        );
                                        
                                        if let Err(e) = self.rt.block_on(proto.send_command(&start_cmd)) {
                                            if verbose {
                                                println!("[AUTO-CHARGE] ✗ Slot {}: Failed to start: {}", slot_id.0 + 1, e);
                                            }
                                            continue;
                                        }
                                        
                                        if verbose {
                                            println!("[AUTO-CHARGE] Slot {}: Started initial charge at {}mA", 
                                                slot_id.0 + 1, initial_current_ma);
                                        }
                                        
                                        // Create task config
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
                                        
                                        slot.current_task = Some(task);
                                        slot.start_time = Some(Instant::now());
                                        slot.state = SlotState::Charging;
                                        
                                        // Add to auto-charge pending list for resistance-based adjustment
                                        self.auto_charge_pending.push(slot_id);
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
                    }
                }
                
                Command::none()
            }
            
            AppMessage::ExportData => {
                // Default export: time-aligned format
                Command::perform(
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
                Command::perform(
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
                Command::perform(
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
                Command::none()
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
                Command::none()
            }
            
            AppMessage::FileSelectedAllSamples(path) => {
                if let Some(path) = path {
                    let measurements = self.data_logger.get_all_measurements();
                    if let Err(e) = self.csv_exporter.export_to_file(&path, &measurements) {
                        log::error!("Failed to export all samples: {}", e);
                        eprintln!("Failed to export all samples: {}", e);
                    } else {
                        println!("✓ Exported {} individual samples to {:?}", measurements.len(), path);
                    }
                }
                Command::none()
            }
            
            AppMessage::ToggleDetailedStats => {
                self.show_detailed_stats = !self.show_detailed_stats;
                Command::none()
            }
            
            AppMessage::ClearData => {
                self.data_logger.clear();
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<Self::Message> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        if verbose && self.update_counter % 10 == 0 {
            use std::io::Write;
            println!("[GUI VERBOSE] View rendered, update_counter: {}", self.update_counter);
            let _ = std::io::stdout().flush();
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
        )
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let tick = iced::time::every(Duration::from_millis(1000))
            .map(|_| AppMessage::Tick);
        
        // Add notification listener if connected
        let notifications = if let Some(ref device) = self.connected_device {
            if let Some(ref proto) = device.bluetooth_protocol {
                if let Some(peripheral) = proto.get_peripheral() {
                    let peripheral_clone = peripheral.clone();
                    let verbose = std::env::var("MC5000_VERBOSE").is_ok();
                    
                    iced::subscription::unfold(
                        "mc5000_notifications",
                        (peripheral_clone, None),  // Store the stream in state
                        move |(peripheral, mut stream_opt)| async move {
                            use btleplug::api::Peripheral as _;
                            use futures::stream::StreamExt;
                            
                            // Get or create the notification stream
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
                                        return (AppMessage::Tick, (peripheral, None));
                                    }
                                }
                            }
                            
                            // Try to get a notification from the stream
                            if let Some(stream) = stream_opt.as_mut() {
                                // Wait for notification with timeout
                                match tokio::time::timeout(
                                    Duration::from_millis(50),
                                    stream.next()
                                ).await {
                                    Ok(Some(notif)) => {
                                        if verbose {
                                            use std::io::Write;
                                            println!("[SUBSCRIPTION VERBOSE] ✓ Got notification: {} bytes", notif.value.len());
                                            let _ = std::io::stdout().flush();
                                        }
                                        return (AppMessage::NotificationReceived(notif.value), (peripheral, stream_opt));
                                    }
                                    Ok(None) => {
                                        if verbose {
                                            use std::io::Write;
                                            println!("[SUBSCRIPTION VERBOSE] Stream ended, will recreate");
                                            let _ = std::io::stdout().flush();
                                        }
                                        // Stream ended, recreate it next time
                                        return (AppMessage::Tick, (peripheral, None));
                                    }
                                    Err(_) => {
                                        // Timeout - return Tick to keep subscription alive
                                        return (AppMessage::Tick, (peripheral, stream_opt));
                                    }
                                }
                            }
                            
                            // Shouldn't reach here, but just in case
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            (AppMessage::Tick, (peripheral, stream_opt))
                        }
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
        
        Subscription::batch([tick, notifications])
    }

    fn theme(&self) -> Self::Theme {
        Theme::Dark
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