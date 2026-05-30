use iced::{
    widget::{button, column, container, pick_list, row, scrollable, text, text_input},
    Element, Length,
};

use crate::config_dialog::{ChargeConfig, ChargeMode};
use crate::i18n::t;
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
    
    // Profile management
    pub profile_name_input: String,
    pub selected_profile: Option<usize>,
    pub config_modified: bool, // Track if config was changed since profile was applied
    pub deleted_profile: Option<crate::profiles::Profile>, // For undo support
    /// Name of the last applied profile (persisted across sessions, resolved to index on open)
    pub last_profile_name: Option<String>,
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
        } else {
            // Below 1.7V: NiMH/NiCd (nominal 1.2V, discharged can be 0.8-1.3V)
            BatteryChemistry::NiMH
        }
    }

    pub fn new(voltage_v: f32) -> Self {
        let chemistry = Self::detect_chemistry_from_voltage(voltage_v);
        let mode = ChargeMode::Charge;  // Default mode
        let config = ChargeConfig::default_for_chemistry_and_mode(chemistry, mode);
        
        Self {
            chemistry,
            mode,
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
            profile_name_input: String::new(),
            selected_profile: None,
            config_modified: false,
            deleted_profile: None,
            last_profile_name: None,
        }
    }

    /// Reconstruct dialog state from a persisted slot config.
    /// `selected_profile` is left `None`; call `resolve_profile` afterwards.
    pub fn from_persisted(p: &crate::slot_persist::PersistedSlotConfig) -> Self {
        Self {
            chemistry: p.chemistry,
            mode: p.mode,
            capacity_input: p.config.capacity_mah.to_string(),
            charge_current_input: p.config.charge_current_ma.to_string(),
            discharge_current_input: p.config.discharge_current_ma.to_string(),
            target_voltage_input: format!("{:.2}", p.config.target_voltage_mv as f32 / 1000.0),
            cutoff_voltage_input: format!("{:.2}", p.config.cutoff_voltage_mv as f32 / 1000.0),
            storage_voltage_input: p.config.storage_voltage_mv
                .map(|v| format!("{:.2}", v as f32 / 1000.0))
                .unwrap_or_default(),
            delta_peak_input: p.config.delta_peak_mv.to_string(),
            trickle_charge_input: p.config.trickle_charge_ma.to_string(),
            cutoff_timer_input: p.config.cutoff_timer_min.to_string(),
            charge_cutoff_current_input: p.config.charge_cutoff_current_ma.to_string(),
            discharge_cutoff_current_input: p.config.discharge_cutoff_current_ma.to_string(),
            charge_resting_input: p.config.charge_resting_min.to_string(),
            discharge_resting_input: p.config.discharge_resting_min.to_string(),
            cycle_count_input: p.config.cycle_count.to_string(),
            config: p.config.clone(),
            profile_name_input: p.last_profile_name.clone().unwrap_or_default(),
            selected_profile: None,
            config_modified: false,
            deleted_profile: None,
            last_profile_name: p.last_profile_name.clone(),
        }
    }

    /// Look up `last_profile_name` in the profile store and set `selected_profile` to
    /// the matching index. Called when the dialog is opened to re-highlight the last profile.
    pub fn resolve_profile(&mut self, profile_store: &crate::profiles::ProfileStore) {
        if let Some(ref name) = self.last_profile_name.clone() {
            self.selected_profile = profile_store.profiles.iter().position(|p| &p.name == name);
        } else {
            self.selected_profile = None;
        }
    }

    pub fn update_chemistry(&mut self, new_chemistry: BatteryChemistry) {
        self.chemistry = new_chemistry;
        self.config = ChargeConfig::default_for_chemistry_and_mode(new_chemistry, self.mode);
        
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
        self.mode = new_mode;
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
        self.config = ChargeConfig::default_for_chemistry_and_mode(self.chemistry, self.mode);
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

    pub fn apply_profile(&mut self, profile: &crate::profiles::Profile) {
        self.chemistry = profile.chemistry;
        self.mode = profile.mode;
        self.config = profile.config.clone();
        self.profile_name_input = profile.name.clone();
        
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
}

pub fn view_config_dialog<'a>(state: &'a ConfigDialogState, profile_store: &'a crate::profiles::ProfileStore) -> Element<'a, AppMessage> {
    // === LEFT COLUMN: Configuration fields ===
    let all_chemistries = BatteryChemistry::all();
    let chemistry_picker = pick_list(
        all_chemistries,
        Some(state.chemistry),
        AppMessage::ConfigChemistryChanged,
    )
    .placeholder(t!("config.battery_type").to_string())
    .width(Length::Fixed(200.0));

    let available_modes = ChargeMode::available_for_chemistry(state.chemistry);
    let mode_picker = pick_list(
        available_modes,
        Some(state.mode),
        AppMessage::ConfigModeChanged,
    )
    .placeholder(t!("config.mode").to_string())
    .width(Length::Fixed(200.0));

    let mut left_col = column![
        text(t!("config.title").to_string()).size(18),
        row![text(format!("{}:", t!("config.battery_type"))).width(Length::Fixed(180.0)), chemistry_picker]
            .spacing(10)
            .align_y(iced::Center),
        row![text(format!("{}:", t!("config.mode"))).width(Length::Fixed(180.0)), mode_picker]
            .spacing(10)
            .align_y(iced::Center),
    ]
    .spacing(12)
    .width(Length::FillPortion(3));

    // Capacity
    left_col = left_col.push(
        row![
            text(format!("{} (mAh):", t!("config.capacity"))).width(Length::Fixed(180.0)),
            text_input("3000", &state.capacity_input)
                .on_input(AppMessage::ConfigCapacityChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Charge current
    if state.config.charge_current_ma > 0 || matches!(state.mode, ChargeMode::Charge | ChargeMode::Cycle | ChargeMode::Storage) {
        left_col = left_col.push(
            row![
                text(format!("{} (mA):", t!("config.charge_current"))).width(Length::Fixed(180.0)),
                text_input("1000", &state.charge_current_input)
                    .on_input(AppMessage::ConfigChargeCurrentChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Discharge current
    if state.config.discharge_current_ma > 0 || matches!(state.mode, ChargeMode::Discharge | ChargeMode::Cycle | ChargeMode::Storage | ChargeMode::Refresh) {
        left_col = left_col.push(
            row![
                text(format!("{} (mA):", t!("config.discharge_current"))).width(Length::Fixed(180.0)),
                text_input("1000", &state.discharge_current_input)
                    .on_input(AppMessage::ConfigDischargeCurrentChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Target voltage
    left_col = left_col.push(
        row![
            text(format!("{} (V):", t!("config.target_voltage"))).width(Length::Fixed(180.0)),
            text_input("4.2", &state.target_voltage_input)
                .on_input(AppMessage::ConfigTargetVoltageChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Cutoff voltage
    left_col = left_col.push(
        row![
            text(format!("{} (V):", t!("config.cutoff_voltage"))).width(Length::Fixed(180.0)),
            text_input("3.0", &state.cutoff_voltage_input)
                .on_input(AppMessage::ConfigCutoffVoltageChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Storage voltage
    if matches!(state.mode, ChargeMode::Storage) {
        left_col = left_col.push(
            row![
                text(format!("{} (V):", t!("config.storage_voltage"))).width(Length::Fixed(180.0)),
                text_input("3.8", &state.storage_voltage_input)
                    .on_input(AppMessage::ConfigStorageVoltageChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Delta-peak (NiMH/NiCd)
    if matches!(state.chemistry, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop) {
        left_col = left_col.push(
            row![
                text(format!("{} (mV):", t!("config.delta_peak"))).width(Length::Fixed(180.0)),
                text_input("6", &state.delta_peak_input)
                    .on_input(AppMessage::ConfigDeltaPeakChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
        left_col = left_col.push(
            row![
                text(format!("{} (mA):", t!("config.trickle_charge"))).width(Length::Fixed(180.0)),
                text_input("50", &state.trickle_charge_input)
                    .on_input(AppMessage::ConfigTrickleChargeChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
    }

    // Charge/discharge cutoff currents
    left_col = left_col.push(
        row![
            text(format!("{} (mA):", t!("config.charge_cutoff_current"))).width(Length::Fixed(180.0)),
            text_input("100", &state.charge_cutoff_current_input)
                .on_input(AppMessage::ConfigChargeCutoffCurrentChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );
    left_col = left_col.push(
        row![
            text(format!("{} (mA):", t!("config.discharge_cutoff_current"))).width(Length::Fixed(180.0)),
            text_input("100", &state.discharge_cutoff_current_input)
                .on_input(AppMessage::ConfigDischargeCutoffCurrentChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Cutoff timer
    left_col = left_col.push(
        row![
            text(format!("{}:", t!("config.cutoff_timer_desc"))).width(Length::Fixed(180.0)),
            text_input("0", &state.cutoff_timer_input)
                .on_input(AppMessage::ConfigCutoffTimerChanged)
                .width(Length::Fixed(150.0))
        ]
        .spacing(10)
        .align_y(iced::Center)
    );

    // Cycle-specific parameters
    if matches!(state.mode, ChargeMode::Cycle | ChargeMode::Refresh | ChargeMode::BreakIn) {
        left_col = left_col.push(
            row![
                text(format!("{} (min):", t!("config.charge_resting"))).width(Length::Fixed(180.0)),
                text_input("10", &state.charge_resting_input)
                    .on_input(AppMessage::ConfigChargeRestingChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
        left_col = left_col.push(
            row![
                text(format!("{} (min):", t!("config.discharge_resting"))).width(Length::Fixed(180.0)),
                text_input("10", &state.discharge_resting_input)
                    .on_input(AppMessage::ConfigDischargeRestingChanged)
                    .width(Length::Fixed(150.0))
            ]
            .spacing(10)
            .align_y(iced::Center)
        );
        if matches!(state.mode, ChargeMode::Cycle) {
            left_col = left_col.push(
                row![
                    text(format!("{}:", t!("config.cycle_count"))).width(Length::Fixed(180.0)),
                    text_input("1", &state.cycle_count_input)
                        .on_input(AppMessage::ConfigCycleCountChanged)
                        .width(Length::Fixed(150.0))
                ]
                .spacing(10)
                .align_y(iced::Center)
            );
        }
    }

    // Action buttons
    let action_text = match state.mode {
        ChargeMode::Charge => t!("config.start_charging").to_string(),
        ChargeMode::Storage => t!("config.start_storage").to_string(),
        ChargeMode::Discharge => t!("config.start_discharge").to_string(),
        ChargeMode::Cycle => t!("config.start_cycle").to_string(),
        ChargeMode::Refresh => t!("config.start_refresh").to_string(),
        ChargeMode::BreakIn => t!("config.start_breakin").to_string(),
    };

    left_col = left_col.push(
        row![
            button(text(t!("btn.cancel").to_string()))
                .on_press(AppMessage::ConfigDialogCancel)
                .padding(10),
            button(text(t!("btn.default").to_string()))
                .on_press(AppMessage::ConfigDialogDefault)
                .padding(10)
                .style(button::secondary),
            iced::widget::space::horizontal().width(30.0),
            button(text(action_text))
                .on_press(AppMessage::ConfigDialogConfirm)
                .padding(10)
                .style(button::primary),
        ]
        .spacing(10)
        .padding(iced::Padding { top: 15.0, right: 0.0, bottom: 0.0, left: 0.0 })
    );

    // === RIGHT COLUMN: Profile management ===
    let auto_name = crate::profiles::Profile::generate_name(state.chemistry, state.mode, &state.get_config());
    let placeholder_name = auto_name.clone();

    // Title row with Export/Import buttons aligned right
    let profile_title_row = row![
        text(t!("config.profiles").to_string()).size(18),
        iced::widget::space::horizontal().width(Length::Fill),
        button(text(t!("config.export_profiles").to_string()).size(14))
            .on_press(AppMessage::ConfigExportProfiles)
            .padding([4, 8]),
        button(text(t!("config.import_profiles").to_string()).size(14))
            .on_press(AppMessage::ConfigImportProfiles)
            .padding([4, 8]),
    ]
    .spacing(5)
    .align_y(iced::Center);

    // Profile name input
    let name_input = text_input(&placeholder_name, &state.profile_name_input)
        .on_input(AppMessage::ConfigProfileNameChanged)
        .width(Length::Fill)
        .size(14);

    // Create profile + Update buttons row
    let update_btn = if state.config_modified && state.selected_profile.is_some() {
        button(text(t!("config.update_profile").to_string()).size(14))
            .on_press(AppMessage::ConfigUpdateProfile)
            .padding([5, 10])
    } else {
        button(text(t!("config.update_profile").to_string()).size(14))
            .padding([5, 10])
    };

    // Combined action row: Create/Update on left, Undo/Delete on right with fill space between
    let action_row = {
        let mut r = row![
            button(text(t!("config.create_profile").to_string()).size(14))
                .on_press(AppMessage::ConfigSaveProfile)
                .padding([5, 10])
                .style(button::primary),
            update_btn,
            iced::widget::space::horizontal().width(Length::Fill),
        ];
        if state.deleted_profile.is_some() {
            r = r.push(
                button(text(t!("config.undo_delete").to_string()).size(14))
                    .on_press(AppMessage::ConfigUndoDelete)
                    .padding([5, 10])
                    .style(button::secondary)
            );
        }
        r = r.push(
            button(text(t!("config.delete_profile").to_string()).size(14))
                .on_press_maybe(state.selected_profile.map(|_| AppMessage::ConfigDeleteProfile))
                .padding([5, 10])
                .style(button::danger)
        );
        r.spacing(5)
    };

    // Profile list (sorted alphabetically)
    let sorted = profile_store.sorted_profiles();
    let profile_list: Vec<Element<AppMessage>> = sorted
        .iter()
        .map(|(idx, profile)| {
            let is_selected = state.selected_profile == Some(*idx);
            let style = if is_selected { button::primary } else { button::secondary };
            button(text(&profile.name).size(14))
                .on_press(AppMessage::ConfigSelectProfile(*idx))
                .padding([4, 8])
                .width(Length::Fill)
                .style(style)
                .into()
        })
        .collect();

    let profile_list_widget = scrollable(
        column(profile_list).spacing(3)
    )
    .height(Length::Fill);

    let right_col = column![
        profile_title_row,
        name_input,
        action_row,
        profile_list_widget,
    ]
    .spacing(8)
    .width(Length::FillPortion(2));

    // === Two-column layout ===
    let content = row![
        left_col,
        iced::widget::rule::vertical(1),
        right_col,
    ]
    .spacing(15)
    .padding(20);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
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
