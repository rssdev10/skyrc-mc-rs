use iced::{
    widget::{button, column, container, pick_list, row, text, text_input},
    Element, Length,
};

use crate::config_dialog::{ChargeConfig, ChargeMode};
use crate::slot::BatteryChemistry;
use crate::app::AppMessage;

#[derive(Debug, Clone)]
pub struct ConfigDialogState {
    pub chemistry: BatteryChemistry,
    pub mode: ChargeMode,
    pub config: ChargeConfig,
    
    // Input field states
    pub capacity_input: String,
    pub charge_current_input: String,
    pub discharge_current_input: String,
    pub target_voltage_input: String,
    pub cutoff_voltage_input: String,
    pub storage_voltage_input: String,
    pub delta_peak_input: String,
    pub trickle_charge_input: String,
    pub cutoff_timer_input: String,
    pub charge_cutoff_current_input: String,
    pub discharge_cutoff_current_input: String,
    pub charge_resting_input: String,
    pub discharge_resting_input: String,
    pub cycle_count_input: String,
}

impl ConfigDialogState {
    /// Detect battery chemistry based on current voltage
    pub fn detect_chemistry_from_voltage(voltage_v: f32) -> BatteryChemistry {
        // Use voltage ranges to suggest chemistry type
        if voltage_v >= 4.0 {
            BatteryChemistry::LiIon  // 4.2V nominal
        } else if voltage_v >= 3.8 {
            BatteryChemistry::LiIonHV  // 4.35V nominal
        } else if voltage_v >= 3.3 {
            BatteryChemistry::LiFePO4  // 3.65V nominal
        } else if voltage_v >= 2.2 {
            BatteryChemistry::LTO  // 2.8V nominal
        } else if voltage_v >= 1.7 {
            BatteryChemistry::NiZn  // 1.85V nominal
        } else if voltage_v >= 1.3 {
            BatteryChemistry::NiMH  // 1.65V nominal (or NiCd)
        } else {
            BatteryChemistry::LiIon  // Default fallback
        }
    }

    pub fn new(voltage_v: f32) -> Self {
        let chemistry = Self::detect_chemistry_from_voltage(voltage_v);
        let mode = ChargeMode::Charge;  // Default mode
        let config = ChargeConfig::default_for_chemistry_and_mode(chemistry, mode);
        
        Self {
            chemistry,
            mode: mode.clone(),
            capacity_input: config.capacity_mah.to_string(),
            charge_current_input: config.charge_current_ma.to_string(),
            discharge_current_input: config.discharge_current_ma.to_string(),
            target_voltage_input: format!("{:.2}", config.target_voltage_mv as f32 / 1000.0),
            cutoff_voltage_input: format!("{:.2}", config.cutoff_voltage_mv as f32 / 1000.0),
            storage_voltage_input: config.storage_voltage_mv.map(|v| format!("{:.2}", v as f32 / 1000.0)).unwrap_or_default(),
            delta_peak_input: config.delta_peak_mv.to_string(),
            trickle_charge_input: config.trickle_charge_ma.to_string(),
            cutoff_timer_input: config.cutoff_timer_min.to_string(),
            charge_cutoff_current_input: config.charge_cutoff_current_ma.to_string(),
            discharge_cutoff_current_input: config.discharge_cutoff_current_ma.to_string(),
            charge_resting_input: config.charge_resting_min.to_string(),
            discharge_resting_input: config.discharge_resting_min.to_string(),
            cycle_count_input: config.cycle_count.to_string(),
            config,
        }
    }

    pub fn update_chemistry(&mut self, new_chemistry: BatteryChemistry) {
        self.chemistry = new_chemistry;
        self.config = ChargeConfig::default_for_chemistry_and_mode(new_chemistry, self.mode.clone());
        
        // Update all input fields with new defaults
        self.capacity_input = self.config.capacity_mah.to_string();
        self.charge_current_input = self.config.charge_current_ma.to_string();
        self.discharge_current_input = self.config.discharge_current_ma.to_string();
        self.target_voltage_input = format!("{:.2}", self.config.target_voltage_mv as f32 / 1000.0);
        self.cutoff_voltage_input = format!("{:.2}", self.config.cutoff_voltage_mv as f32 / 1000.0);
        self.storage_voltage_input = self.config.storage_voltage_mv.map(|v| format!("{:.2}", v as f32 / 1000.0)).unwrap_or_default();
        self.delta_peak_input = self.config.delta_peak_mv.to_string();
        self.trickle_charge_input = self.config.trickle_charge_ma.to_string();
        self.cutoff_timer_input = self.config.cutoff_timer_min.to_string();
        self.charge_cutoff_current_input = self.config.charge_cutoff_current_ma.to_string();
        self.discharge_cutoff_current_input = self.config.discharge_cutoff_current_ma.to_string();
        self.charge_resting_input = self.config.charge_resting_min.to_string();
        self.discharge_resting_input = self.config.discharge_resting_min.to_string();
        self.cycle_count_input = self.config.cycle_count.to_string();
    }

    pub fn update_mode(&mut self, new_mode: ChargeMode) {
        self.mode = new_mode.clone();
        self.config = ChargeConfig::default_for_chemistry_and_mode(self.chemistry, new_mode);
        
        // Update all input fields
        self.capacity_input = self.config.capacity_mah.to_string();
        self.charge_current_input = self.config.charge_current_ma.to_string();
        self.discharge_current_input = self.config.discharge_current_ma.to_string();
        self.target_voltage_input = format!("{:.2}", self.config.target_voltage_mv as f32 / 1000.0);
        self.cutoff_voltage_input = format!("{:.2}", self.config.cutoff_voltage_mv as f32 / 1000.0);
        self.storage_voltage_input = self.config.storage_voltage_mv.map(|v| format!("{:.2}", v as f32 / 1000.0)).unwrap_or_default();
        self.delta_peak_input = self.config.delta_peak_mv.to_string();
        self.trickle_charge_input = self.config.trickle_charge_ma.to_string();
        self.cutoff_timer_input = self.config.cutoff_timer_min.to_string();
        self.charge_cutoff_current_input = self.config.charge_cutoff_current_ma.to_string();
        self.discharge_cutoff_current_input = self.config.discharge_cutoff_current_ma.to_string();
        self.charge_resting_input = self.config.charge_resting_min.to_string();
        self.discharge_resting_input = self.config.discharge_resting_min.to_string();
        self.cycle_count_input = self.config.cycle_count.to_string();
    }

    pub fn reset_to_defaults(&mut self) {
        self.config = ChargeConfig::default_for_chemistry_and_mode(self.chemistry, self.mode.clone());
        self.capacity_input = self.config.capacity_mah.to_string();
        self.charge_current_input = self.config.charge_current_ma.to_string();
        self.discharge_current_input = self.config.discharge_current_ma.to_string();
        self.target_voltage_input = format!("{:.2}", self.config.target_voltage_mv as f32 / 1000.0);
        self.cutoff_voltage_input = format!("{:.2}", self.config.cutoff_voltage_mv as f32 / 1000.0);
        self.storage_voltage_input = self.config.storage_voltage_mv.map(|v| format!("{:.2}", v as f32 / 1000.0)).unwrap_or_default();
        self.delta_peak_input = self.config.delta_peak_mv.to_string();
        self.trickle_charge_input = self.config.trickle_charge_ma.to_string();
        self.cutoff_timer_input = self.config.cutoff_timer_min.to_string();
        self.charge_cutoff_current_input = self.config.charge_cutoff_current_ma.to_string();
        self.discharge_cutoff_current_input = self.config.discharge_cutoff_current_ma.to_string();
        self.charge_resting_input = self.config.charge_resting_min.to_string();
        self.discharge_resting_input = self.config.discharge_resting_min.to_string();
        self.cycle_count_input = self.config.cycle_count.to_string();
    }

    pub fn get_config(&self) -> ChargeConfig {
        let mut config = self.config.clone();
        
        if let Ok(val) = self.capacity_input.parse() {
            config.capacity_mah = val;
        }
        if let Ok(val) = self.charge_current_input.parse() {
            config.charge_current_ma = val;
        }
        if let Ok(val) = self.discharge_current_input.parse() {
            config.discharge_current_ma = val;
        }
        if let Ok(val) = self.target_voltage_input.parse::<f32>() {
            config.target_voltage_mv = (val * 1000.0) as u16;
        }
        if let Ok(val) = self.cutoff_voltage_input.parse::<f32>() {
            config.cutoff_voltage_mv = (val * 1000.0) as u16;
        }
        if !self.storage_voltage_input.is_empty() {
            if let Ok(val) = self.storage_voltage_input.parse::<f32>() {
                config.storage_voltage_mv = Some((val * 1000.0) as u16);
            }
        }
        if let Ok(val) = self.delta_peak_input.parse() {
            config.delta_peak_mv = val;
        }
        if let Ok(val) = self.trickle_charge_input.parse() {
            config.trickle_charge_ma = val;
        }
        if let Ok(val) = self.cutoff_timer_input.parse() {
            config.cutoff_timer_min = val;
        }
        if let Ok(val) = self.charge_cutoff_current_input.parse() {
            config.charge_cutoff_current_ma = val;
        }
        if let Ok(val) = self.discharge_cutoff_current_input.parse() {
            config.discharge_cutoff_current_ma = val;
        }
        if let Ok(val) = self.charge_resting_input.parse() {
            config.charge_resting_min = val;
        }
        if let Ok(val) = self.discharge_resting_input.parse() {
            config.discharge_resting_min = val;
        }
        if let Ok(val) = self.cycle_count_input.parse() {
            config.cycle_count = val;
        }
        
        config
    }
}

pub fn view_config_dialog(state: &ConfigDialogState) -> Element<'_, AppMessage> {
    // Battery type picker
    let all_chemistries = BatteryChemistry::all();
    let chemistry_picker = pick_list(
        all_chemistries,
        Some(state.chemistry),
        AppMessage::ConfigChemistryChanged,
    )
    .placeholder("Select battery type")
    .width(Length::Fixed(200.0));

    // Mode picker - filtered by chemistry
    let available_modes = ChargeMode::available_for_chemistry(state.chemistry);
    
    let mode_picker = pick_list(
        available_modes,
        Some(state.mode.clone()),
        AppMessage::ConfigModeChanged,
    )
    .placeholder("Select mode")
    .width(Length::Fixed(200.0));

    let mut content = column![
        text("Battery Configuration")
        .size(24),
        row![text("Battery Type: ").width(Length::Fixed(150.0)), chemistry_picker]
            .spacing(10)
            .align_y(iced::Center),
        row![text("Mode: ").width(Length::Fixed(150.0)), mode_picker]
            .spacing(10)
            .align_y(iced::Center),
    ]
    .spacing(15)
    .padding(20);

    // Capacity (always shown)
    content = content.push(
        row![
            text("Capacity (mAh):").width(Length::Fixed(200.0)),
            text_input("3000", &state.capacity_input)
                .on_input(AppMessage::ConfigCapacityChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Charge current (for charge/cycle/storage modes)
    if state.config.charge_current_ma > 0 || matches!(state.mode, ChargeMode::Charge | ChargeMode::Cycle | ChargeMode::Storage) {
        content = content.push(
            row![
                text("Charge Current (mA):").width(Length::Fixed(200.0)),
                text_input("1000", &state.charge_current_input)
                    .on_input(AppMessage::ConfigChargeCurrentChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Discharge current (for discharge/cycle/storage modes)
    if state.config.discharge_current_ma > 0 || matches!(state.mode, ChargeMode::Discharge | ChargeMode::Cycle | ChargeMode::Storage | ChargeMode::Refresh) {
        content = content.push(
            row![
                text("Discharge Current (mA):").width(Length::Fixed(200.0)),
                text_input("1000", &state.discharge_current_input)
                    .on_input(AppMessage::ConfigDischargeCurrentChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Target voltage
    content = content.push(
        row![
            text("Target Voltage (V):").width(Length::Fixed(200.0)),
            text_input("4.2", &state.target_voltage_input)
                .on_input(AppMessage::ConfigTargetVoltageChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Cutoff voltage
    content = content.push(
        row![
            text("Cutoff Voltage (V):").width(Length::Fixed(200.0)),
            text_input("3.0", &state.cutoff_voltage_input)
                .on_input(AppMessage::ConfigCutoffVoltageChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Storage voltage (for storage mode)
    if matches!(state.mode, ChargeMode::Storage) {
        content = content.push(
            row![
                text("Storage Voltage (V):").width(Length::Fixed(200.0)),
                text_input("3.8", &state.storage_voltage_input)
                    .on_input(AppMessage::ConfigStorageVoltageChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Delta-peak (for NiMH/NiCd/eneloop)
    if matches!(state.chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop) {
        content = content.push(
            row![
                text("Delta Peak (mV):").width(Length::Fixed(200.0)),
                text_input("6", &state.delta_peak_input)
                    .on_input(AppMessage::ConfigDeltaPeakChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );

        // Trickle charge
        content = content.push(
            row![
                text("Trickle Charge (mA):").width(Length::Fixed(200.0)),
                text_input("50", &state.trickle_charge_input)
                    .on_input(AppMessage::ConfigTrickleChargeChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Charge cutoff current
    content = content.push(
        row![
            text("Charge Cutoff Current (mA):").width(Length::Fixed(200.0)),
            text_input("100", &state.charge_cutoff_current_input)
                .on_input(AppMessage::ConfigChargeCutoffCurrentChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Discharge cutoff current
    content = content.push(
        row![
            text("Discharge Cutoff Current (mA):").width(Length::Fixed(200.0)),
            text_input("100", &state.discharge_cutoff_current_input)
                .on_input(AppMessage::ConfigDischargeCutoffCurrentChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Cutoff timer
    content = content.push(
        row![
            text("Cutoff Timer (min, 0=off):").width(Length::Fixed(200.0)),
            text_input("0", &state.cutoff_timer_input)
                .on_input(AppMessage::ConfigCutoffTimerChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Cycle-specific parameters
    if matches!(state.mode, ChargeMode::Cycle | ChargeMode::Refresh | ChargeMode::BreakIn) {
        content = content.push(
            row![
                text("Charge Resting (min):").width(Length::Fixed(200.0)),
                text_input("10", &state.charge_resting_input)
                    .on_input(AppMessage::ConfigChargeRestingChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );

        content = content.push(
            row![
                text("Discharge Resting (min):").width(Length::Fixed(200.0)),
                text_input("10", &state.discharge_resting_input)
                    .on_input(AppMessage::ConfigDischargeRestingChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );

        if matches!(state.mode, ChargeMode::Cycle) {
            content = content.push(
                row![
                    text("Cycle Count:").width(Length::Fixed(200.0)),
                    text_input("1", &state.cycle_count_input)
                        .on_input(AppMessage::ConfigCycleCountChanged)
                        .width(Length::Fixed(150.0))
                ]
                .spacing(10)
                .align_y(iced::Center)
            );
        }
    }

    // Determine action button text based on mode
    let action_text = match state.mode {
        ChargeMode::Charge => "Start Charging",
        ChargeMode::Storage => "Start Storage",
        ChargeMode::Discharge => "Start Discharge",
        ChargeMode::Cycle => "Start Cycle",
        ChargeMode::Refresh => "Start Refresh",
        ChargeMode::BreakIn => "Start Break-In",
    };

    // Buttons (macOS native order: Cancel on left, Action on right)
    content = content.push(
        row![
            button(text("Cancel"))
                .on_press(AppMessage::ConfigDialogCancel)
                .padding(10),
            button(text("Default"))
                .on_press(AppMessage::ConfigDialogDefault)
                .padding(10)
                .style(button::secondary),
            iced::widget::space::horizontal(),
            button(text(action_text))
                .on_press(AppMessage::ConfigDialogConfirm)
                .padding(10)
                .style(button::primary),
        ]
        .spacing(10)
        .padding(iced::Padding { top: 20.0, right: 0.0, bottom: 0.0, left: 0.0 })
    );

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(20)
        .into()
}

impl std::fmt::Display for ChargeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChargeMode::Charge => write!(f, "Charge"),
            ChargeMode::Storage => write!(f, "Storage"),
            ChargeMode::Discharge => write!(f, "Discharge"),
            ChargeMode::Cycle => write!(f, "Cycle"),
            ChargeMode::Refresh => write!(f, "Refresh"),
            ChargeMode::BreakIn => write!(f, "Break-in"),
        }
    }
}
