use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, BTreeMap};

use crate::slot::SlotId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementPoint {
    pub timestamp: DateTime<Utc>,
    pub slot_id: SlotId,
    pub voltage: f32,
    pub current: f32,
    pub state: String,
    pub mode: String,
    pub resistance_milliohm: u16,
    pub elapsed_seconds: u16,
}

impl MeasurementPoint {
    #[allow(dead_code)]
    pub fn new(slot_id: SlotId, voltage: f32, current: f32) -> Self {
        Self {
            timestamp: Utc::now(),
            slot_id,
            voltage,
            current,
            state: String::new(),
            mode: String::new(),
            resistance_milliohm: 0,
            elapsed_seconds: 0,
        }
    }
    
    pub fn new_full(
        slot_id: SlotId,
        voltage: f32,
        current: f32,
        state: String,
        mode: String,
        resistance_milliohm: u16,
        elapsed_seconds: u16,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            slot_id,
            voltage,
            current,
            state,
            mode,
            resistance_milliohm,
            elapsed_seconds,
        }
    }
}

/// Data for a single slot at a specific second (averaged if multiple samples)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlotSecondData {
    pub voltage: f32,
    pub current: f32,
    pub state: String,
    pub mode: String,
    pub resistance_milliohm: u16,
    pub elapsed_seconds: u16,
    pub sample_count: u32,
}

/// Time-aligned row containing all slots' data for a specific second
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeAlignedRow {
    pub timestamp: DateTime<Utc>,
    pub slots: [Option<SlotSecondData>; 4],
}

impl TimeAlignedRow {
    pub fn new(timestamp: DateTime<Utc>) -> Self {
        Self {
            timestamp,
            slots: Default::default(),
        }
    }
}

pub struct DataLogger {
    measurements: Vec<MeasurementPoint>,
    max_points: usize,
}

impl DataLogger {
    pub fn new() -> Self {
        Self {
            measurements: Vec::new(),
            max_points: 1_000_000, // Support up to 1 million samples
        }
    }

    pub fn add_measurement(&mut self, measurement: MeasurementPoint) {
        self.measurements.push(measurement);
        
        // Keep only the most recent measurements
        if self.measurements.len() > self.max_points {
            self.measurements.remove(0);
        }
    }
    
    /// Generate time-aligned data with all slots per row, averaged by second.
    /// Uses forward-fill: if a slot has no data for a specific second, 
    /// the last known value for that slot is carried forward.
    pub fn get_time_aligned_data(&self) -> Vec<TimeAlignedRow> {
        if self.measurements.is_empty() {
            return Vec::new();
        }

        // Group measurements by second (truncate to second precision)
        let mut by_second: BTreeMap<i64, HashMap<u8, Vec<&MeasurementPoint>>> = BTreeMap::new();
        
        for m in &self.measurements {
            let second_ts = m.timestamp.timestamp();
            let slot_idx = m.slot_id.0 as u8;
            by_second
                .entry(second_ts)
                .or_default()
                .entry(slot_idx)
                .or_default()
                .push(m);
        }
        
        // Track last known values for each slot (for forward-fill)
        let mut last_known: [Option<SlotSecondData>; 4] = Default::default();
        
        // Convert to time-aligned rows with averaging and forward-fill
        let mut rows = Vec::with_capacity(by_second.len());
        
        for (ts, slot_data) in by_second {
            let base_time = DateTime::from_timestamp(ts, 0)
                .unwrap_or_else(Utc::now);
            
            let mut row = TimeAlignedRow::new(base_time);
            
            // Process slots that have data this second
            for (slot_idx, measurements) in slot_data {
                if slot_idx < 4 && !measurements.is_empty() {
                    let count = measurements.len() as f32;
                    let avg_voltage: f32 = measurements.iter().map(|m| m.voltage).sum::<f32>() / count;
                    let avg_current: f32 = measurements.iter().map(|m| m.current).sum::<f32>() / count;
                    let avg_resistance: u16 = (measurements.iter()
                        .map(|m| m.resistance_milliohm as u32)
                        .sum::<u32>() / count as u32) as u16;
                    
                    // Use the most recent state, mode, and elapsed_seconds
                    let last = measurements.last().unwrap();
                    
                    let sd = SlotSecondData {
                        voltage: avg_voltage,
                        current: avg_current,
                        state: last.state.clone(),
                        mode: last.mode.clone(),
                        resistance_milliohm: avg_resistance,
                        elapsed_seconds: last.elapsed_seconds,
                        sample_count: measurements.len() as u32,
                    };
                    
                    // Update last known and set in row
                    last_known[slot_idx as usize] = Some(sd.clone());
                    row.slots[slot_idx as usize] = Some(sd);
                }
            }
            
            // Forward-fill: for slots without data this second, use last known value
            for (slot_idx, last_data_opt) in last_known.iter().enumerate() {
                if row.slots[slot_idx].is_none() {
                    if let Some(ref last_data) = last_data_opt {
                        row.slots[slot_idx] = Some(last_data.clone());
                    }
                }
            }
            
            rows.push(row);
        }
        
        rows
    }

    pub fn get_measurements_for_slot(&self, slot_id: SlotId) -> Vec<&MeasurementPoint> {
        self.measurements
            .iter()
            .filter(|m| m.slot_id.0 == slot_id.0)
            .collect()
    }

    pub fn get_all_measurements(&self) -> &[MeasurementPoint] {
        &self.measurements
    }

    pub fn get_recent_measurements(&self, count: usize) -> &[MeasurementPoint] {
        let start = if self.measurements.len() > count {
            self.measurements.len() - count
        } else {
            0
        };
        &self.measurements[start..]
    }

    #[allow(dead_code)]
    pub fn get_measurements_since(&self, since: DateTime<Utc>) -> Vec<&MeasurementPoint> {
        self.measurements
            .iter()
            .filter(|m| m.timestamp >= since)
            .collect()
    }

    pub fn clear(&mut self) {
        self.measurements.clear();
    }

    #[allow(dead_code)]
    pub fn clear_slot(&mut self, slot_id: SlotId) {
        self.measurements.retain(|m| m.slot_id.0 != slot_id.0);
    }

    pub fn get_statistics(&self) -> DataStatistics {
        if self.measurements.is_empty() {
            return DataStatistics::default();
        }

        let mut slot_stats: HashMap<SlotId, SlotStatistics> = HashMap::new();

        for measurement in &self.measurements {
            let slot_stat = slot_stats.entry(measurement.slot_id).or_insert_with(|| SlotStatistics {
                slot_id: measurement.slot_id,
                measurement_count: 0,
                min_voltage: measurement.voltage,
                max_voltage: measurement.voltage,
                avg_voltage: 0.0,
                min_current: measurement.current,
                max_current: measurement.current,
                avg_current: 0.0,
                total_voltage: 0.0,
                total_current: 0.0,
            });

            slot_stat.measurement_count += 1;
            slot_stat.total_voltage += measurement.voltage;
            slot_stat.total_current += measurement.current;
            slot_stat.min_voltage = slot_stat.min_voltage.min(measurement.voltage);
            slot_stat.max_voltage = slot_stat.max_voltage.max(measurement.voltage);
            slot_stat.min_current = slot_stat.min_current.min(measurement.current);
            slot_stat.max_current = slot_stat.max_current.max(measurement.current);
        }

        // Calculate averages
        for slot_stat in slot_stats.values_mut() {
            slot_stat.avg_voltage = slot_stat.total_voltage / slot_stat.measurement_count as f32;
            slot_stat.avg_current = slot_stat.total_current / slot_stat.measurement_count as f32;
        }

        DataStatistics {
            total_measurements: self.measurements.len(),
            slots: slot_stats,
            oldest_measurement: self.measurements.first().map(|m| m.timestamp),
            newest_measurement: self.measurements.last().map(|m| m.timestamp),
        }
    }
}

#[derive(Debug, Clone)]
#[derive(Default)]
pub struct DataStatistics {
    pub total_measurements: usize,
    #[allow(dead_code)]
    pub slots: HashMap<SlotId, SlotStatistics>,
    pub oldest_measurement: Option<DateTime<Utc>>,
    pub newest_measurement: Option<DateTime<Utc>>,
}


#[derive(Debug, Clone)]
pub struct SlotStatistics {
    #[allow(dead_code)]
    pub slot_id: SlotId,
    pub measurement_count: usize,
    pub min_voltage: f32,
    pub max_voltage: f32,
    pub avg_voltage: f32,
    pub min_current: f32,
    pub max_current: f32,
    pub avg_current: f32,
    pub total_voltage: f32,
    pub total_current: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    
    #[test]
    fn test_forward_fill_carries_last_known_values() {
        let mut logger = DataLogger::new();
        
        let base = Utc.with_ymd_and_hms(2026, 1, 31, 20, 21, 21).unwrap();
        
        // Second 0: slot 0 and slot 3 report
        logger.add_measurement(MeasurementPoint {
            timestamp: base,
            slot_id: SlotId(0),
            voltage: 1.296, current: 0.0,
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0,
        });
        logger.add_measurement(MeasurementPoint {
            timestamp: base,
            slot_id: SlotId(3),
            voltage: 4.083, current: 0.0,
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0,
        });
        
        // Second 1: only slot 0 reports
        let t1 = base + chrono::Duration::seconds(1);
        logger.add_measurement(MeasurementPoint {
            timestamp: t1,
            slot_id: SlotId(0),
            voltage: 1.296, current: 0.0,
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0,
        });
        
        // Second 2: slot 0 and slot 1 report
        let t2 = base + chrono::Duration::seconds(2);
        logger.add_measurement(MeasurementPoint {
            timestamp: t2,
            slot_id: SlotId(0),
            voltage: 1.296, current: 0.0,
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0,
        });
        logger.add_measurement(MeasurementPoint {
            timestamp: t2,
            slot_id: SlotId(1),
            voltage: 1.341, current: 0.0,
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0,
        });
        
        // Second 3: only slot 0 reports
        let t3 = base + chrono::Duration::seconds(3);
        logger.add_measurement(MeasurementPoint {
            timestamp: t3,
            slot_id: SlotId(0),
            voltage: 1.296, current: 0.0,
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0,
        });
        
        let rows = logger.get_time_aligned_data();
        assert_eq!(rows.len(), 4);
        
        // Second 0: slot 0 and 3 present, slot 1 and 2 absent (never seen)
        assert!(rows[0].slots[0].is_some());
        assert!(rows[0].slots[1].is_none()); // never seen yet
        assert!(rows[0].slots[2].is_none()); // never seen
        assert!(rows[0].slots[3].is_some());
        
        // Second 1: slot 0 present, slot 3 must be forward-filled
        assert!(rows[1].slots[0].is_some());
        assert!(rows[1].slots[3].is_some(), "slot 3 should be forward-filled from second 0");
        assert!((rows[1].slots[3].as_ref().unwrap().voltage - 4.083).abs() < 0.001);
        
        // Second 2: slot 0 and 1 present, slot 3 forward-filled
        assert!(rows[2].slots[0].is_some());
        assert!(rows[2].slots[1].is_some());
        assert!(rows[2].slots[3].is_some(), "slot 3 should be forward-filled");
        
        // Second 3: only slot 0 new, but slot 1 AND slot 3 must be forward-filled 
        assert!(rows[3].slots[0].is_some());
        assert!(rows[3].slots[1].is_some(), "slot 1 should be forward-filled from second 2");
        assert!((rows[3].slots[1].as_ref().unwrap().voltage - 1.341).abs() < 0.001);
        assert!(rows[3].slots[3].is_some(), "slot 3 should be forward-filled from second 0");
        assert!((rows[3].slots[3].as_ref().unwrap().voltage - 4.083).abs() < 0.001);
    }
}