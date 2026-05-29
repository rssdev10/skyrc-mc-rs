use serde::{Deserialize, Serialize};
use crate::slot::BatteryChemistry;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChargeMode {
    Charge,
    Storage,
    Discharge,
    Cycle,
    Refresh,
    BreakIn,
}

impl ChargeMode {
    pub fn available_for_chemistry(chemistry: BatteryChemistry) -> Vec<ChargeMode> {
        match chemistry {
            BatteryChemistry::LiIon | BatteryChemistry::LiIonHV | BatteryChemistry::LiFePO4 => {
                vec![
                    ChargeMode::Charge,
                    ChargeMode::Storage,
                    ChargeMode::Discharge,
                    ChargeMode::Cycle,
                ]
            }
            BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop => {
                vec![
                    ChargeMode::Charge,
                    ChargeMode::Refresh,
                    ChargeMode::BreakIn,
                    ChargeMode::Discharge,
                    ChargeMode::Cycle,
                ]
            }
            BatteryChemistry::NiZn | BatteryChemistry::RAM => {
                vec![ChargeMode::Charge, ChargeMode::Discharge, ChargeMode::Cycle]
            }
            BatteryChemistry::LTO | BatteryChemistry::NaIon => {
                // LTO and Na-Ion support Storage mode (confirmed series 5)
                vec![
                    ChargeMode::Charge,
                    ChargeMode::Storage,
                    ChargeMode::Discharge,
                    ChargeMode::Cycle,
                ]
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargeConfig {
    // Common parameters
    pub capacity_mah: u16,
    pub charge_current_ma: u16,
    pub discharge_current_ma: u16,
    
    // Voltage parameters
    pub target_voltage_mv: u16,
    pub cutoff_voltage_mv: u16,
    pub storage_voltage_mv: Option<u16>,
    pub keep_voltage_mv: Option<u16>,
    
    // Current cutoffs
    pub charge_cutoff_current_ma: u16,
    pub discharge_cutoff_current_ma: u16,
    
    // NiMH/NiCd specific
    pub delta_peak_mv: u16,
    pub trickle_charge_ma: u16,
    
    // Timing
    pub cutoff_timer_min: u16,
    pub charge_resting_min: u16,
    pub discharge_resting_min: u16,
    
    // Cycle mode
    pub cycle_mode: Option<String>,
    pub cycle_count: u16,
}

impl ChargeConfig {
    pub fn default_for_chemistry_and_mode(
        chemistry: BatteryChemistry,
        mode: ChargeMode,
    ) -> Self {
        match chemistry {
            BatteryChemistry::LiIon => Self::default_li_ion(mode),
            BatteryChemistry::LiIonHV => Self::default_li_ion_hv(mode),
            BatteryChemistry::LiFePO4 => Self::default_lifepo4(mode),
            BatteryChemistry::NiMH => Self::default_nimh(mode),
            BatteryChemistry::NiCd => Self::default_nicd(mode),
            BatteryChemistry::Eneloop => Self::default_eneloop(mode),
            BatteryChemistry::NiZn => Self::default_nizn(mode),
            BatteryChemistry::LTO => Self::default_lto(mode),
            BatteryChemistry::RAM => Self::default_ram(mode),
            BatteryChemistry::NaIon => Self::default_naion(mode),
        }
    }

    fn default_li_ion(mode: ChargeMode) -> Self {
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 5000,
                charge_current_ma: 3000,
                discharge_current_ma: 0,
                target_voltage_mv: 4200,
                cutoff_voltage_mv: 3200,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Storage => Self {
                capacity_mah: 5000,
                charge_current_ma: 3000,
                discharge_current_ma: 2000,
                target_voltage_mv: 4200,
                cutoff_voltage_mv: 3200,
                storage_voltage_mv: Some(3800),
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Discharge => Self {
                capacity_mah: 5000,
                charge_current_ma: 0,
                discharge_current_ma: 2000,
                target_voltage_mv: 4200,
                cutoff_voltage_mv: 3200,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Cycle => Self {
                capacity_mah: 5000,
                charge_current_ma: 3000,
                discharge_current_ma: 2000,
                target_voltage_mv: 4200,
                cutoff_voltage_mv: 3200,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D".to_string()),
                cycle_count: 1,
            },
            _ => Self::default_li_ion(ChargeMode::Charge),
        }
    }

    fn default_li_ion_hv(mode: ChargeMode) -> Self {
        let mut config = Self::default_li_ion(mode);
        config.target_voltage_mv = 4350;
        config.cutoff_voltage_mv = 3400;
        if let Some(ref mut storage) = config.storage_voltage_mv {
            *storage = 3900;
        }
        config
    }

    fn default_lifepo4(mode: ChargeMode) -> Self {
        let mut config = Self::default_li_ion(mode);
        config.target_voltage_mv = 3650;
        config.cutoff_voltage_mv = 2900;
        if let Some(ref mut storage) = config.storage_voltage_mv {
            *storage = 3300;
        }
        config
    }

    fn default_nimh(mode: ChargeMode) -> Self {
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 3000,
                charge_current_ma: 400,
                discharge_current_ma: 0,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 90,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Refresh => Self {
                capacity_mah: 3000,
                charge_current_ma: 400,
                discharge_current_ma: 250,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 90,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::BreakIn => Self {
                capacity_mah: 3000,
                charge_current_ma: 400,
                discharge_current_ma: 250,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 90,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D>C".to_string()),
                cycle_count: 1,
            },
            ChargeMode::Discharge => Self {
                capacity_mah: 3000,
                charge_current_ma: 0,
                discharge_current_ma: 600,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 90,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Cycle => Self {
                capacity_mah: 3000,
                charge_current_ma: 300,
                discharge_current_ma: 600,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 90,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D>C".to_string()),
                cycle_count: 1,
            },
            _ => Self::default_nimh(ChargeMode::Charge),
        }
    }

    fn default_nicd(mode: ChargeMode) -> Self {
        let mut config = Self::default_nimh(mode);
        match mode {
            ChargeMode::Charge => {
                config.charge_current_ma = 300;
                config.trickle_charge_ma = 50;
                config.cutoff_timer_min = 90;
            }
            ChargeMode::Refresh => {
                config.charge_current_ma = 300;
                config.discharge_current_ma = 600;
                config.trickle_charge_ma = 50;
                config.cutoff_timer_min = 90;
                config.discharge_cutoff_current_ma = 100;
            }
            ChargeMode::Discharge => {
                config.discharge_current_ma = 1000;
                config.cutoff_timer_min = 90;
                config.discharge_cutoff_current_ma = 100;
            }
            ChargeMode::Cycle => {
                config.charge_current_ma = 300;
                config.discharge_current_ma = 1000;
                config.trickle_charge_ma = 50;
                config.cutoff_timer_min = 90;
                config.discharge_cutoff_current_ma = 100;
                config.cycle_mode = Some("C>D".to_string());
            }
            _ => {}
        }
        config
    }

    fn default_nizn(mode: ChargeMode) -> Self {
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 0,
                target_voltage_mv: 1900,
                cutoff_voltage_mv: 1100,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Discharge => Self {
                capacity_mah: 3000,
                charge_current_ma: 0,
                discharge_current_ma: 1000,
                target_voltage_mv: 1900,
                cutoff_voltage_mv: 1100,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Cycle => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 1000,
                target_voltage_mv: 1900,
                cutoff_voltage_mv: 1100,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D".to_string()),
                cycle_count: 1,
            },
            _ => Self::default_nizn(ChargeMode::Charge),
        }
    }

    fn default_lto(mode: ChargeMode) -> Self {
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 0,
                target_voltage_mv: 2850,
                cutoff_voltage_mv: 1800,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Storage => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 1100,
                target_voltage_mv: 2850,
                cutoff_voltage_mv: 1800,
                storage_voltage_mv: Some(2400),
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Discharge => Self {
                capacity_mah: 3000,
                charge_current_ma: 0,
                discharge_current_ma: 1000,
                target_voltage_mv: 2850,
                cutoff_voltage_mv: 1800,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Cycle => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 1000,
                target_voltage_mv: 2850,
                cutoff_voltage_mv: 1800,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D".to_string()),
                cycle_count: 1,
            },
            _ => Self::default_lto(ChargeMode::Charge),
        }
    }

    fn default_ram(mode: ChargeMode) -> Self {
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 1000,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 1,
            },
            ChargeMode::Discharge => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 1000,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 1,
            },
            ChargeMode::Cycle => Self {
                capacity_mah: 3000,
                charge_current_ma: 1000,
                discharge_current_ma: 1000,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D".to_string()),
                cycle_count: 1,
            },
            _ => Self::default_ram(ChargeMode::Charge),
        }
    }

    fn default_eneloop(mode: ChargeMode) -> Self {
        // eneloop is NiMH-based; same modes as NiMH but with eneloop-optimized defaults
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 2000,
                charge_current_ma: 1000,
                discharge_current_ma: 0,
                target_voltage_mv: 1650,
                cutoff_voltage_mv: 900,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 6,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            // Fall back to NiMH defaults for other modes
            _ => {
                let mut config = Self::default_nimh(mode);
                config.capacity_mah = 2000; // eneloop typical stdAA capacity
                config
            }
        }
    }

    fn default_naion(mode: ChargeMode) -> Self {
        match mode {
            ChargeMode::Charge => Self {
                capacity_mah: 5000,
                charge_current_ma: 2000,
                discharge_current_ma: 0,
                target_voltage_mv: 4000,
                cutoff_voltage_mv: 2000,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Storage => Self {
                capacity_mah: 5000,
                charge_current_ma: 3000,
                discharge_current_ma: 2000,
                target_voltage_mv: 4000,
                cutoff_voltage_mv: 2000,
                storage_voltage_mv: Some(3500),
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Discharge => Self {
                capacity_mah: 5000,
                charge_current_ma: 0,
                discharge_current_ma: 1950,
                target_voltage_mv: 4000,
                cutoff_voltage_mv: 2000,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 0,
                discharge_resting_min: 0,
                cycle_mode: None,
                cycle_count: 0,
            },
            ChargeMode::Cycle => Self {
                capacity_mah: 5000,
                charge_current_ma: 2000,
                discharge_current_ma: 2000,
                target_voltage_mv: 4000,
                cutoff_voltage_mv: 2000,
                storage_voltage_mv: None,
                keep_voltage_mv: None,
                charge_cutoff_current_ma: 100,
                discharge_cutoff_current_ma: 100,
                delta_peak_mv: 0,
                trickle_charge_ma: 0,
                cutoff_timer_min: 0,
                charge_resting_min: 10,
                discharge_resting_min: 10,
                cycle_mode: Some("C>D".to_string()),
                cycle_count: 1,
            },
            _ => Self::default_naion(ChargeMode::Charge),
        }
    }
}
