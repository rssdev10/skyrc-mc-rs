//! Charging rules shared by every front end: chemistry detection for unattended charging,
//! Auto's fixed current, and Smart's probe and current choice.
//!
//! - **Auto**: detect the chemistry from the resting voltage and charge at
//!   [`AUTO_CURRENT_MA`] (500 mA).
//! - **Smart**: detect the chemistry, charge briefly at [`probe_current_ma`] until the charger
//!   reports the internal resistance (at least [`SMART_PROBE_SETTLE_S`] into the program,
//!   at most [`SMART_PROBE_TIMEOUT_S`]), then charge at [`smart_current_ma`] or stop the cell.
//!
//! Voltage alone cannot tell several chemistries apart, so detection only commits where it
//! is unambiguous and never picks a chemistry whose target voltage could overcharge a
//! lower-voltage cell: Li-Ion HV is never chosen.

use crate::bluetooth::BatteryChemistry;

/// At or above this resting voltage a cell is taken for Li-Ion (4.20 V target).
/// LiFePO4 never rests this high, and Li-Ion HV cannot be told apart, so it is not chosen.
pub const LI_ION_MIN_V: f32 = 2.7;
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
    Ambiguous { voltage: f32 },
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

/// Detection for unattended charging: Li-Ion at or above 2.7 V, NiMH below 1.5 V; any
/// other voltage is skipped as ambiguous.
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

fn is_nickel(chemistry: BatteryChemistry) -> bool {
    matches!(
        chemistry,
        BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop
    )
}

/// Auto's charge current, the same for every chemistry: a moderate fixed current for
/// starting right away. Smart measures the cell first instead.
pub const AUTO_CURRENT_MA: u16 = 500;

/// Current of Smart's measuring probe: low enough for the smallest cell of the family.
pub fn probe_current_ma(chemistry: BatteryChemistry) -> u16 {
    if is_nickel(chemistry) {
        200
    } else {
        300
    }
}

/// Smart reads the resistance only once the probe has run this long (seconds of the
/// charger's own program time), so the reading has settled.
pub const SMART_PROBE_SETTLE_S: u32 = 5;
/// Smart gives up waiting for a resistance reading after this long (seconds); the cell then
/// keeps charging at the probe current.
pub const SMART_PROBE_TIMEOUT_S: u64 = 25;

/// Voltage the charge current may add across the cell's internal resistance.
const MAX_IR_DROP_MV: u32 = 100;

/// Below this resistance a lithium cell is taken for a large (21700/26650-class) cell.
/// Measured on a live charger: a high-current 21700 read 17 mΩ, an 18650 30-33 mΩ.
pub const LARGE_LITHIUM_MAX_MOHM: u16 = 25;

/// Above this resistance a nickel cell is charged only at the minimum current and flagged
/// as worn. Measured on a live charger: healthy-ish AA/AAA cells read 130–490 mΩ, worn cells
/// 548–660 mΩ (the reading includes contact resistance).
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
///   and reflects condition more than size, so it never raises the nickel current. Cells
///   above 500 mΩ are charged at the 200 mA minimum and flagged as worn; above 1000 mΩ
///   they are refused;
/// - lithium: 2000 mA below 25 mΩ (21700/26650 class), otherwise 1000 mA (18650 class).
///   For lithium cells the reading does track size. Above 300 mΩ refused.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_never_picks_li_ion_hv_and_skips_ambiguous_voltages() {
        for v in [3.7, 3.85, 3.95, 4.1, 4.3] {
            assert_eq!(detect_chemistry(v), Ok(BatteryChemistry::LiIon), "{v} V");
        }
        assert_eq!(detect_chemistry(1.2), Ok(BatteryChemistry::NiMH));
        for v in [1.5, 1.7, 2.4] {
            assert!(matches!(detect_chemistry(v), Err(SkipReason::Ambiguous { .. })), "{v} V");
        }
        assert_eq!(detect_chemistry(0.0), Err(SkipReason::Empty));
    }

    /// The cells measured on a live charger and what Smart chose for them.
    #[test]
    fn smart_currents_for_measured_cells() {
        let pick = |c, r| smart_current_ma(c, r).map(|s| (s.current_ma, s.worn));
        assert_eq!(pick(BatteryChemistry::LiIon, 30), Ok((1000, false)), "18650");
        assert_eq!(pick(BatteryChemistry::LiIon, 17), Ok((2000, false)), "21700");
        assert_eq!(pick(BatteryChemistry::NiMH, 336), Ok((250, false)), "AA");
        assert_eq!(pick(BatteryChemistry::NiMH, 497), Ok((200, false)), "AA");
        assert_eq!(pick(BatteryChemistry::NiMH, 646), Ok((200, true)), "worn AAA");
        assert_eq!(pick(BatteryChemistry::NiMH, 170), Ok((400, false)), "AAA never above 400");
        assert_eq!(
            pick(BatteryChemistry::NiMH, 1200),
            Err(SmartRefusal::TooResistive { resistance_mohm: 1200 })
        );
        assert_eq!(pick(BatteryChemistry::LiIon, 0), Err(SmartRefusal::NotMeasured));
    }
}
