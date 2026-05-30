use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotId(pub usize);

#[derive(Debug, Clone)]
pub struct Slot {
    pub id: SlotId,
    pub state: SlotState,
    pub current_task: Option<TaskConfig>,
    pub current_voltage: f32,
    pub current_current: f32,
    pub capacity_mah: u32,
    pub resistance_milliohm: u16,
    pub elapsed_seconds: u16,
    pub start_time: Option<Instant>,
    pub last_update: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SlotState {
    Idle,
    Charging,
    Discharging,
    Completed,
    Error(String),
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    pub task_type: TaskType,
    pub battery_chemistry: BatteryChemistry,
    pub target_voltage: f32,
    pub target_current: f32,
    pub cutoff_voltage: Option<f32>,
    pub capacity_limit: Option<u32>, // mAh
    pub time_limit: Option<Duration>,
    pub temperature_limit: Option<f32>,
    pub charge_current_ma: u16,
    pub discharge_current_ma: u16,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatteryChemistry {
    LiIon,      // 4.2V
    LiIonHV,    // 4.35V
    LiFePO4,    // 3.65V
    NiMH,       // 1.65V
    NiCd,       // 1.65V
    Eneloop,    // 1.65V (eneloop NiMH)
    NiZn,       // 1.9V
    RAM,        // 1.65V
    LTO,        // 2.85V
    NaIon,      // 4.0V
}

impl BatteryChemistry {
    /// Get standard target voltage in volts (reuses bluetooth.rs values)
    pub fn target_voltage(&self) -> f32 {
        // Reuse the millivolt values from bluetooth.rs and convert to volts
        let bt_chem = self.to_bluetooth_chemistry();
        bt_chem.target_voltage_mv() as f32 / 1000.0
    }

    /// Get standard cutoff voltage in volts (reuses bluetooth.rs values)
    pub fn cutoff_voltage(&self) -> f32 {
        // Reuse the millivolt values from bluetooth.rs and convert to volts
        let bt_chem = self.to_bluetooth_chemistry();
        bt_chem.cutoff_voltage_mv() as f32 / 1000.0
    }

    /// Convert slot chemistry to bluetooth chemistry
    pub fn to_bluetooth_chemistry(self) -> mc5000_protocol::BatteryChemistry {
        match self {
            BatteryChemistry::LiIon => mc5000_protocol::BatteryChemistry::LiIon,
            BatteryChemistry::LiIonHV => mc5000_protocol::BatteryChemistry::LiIonHV,
            BatteryChemistry::LiFePO4 => mc5000_protocol::BatteryChemistry::LiFePO4,
            BatteryChemistry::NiMH => mc5000_protocol::BatteryChemistry::NiMH,
            BatteryChemistry::NiCd => mc5000_protocol::BatteryChemistry::NiCd,
            BatteryChemistry::Eneloop => mc5000_protocol::BatteryChemistry::Eneloop,
            BatteryChemistry::NiZn => mc5000_protocol::BatteryChemistry::NiZn,
            BatteryChemistry::RAM => mc5000_protocol::BatteryChemistry::RAM,
            BatteryChemistry::LTO => mc5000_protocol::BatteryChemistry::LTO,
            BatteryChemistry::NaIon => mc5000_protocol::BatteryChemistry::NaIon,
        }
    }

    pub fn all() -> Vec<BatteryChemistry> {
        vec![
            BatteryChemistry::LiIon,
            BatteryChemistry::LiIonHV,
            BatteryChemistry::LiFePO4,
            BatteryChemistry::NiMH,
            BatteryChemistry::NiCd,
            BatteryChemistry::Eneloop,
            BatteryChemistry::NiZn,
            BatteryChemistry::RAM,
            BatteryChemistry::LTO,
            BatteryChemistry::NaIon,
        ]
    }
}

impl std::fmt::Display for BatteryChemistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatteryChemistry::LiIon => write!(f, "Li-Ion (4.2V)"),
            BatteryChemistry::LiIonHV => write!(f, "Li-Ion HV (4.35V)"),
            BatteryChemistry::LiFePO4 => write!(f, "LiFePO4 (3.65V)"),
            BatteryChemistry::NiMH => write!(f, "NiMH (1.65V)"),
            BatteryChemistry::NiCd => write!(f, "NiCd (1.65V)"),
            BatteryChemistry::Eneloop => write!(f, "eneloop (1.65V)"),
            BatteryChemistry::NiZn => write!(f, "NiZn (1.9V)"),
            BatteryChemistry::RAM => write!(f, "RAM (1.65V)"),
            BatteryChemistry::LTO => write!(f, "LTO (2.85V)"),
            BatteryChemistry::NaIon => write!(f, "Na-Ion (4.0V)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskType {
    Charge,
    Discharge,
    Cycle { charge_cycles: u32, discharge_cycles: u32 },
    Storage,
    CalibrateFast,
    CalibrateFull,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Charge => write!(f, "Charge"),
            TaskType::Discharge => write!(f, "Discharge"),
            TaskType::Cycle { .. } => write!(f, "Cycle"),
            TaskType::Storage => write!(f, "Storage"),
            TaskType::CalibrateFast => write!(f, "Fast Calibrate"),
            TaskType::CalibrateFull => write!(f, "Full Calibrate"),
        }
    }
}

impl Slot {
    pub fn new(id: SlotId) -> Self {
        Self {
            id,
            state: SlotState::Idle,
            current_task: None,
            current_voltage: 0.0,
            current_current: 0.0,
            capacity_mah: 0,
            resistance_milliohm: 0,
            elapsed_seconds: 0,
            start_time: None,
            last_update: None,
        }
    }

    pub fn start_task(&mut self, config: TaskConfig) {
        self.current_task = Some(config.clone());
        self.state = match config.task_type {
            TaskType::Charge => SlotState::Charging,
            TaskType::Discharge => SlotState::Discharging,
            _ => SlotState::Charging, // Default to charging for other types
        };
        self.start_time = Some(Instant::now());
        self.last_update = Some(Instant::now());
        
        log::info!("Started task on slot {}: {:?}", self.id.0, config.task_type);
    }

    pub fn stop(&mut self) {
        self.state = SlotState::Idle;
        self.current_task = None;
        self.start_time = None;
        self.last_update = None;
        
        log::info!("Stopped task on slot {}", self.id.0);
    }

    #[allow(dead_code)]
    pub fn pause(&mut self) {
        if self.is_active() {
            self.state = SlotState::Paused;
            log::info!("Paused task on slot {}", self.id.0);
        }
    }

    #[allow(dead_code)]
    pub fn resume(&mut self) {
        if self.state == SlotState::Paused {
            if let Some(ref task) = self.current_task {
                self.state = match task.task_type {
                    TaskType::Charge => SlotState::Charging,
                    TaskType::Discharge => SlotState::Discharging,
                    _ => SlotState::Charging,
                };
                log::info!("Resumed task on slot {}", self.id.0);
            }
        }
    }

    pub fn update_measurement(&mut self, voltage: f32, current: f32) {
        self.current_voltage = voltage;
        self.current_current = current;
        self.last_update = Some(Instant::now());

        // Check for completion conditions
        if let Some(task) = self.current_task.clone() {
            self.check_completion_conditions(&task);
        }
    }
    
    pub fn update_full_status(&mut self, voltage: f32, current: f32, capacity: u32, resistance: u16, elapsed: u16) {
        self.current_voltage = voltage;
        self.current_current = current;
        self.capacity_mah = capacity;
        self.resistance_milliohm = resistance;
        self.elapsed_seconds = elapsed;
        self.last_update = Some(Instant::now());

        // Check for completion conditions
        if let Some(task) = self.current_task.clone() {
            self.check_completion_conditions(&task);
        }
    }
    
    pub fn power_w(&self) -> f32 {
        self.current_voltage * (self.current_current / 1000.0)
    }

    pub fn set_state(&mut self, state: SlotState) {
        self.state = state;
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, SlotState::Charging | SlotState::Discharging)
    }

    pub fn is_idle(&self) -> bool {
        self.state == SlotState::Idle
    }

    pub fn get_elapsed_time(&self) -> Option<Duration> {
        self.start_time.map(|start| start.elapsed())
    }

    pub fn get_progress_percentage(&self) -> f32 {
        // Calculate battery level using the formula:
        // clip((Vcur + I*R - Vmin) / (Vmax - Vmin), 0, 1) * 100
        
        // If no current voltage reading, can't calculate progress
        if self.current_voltage < 0.001 {
            return 0.0;
        }
        
        // Determine chemistry and voltage limits
        let (vmax, vmin, _chemistry) = if let Some(ref task) = self.current_task {
            // Use task configuration when available
            let (v_max, v_min) = match &task.task_type {
                TaskType::Charge => {
                    (task.target_voltage, task.battery_chemistry.cutoff_voltage())
                }
                TaskType::Discharge => {
                    (task.target_voltage, task.cutoff_voltage.unwrap_or_else(|| task.battery_chemistry.cutoff_voltage()))
                }
                TaskType::Storage => {
                    (task.battery_chemistry.target_voltage(), task.battery_chemistry.cutoff_voltage())
                }
                TaskType::Cycle { .. } => {
                    (task.target_voltage, task.cutoff_voltage.unwrap_or_else(|| task.battery_chemistry.cutoff_voltage()))
                }
                _ => {
                    (task.battery_chemistry.target_voltage(), task.battery_chemistry.cutoff_voltage())
                }
            };
            (v_max, v_min, Some(task.battery_chemistry))
        } else {
            // No task - estimate chemistry from current voltage and use defaults
            let chem = self.estimate_chemistry_from_voltage();
            (chem.target_voltage(), chem.cutoff_voltage(), Some(chem))
        };
        
        // Avoid division by zero
        if (vmax - vmin).abs() < 0.001 {
            return 0.0;
        }
        
        // Convert current from mA to A and resistance from milliohms to ohms
        let current_a = self.current_current / 1000.0;
        let resistance_ohm = self.resistance_milliohm as f32 / 1000.0;
        
        // Calculate compensated voltage: Vcur + I*R
        let v_compensated = self.current_voltage + (current_a * resistance_ohm);
        
        // Calculate progress: (V_compensated - Vmin) / (Vmax - Vmin)
        let progress = (v_compensated - vmin) / (vmax - vmin);
        
        // Clip to [0, 1] and convert to percentage
        progress.clamp(0.0, 1.0) * 100.0
    }
    
    pub fn estimate_chemistry_from_voltage(&self) -> BatteryChemistry {
        // Estimate battery chemistry based on current voltage
        let v = self.current_voltage;
        if v >= 4.0 {
            BatteryChemistry::LiIon
        } else if v >= 3.8 {
            BatteryChemistry::LiIonHV
        } else if v >= 3.3 {
            BatteryChemistry::LiFePO4
        } else if v >= 2.2 {
            BatteryChemistry::LTO
        } else if v >= 1.7 {
            BatteryChemistry::NiZn
        } else {
            // Below 1.7V: NiMH/NiCd (nominal 1.2V, discharged can be 0.8-1.3V)
            BatteryChemistry::NiMH
        }
    }

    fn check_completion_conditions(&mut self, task: &TaskConfig) {
        let mut completed = false;
        let mut error_msg = None;

        // Check voltage limits
        if let Some(cutoff_voltage) = task.cutoff_voltage {
            match task.task_type {
                TaskType::Charge => {
                    if self.current_voltage >= cutoff_voltage {
                        completed = true;
                    }
                }
                TaskType::Discharge if self.current_voltage <= cutoff_voltage => {
                    completed = true;
                }
                _ => {}
            }
        }

        // Check time limit
        if let Some(time_limit) = task.time_limit {
            if let Some(elapsed) = self.get_elapsed_time() {
                if elapsed >= time_limit {
                    completed = true;
                }
            }
        }

        // Check temperature limit
        if let Some(temp_limit) = task.temperature_limit {
            // In a real implementation, you'd get temperature from device
            let current_temp = 25.0; // Mock temperature
            if current_temp > temp_limit {
                error_msg = Some(format!("Temperature limit exceeded: {:.1}°C", current_temp));
            }
        }

        if let Some(msg) = error_msg {
            self.state = SlotState::Error(msg);
        } else if completed {
            self.state = SlotState::Completed;
            log::info!("Task completed on slot {}", self.id.0);
        }
    }
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            task_type: TaskType::Charge,
            battery_chemistry: BatteryChemistry::LiIon,
            target_voltage: 4.2,
            target_current: 1.0,
            cutoff_voltage: Some(3.2),
            capacity_limit: Some(3000),
            time_limit: None,
            temperature_limit: None,
            charge_current_ma: 1000,
            discharge_current_ma: 1000,
        }
    }
}

impl std::fmt::Display for SlotState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlotState::Idle => write!(f, "Idle"),
            SlotState::Charging => write!(f, "Charging"),
            SlotState::Discharging => write!(f, "Discharging"),
            SlotState::Completed => write!(f, "Completed"),
            SlotState::Error(msg) => write!(f, "Error: {}", msg),
            SlotState::Paused => write!(f, "Paused"),
        }
    }
}