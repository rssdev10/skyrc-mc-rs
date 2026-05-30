//! Per-slot config persistence across app restarts.
//! Stored in the platform config dir via `confy`, key `"slot_configs"`.
//! On macOS: `~/Library/Application Support/mc5000/slot_configs.toml`

use serde::{Deserialize, Serialize};
use crate::config_dialog::{ChargeConfig, ChargeMode};
use crate::slot::BatteryChemistry;

const APP: &str = "mc5000";
const CFG: &str = "slot_configs";

/// Minimal persisted state for one slot's configure-and-start dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSlotConfig {
    pub chemistry: BatteryChemistry,
    pub mode: ChargeMode,
    pub config: ChargeConfig,
    /// Profile name that was last applied (if any).
    /// Stored by name (not index) so it survives profile list reorders.
    pub last_profile_name: Option<String>,
}

/// Holds persisted config for all 4 slots (named fields for clean TOML output).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlotConfigStore {
    pub slot0: Option<PersistedSlotConfig>,
    pub slot1: Option<PersistedSlotConfig>,
    pub slot2: Option<PersistedSlotConfig>,
    pub slot3: Option<PersistedSlotConfig>,
}

impl SlotConfigStore {
    pub fn load() -> Self {
        confy::load(APP, CFG).unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = confy::store(APP, CFG, self);
    }

    pub fn get(&self, slot: usize) -> Option<&PersistedSlotConfig> {
        match slot {
            0 => self.slot0.as_ref(),
            1 => self.slot1.as_ref(),
            2 => self.slot2.as_ref(),
            3 => self.slot3.as_ref(),
            _ => None,
        }
    }

    pub fn set(&mut self, slot: usize, config: Option<PersistedSlotConfig>) {
        match slot {
            0 => self.slot0 = config,
            1 => self.slot1 = config,
            2 => self.slot2 = config,
            3 => self.slot3 = config,
            _ => {}
        }
    }
}
