use serde::{Deserialize, Serialize};
use crate::config_dialog::{ChargeConfig, ChargeMode};
use crate::slot::BatteryChemistry;

const APP: &str = "mc5000";
const CFG: &str = "profiles";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub chemistry: BatteryChemistry,
    pub mode: ChargeMode,
    pub config: ChargeConfig,
}

impl Profile {
    /// Generate a default name based on chemistry, mode and current
    pub fn generate_name(chemistry: BatteryChemistry, mode: ChargeMode, config: &ChargeConfig) -> String {
        let chem_name = match chemistry {
            BatteryChemistry::LiIon => "Li-ion",
            BatteryChemistry::LiIonHV => "Li-ion HV",
            BatteryChemistry::LiFePO4 => "LiFePO4",
            BatteryChemistry::NiMH => "NiMH",
            BatteryChemistry::NiCd => "NiCd",
            BatteryChemistry::Eneloop => "Eneloop",
            BatteryChemistry::NiZn => "NiZn",
            BatteryChemistry::RAM => "RAM",
            BatteryChemistry::LTO => "LTO",
            BatteryChemistry::NaIon => "Na-ion",
        };
        let mode_name = match mode {
            ChargeMode::Charge => "Charge",
            ChargeMode::Discharge => "Discharge",
            ChargeMode::Storage => "Storage",
            ChargeMode::Cycle => "Cycle",
            ChargeMode::Refresh => "Refresh",
            ChargeMode::BreakIn => "Break-in",
        };
        let current = if matches!(mode, ChargeMode::Discharge) {
            config.discharge_current_ma
        } else {
            config.charge_current_ma
        };
        format!("{} {} {}mA", chem_name, mode_name, current)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileStore {
    pub profiles: Vec<Profile>,
}

impl ProfileStore {
    pub fn load() -> Self {
        confy::load(APP, CFG).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), confy::ConfyError> {
        confy::store(APP, CFG, self)
    }

    pub fn sorted_profiles(&self) -> Vec<(usize, &Profile)> {
        let mut indexed: Vec<(usize, &Profile)> = self.profiles.iter().enumerate().collect();
        indexed.sort_by_key(|a| a.1.name.to_lowercase());
        indexed
    }

    pub fn add_profile(&mut self, profile: Profile) {
        self.profiles.push(profile);
        let _ = self.save();
    }

    pub fn delete_profile(&mut self, index: usize) {
        if index < self.profiles.len() {
            self.profiles.remove(index);
            let _ = self.save();
        }
    }

    pub fn export_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.profiles)?;
        std::fs::write(path, json)
    }

    pub fn import_from_file(path: &std::path::Path) -> std::io::Result<Vec<Profile>> {
        let content = std::fs::read_to_string(path)?;
        let profiles: Vec<Profile> = serde_json::from_str(&content)?;
        Ok(profiles)
    }
}
