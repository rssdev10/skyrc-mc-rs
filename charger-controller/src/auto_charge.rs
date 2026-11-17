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

/// At or above this resting voltage a cell is taken for Li-Ion (4.20 V target).
/// LiFePO4 never rests this high, and Li-Ion HV cannot be told apart, so it is not chosen.
pub const LI_ION_MIN_V: f32 = 3.7;
/// Below this resting voltage (and above an empty slot) a cell is taken for NiMH.
/// Above it, a full NiMH cell and a non-rechargeable alkaline cell look the same.
pub const NIMH_MAX_V: f32 = 1.5;
/// Below this the slot is treated as empty.
pub const EMPTY_V: f32 = 0.1;

/// Why unattended detection skips a cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SkipReason {
    Empty,
    /// The voltage fits several chemistries (LiFePO4, depleted Li-Ion, LTO, NiZn, or a
    /// non-rechargeable cell).
    Ambiguous {
        voltage: f32,
    },
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::Empty => write!(f, "empty slot"),
            SkipReason::Ambiguous { voltage } => write!(
                f,
                "{voltage:.2} V is ambiguous (LiFePO4, depleted Li-Ion, LTO, NiZn or a \
                 non-rechargeable cell); configure this slot manually"
            ),
        }
    }
}

/// Detection for unattended charging: the chemistry, or why the cell is skipped.
pub fn detect_chemistry(voltage_v: f32) -> Result<BatteryChemistry, SkipReason> {
    if voltage_v < EMPTY_V {
        Err(SkipReason::Empty)
    } else if voltage_v >= LI_ION_MIN_V {
        Ok(BatteryChemistry::LiIon)
    } else if voltage_v < NIMH_MAX_V {
        Ok(BatteryChemistry::NiMH)
    } else {
        Err(SkipReason::Ambiguous { voltage: voltage_v })
    }
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

fn is_nickel(chemistry: BatteryChemistry) -> bool {
    matches!(
        chemistry,
        BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop
    )
}

/// Auto's charge current for every chemistry: a moderate fixed current for starting
/// right away. The Configure dialog lets the user choose; Smart measures the cell first.
pub const AUTO_CURRENT_MA: u16 = 500;

/// Current of Smart's measuring probe: low enough for the smallest cell of the family.
pub fn probe_current_ma(chemistry: BatteryChemistry) -> u16 {
    if is_nickel(chemistry) {
        200
    } else {
        300
    }
}

/// Voltage the charge current may add across the cell's internal resistance.
const MAX_IR_DROP_MV: u32 = 100;

/// Below this resistance a lithium cell is taken for a large (21700/26650-class) cell.
/// Measured on a live charger: a high-current 21700 read 17 mΩ, an 18650 33 mΩ.
pub const LARGE_LITHIUM_MAX_MOHM: u16 = 25;

/// Above this resistance a nickel cell is charged only at the minimum current and flagged
/// as worn. Measured on a live charger: healthy-ish AA/AAA cells read 130–490 mΩ, a worn AA
/// 548 mΩ (the reading includes contact resistance).
pub const NICKEL_WORN_MOHM: u16 = 500;
/// Above this resistance a nickel cell is not charged: likely broken or not rechargeable.
pub const NICKEL_MAX_MOHM: u16 = 1000;
/// Above this resistance a lithium cell is not charged.
pub const LITHIUM_MAX_MOHM: u16 = 300;

/// Smart's choice for a measured cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartCurrent {
    pub current_ma: u16,
    /// High resistance: charged gently at the minimum current; the user should know.
    pub worn: bool,
}

/// Why Smart does not charge a measured cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartRefusal {
    NotMeasured,
    TooResistive { resistance_mohm: u16 },
}

impl std::fmt::Display for SmartRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmartRefusal::NotMeasured => write!(f, "internal resistance was not measured"),
            SmartRefusal::TooResistive { resistance_mohm } => write!(
                f,
                "internal resistance {resistance_mohm} mΩ is too high (broken or not rechargeable)"
            ),
        }
    }
}

/// Smart's charge current for a cell with `resistance_mohm`, or why it is not charged.
///
/// The ceiling is about 0.5 C of the smallest common cell the reading allows:
///
/// - nickel: 400 mA (an ~800 mAh AAA). The charger's reading includes contact resistance
///   and reflects condition more than size (a small AAA read 155–175 mΩ, worn AA cells
///   340–550 mΩ), so it never raises the nickel current. Cells above 500 mΩ are charged
///   at the 200 mA minimum and flagged as worn; above 1000 mΩ they are refused;
/// - lithium: 2000 mA below 25 mΩ (21700/26650 class, ≥ 4000 mAh), otherwise 1000 mA
///   (18650 class). For lithium cells the reading does track size. Above 300 mΩ refused.
///
/// Below the ceiling, the current keeps the voltage across the internal resistance near
/// 100 mV, so small or worn cells get less. Floors: 200 mA nickel, 300 mA lithium. The
/// result is rounded down to 50 mA.
pub fn smart_current_ma(
    chemistry: BatteryChemistry,
    resistance_mohm: u16,
) -> Result<SmartCurrent, SmartRefusal> {
    if resistance_mohm == 0 {
        return Err(SmartRefusal::NotMeasured);
    }
    let nickel = is_nickel(chemistry);
    let (min, max, max_resistance) = if nickel {
        (200, 400, NICKEL_MAX_MOHM)
    } else if resistance_mohm < LARGE_LITHIUM_MAX_MOHM {
        (300, 2000, LITHIUM_MAX_MOHM)
    } else {
        (300, 1000, LITHIUM_MAX_MOHM)
    };
    if resistance_mohm > max_resistance {
        return Err(SmartRefusal::TooResistive { resistance_mohm });
    }
    let worn = nickel && resistance_mohm > NICKEL_WORN_MOHM;
    let current = if worn {
        min
    } else {
        (MAX_IR_DROP_MV * 1000 / u32::from(resistance_mohm)).clamp(min, max)
    };
    Ok(SmartCurrent {
        current_ma: (current / 50 * 50) as u16,
        worn,
    })
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
