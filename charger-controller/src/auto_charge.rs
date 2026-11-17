//! Chemistry detection and charge settings for the Auto and Smart buttons, and the
//! chemistry suggestion of the Configure dialog.
//!
//! - **Auto**: detect the chemistry from the resting voltage and start charging right away
//!   at a fixed 500 mA, with the Configure dialog's other defaults for that chemistry.
//! - **Smart**: detect the chemistry, run a short low-current probe until the charger
//!   reports the cell's internal resistance, then charge at a current chosen from that
//!   resistance (never above about 0.5 C of the smallest common cell). Cells whose
//!   resistance is too high (worn, or not rechargeable) are not charged.
//! - **Every start** (dialog, Auto, Smart) is refused when the measured voltage cannot
//!   belong to the chosen chemistry ([`check_voltage_fits`]).
//!
//! Voltage alone cannot tell several chemistries apart, so unattended detection only
//! commits where it is unambiguous and never picks a chemistry whose target voltage could
//! overcharge a lower-voltage cell (Li-Ion HV is never auto-selected).

use crate::config_dialog::{ChargeConfig as DialogConfig, ChargeMode};
use crate::slot::{BatteryChemistry, TaskConfig, TaskType};
use mc5000_protocol::{ChargeConfig, OperationMode};

// Detection, Auto's current and Smart's rules are shared with the CLI
// (`mc5000_protocol::charging`); these wrappers take the GUI's chemistry type.
pub use mc5000_protocol::charging::{
    SkipReason, SmartCurrent, SmartRefusal, AUTO_CURRENT_MA, EMPTY_V, LI_ION_MIN_V,
};

/// Detection for unattended charging: the chemistry, or why the cell is skipped.
pub fn detect_chemistry(voltage_v: f32) -> Result<BatteryChemistry, SkipReason> {
    mc5000_protocol::charging::detect_chemistry(voltage_v).map(BatteryChemistry::from_bluetooth_chemistry)
}

/// Suggestion for the Configure dialog, which the user reviews. Where the voltage is
/// ambiguous it proposes the chemistry with the lowest target voltage that fits, so
/// confirming the suggestion can never overcharge the cell.
pub fn suggest_chemistry(voltage_v: f32) -> BatteryChemistry {
    if voltage_v >= LI_ION_MIN_V {
        BatteryChemistry::LiIon
    } else if voltage_v >= 2.5 {
        BatteryChemistry::LiFePO4
    } else if voltage_v >= 1.75 {
        BatteryChemistry::NiZn
    } else {
        BatteryChemistry::NiMH
    }
}

/// Current of Smart's measuring probe.
pub fn probe_current_ma(chemistry: BatteryChemistry) -> u16 {
    mc5000_protocol::charging::probe_current_ma(chemistry.to_bluetooth_chemistry())
}

/// Smart's charge current for a cell with `resistance_mohm`, or why it is not charged
/// (rules: `mc5000_protocol::charging::smart_current_ma`).
pub fn smart_current_ma(
    chemistry: BatteryChemistry,
    resistance_mohm: u16,
) -> Result<SmartCurrent, SmartRefusal> {
    mc5000_protocol::charging::smart_current_ma(chemistry.to_bluetooth_chemistry(), resistance_mohm)
}

/// Why a cell's measured voltage cannot belong to the chosen chemistry.
#[derive(Debug, Clone, PartialEq)]
pub enum VoltageMismatch {
    /// Above the chemistry's target voltage: the cell is another, higher-voltage chemistry.
    TooHigh { voltage: f32, limit: f32 },
    /// Below what a lithium-type cell can have: probably a nickel cell or a dead cell.
    TooLow { voltage: f32, min: f32 },
}

impl std::fmt::Display for VoltageMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoltageMismatch::TooHigh { voltage, limit } => write!(
                f,
                "the cell reads {voltage:.2} V, above this chemistry's limit of {limit:.2} V"
            ),
            VoltageMismatch::TooLow { voltage, min } => write!(
                f,
                "the cell reads {voltage:.2} V, below the {min:.2} V this chemistry can have"
            ),
        }
    }
}

/// Checks that a cell at `voltage_v` can be `chemistry` before any program is started on
/// it, whatever the mode: charging a Li-Ion cell with a nickel profile or discharging it to
/// a nickel cutoff would damage it. An empty or unmeasured slot (`< EMPTY_V`) passes.
pub fn check_voltage_fits(
    chemistry: BatteryChemistry,
    voltage_v: f32,
) -> Result<(), VoltageMismatch> {
    if voltage_v < EMPTY_V {
        return Ok(());
    }
    let limit = chemistry.target_voltage() + 0.2;
    if voltage_v > limit {
        return Err(VoltageMismatch::TooHigh {
            voltage: voltage_v,
            limit,
        });
    }
    let min = match chemistry {
        BatteryChemistry::LiIon | BatteryChemistry::LiIonHV | BatteryChemistry::LiFePO4 => 2.0,
        BatteryChemistry::LTO | BatteryChemistry::NaIon => 1.5,
        _ => 0.0,
    };
    if voltage_v < min {
        return Err(VoltageMismatch::TooLow {
            voltage: voltage_v,
            min,
        });
    }
    Ok(())
}

fn bt_mode(mode: ChargeMode) -> OperationMode {
    match mode {
        ChargeMode::Charge => OperationMode::Charge,
        ChargeMode::Storage => OperationMode::Storage,
        ChargeMode::Discharge => OperationMode::Discharge,
        ChargeMode::Cycle => OperationMode::Cycle,
        ChargeMode::Refresh => OperationMode::Refresh,
        ChargeMode::BreakIn => OperationMode::BreakIn,
    }
}

/// The charger configuration for the Configure dialog's settings (what Confirm sends).
pub fn to_protocol_config(
    slot_idx: usize,
    chemistry: BatteryChemistry,
    mode: ChargeMode,
    config: &DialogConfig,
) -> ChargeConfig {
    ChargeConfig {
        channel_bitmask: 1 << slot_idx,
        mode: bt_mode(mode),
        chemistry: chemistry.to_bluetooth_chemistry(),
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
        eneloop_type: mc5000_protocol::EneloopType::from_byte(config.eneloop_type).unwrap_or_default(),
    }
}

/// A charge with the Configure dialog's defaults for `chemistry`, optionally at another
/// current: the charger configuration and the task the GUI tracks for the slot.
pub fn charge_plan(
    slot_idx: usize,
    chemistry: BatteryChemistry,
    current_ma: Option<u16>,
) -> (ChargeConfig, TaskConfig) {
    let mut defaults = DialogConfig::default_for_chemistry_and_mode(chemistry, ChargeMode::Charge);
    if let Some(current) = current_ma {
        defaults.charge_current_ma = current;
    }
    let config = to_protocol_config(slot_idx, chemistry, ChargeMode::Charge, &defaults);
    let task = TaskConfig {
        task_type: TaskType::Charge,
        battery_chemistry: chemistry,
        target_voltage: f32::from(defaults.target_voltage_mv) / 1000.0,
        target_current: f32::from(defaults.charge_current_ma) / 1000.0,
        cutoff_voltage: Some(f32::from(defaults.cutoff_voltage_mv) / 1000.0),
        capacity_limit: Some(u32::from(defaults.capacity_mah)),
        time_limit: None,
        temperature_limit: None,
        charge_current_ma: defaults.charge_current_ma,
        discharge_current_ma: 0,
    };
    (config, task)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_commits_only_where_voltage_is_unambiguous() {
        assert_eq!(detect_chemistry(4.13), Ok(BatteryChemistry::LiIon));
        assert_eq!(
            detect_chemistry(3.85),
            Ok(BatteryChemistry::LiIon),
            "storage-level Li-Ion"
        );
        assert_eq!(detect_chemistry(1.27), Ok(BatteryChemistry::NiMH));
        assert_eq!(detect_chemistry(0.95), Ok(BatteryChemistry::NiMH));
        for ambiguous in [3.3, 2.6, 1.9, 1.55] {
            assert!(detect_chemistry(ambiguous).is_err(), "{ambiguous} V");
        }
        assert!(detect_chemistry(0.0).is_err());
    }

    #[test]
    fn nothing_suggests_li_ion_hv() {
        for tenths in 0..=45 {
            let v = f32::from(tenths as u8) / 10.0;
            assert_ne!(suggest_chemistry(v), BatteryChemistry::LiIonHV, "{v} V");
            if let Ok(chemistry) = detect_chemistry(v) {
                assert_ne!(chemistry, BatteryChemistry::LiIonHV, "{v} V");
            }
        }
    }

    #[test]
    fn suggestion_never_targets_above_a_plausible_cell() {
        // A partly charged Li-Ion (3.85 V) must not be suggested a 4.35 V target.
        assert_eq!(suggest_chemistry(3.85), BatteryChemistry::LiIon);
        // A depleted Li-Ion or a LiFePO4 at 3.3 V gets the lower 3.65 V target.
        assert_eq!(suggest_chemistry(3.3), BatteryChemistry::LiFePO4);
    }

    #[test]
    fn smart_current_never_exceeds_half_c_of_the_smallest_cell() {
        // Resistances measured on a live charger.
        let cases = [
            (BatteryChemistry::NiMH, 155, 400, "AAA"),
            (BatteryChemistry::NiMH, 346, 250, "worn AA"),
            (BatteryChemistry::NiMH, 461, 200, "worn AA"),
            (
                BatteryChemistry::NiMH,
                40,
                400,
                "low R never raises nickel current",
            ),
            (BatteryChemistry::LiIon, 17, 2000, "21700"),
            (BatteryChemistry::LiIon, 33, 1000, "18650"),
            (BatteryChemistry::LiIon, 200, 500, "small lithium cell"),
        ];
        for (chemistry, resistance, expected, cell) in cases {
            let smart = smart_current_ma(chemistry, resistance).expect(cell);
            assert_eq!(smart.current_ma, expected, "{cell}");
            assert!(!smart.worn, "{cell}");
        }
    }

    #[test]
    fn worn_nickel_cells_are_charged_gently_and_flagged() {
        // The AA that read 548 mΩ on a live charger.
        assert_eq!(
            smart_current_ma(BatteryChemistry::NiMH, 548),
            Ok(SmartCurrent {
                current_ma: 200,
                worn: true
            })
        );
        assert!(smart_current_ma(BatteryChemistry::NiMH, 1000).is_ok());
        assert_eq!(
            smart_current_ma(BatteryChemistry::NiMH, 1001),
            Err(SmartRefusal::TooResistive {
                resistance_mohm: 1001
            })
        );
    }

    #[test]
    fn smart_refuses_unmeasured_or_too_resistive_cells() {
        assert_eq!(
            smart_current_ma(BatteryChemistry::NiMH, 0),
            Err(SmartRefusal::NotMeasured)
        );
        assert!(smart_current_ma(BatteryChemistry::NiMH, 1500).is_err());
        assert!(smart_current_ma(BatteryChemistry::LiIon, 400).is_err());
    }

    #[test]
    fn voltage_check_refuses_another_chemistrys_cell() {
        // A Li-Ion cell (4.11 V) configured as NiMH, as happened on the live charger.
        assert!(matches!(
            check_voltage_fits(BatteryChemistry::NiMH, 4.11),
            Err(VoltageMismatch::TooHigh { .. })
        ));
        assert!(check_voltage_fits(BatteryChemistry::LiFePO4, 4.11).is_err());
        // A NiMH cell configured as Li-Ion.
        assert!(matches!(
            check_voltage_fits(BatteryChemistry::LiIon, 1.30),
            Err(VoltageMismatch::TooLow { .. })
        ));
        // Matching cells, and an empty slot, pass.
        assert!(check_voltage_fits(BatteryChemistry::NiMH, 1.30).is_ok());
        assert!(check_voltage_fits(BatteryChemistry::LiIon, 4.11).is_ok());
        assert!(check_voltage_fits(BatteryChemistry::LiIon, 4.20).is_ok());
        assert!(check_voltage_fits(BatteryChemistry::NiMH, 0.0).is_ok());
    }

    #[test]
    fn auto_charges_every_chemistry_at_500_ma() {
        for chemistry in [BatteryChemistry::LiIon, BatteryChemistry::NiMH] {
            let (config, _) = charge_plan(0, chemistry, Some(AUTO_CURRENT_MA));
            assert_eq!(config.charge_current_ma, 500, "{chemistry:?}");
        }
    }

    #[test]
    fn charge_plan_uses_dialog_defaults_and_the_given_current() {
        let (config, task) = charge_plan(3, BatteryChemistry::LiIon, None);
        assert_eq!(config.channel_bitmask, 0x08);
        assert_eq!(config.mode, OperationMode::Charge);
        assert_eq!(
            (config.charge_current_ma, config.target_voltage_mv),
            (3000, 4200)
        );
        assert_eq!(task.task_type, TaskType::Charge);

        let (config, _) = charge_plan(1, BatteryChemistry::NiMH, Some(400));
        assert_eq!(config.channel_bitmask, 0x02);
        assert_eq!(
            (config.charge_current_ma, config.target_voltage_mv),
            (400, 1650)
        );
    }
}
