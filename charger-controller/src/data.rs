use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, BTreeMap};

use crate::slot::SlotId;

/// Where a measurement came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleSource {
    /// A 0x91 status read while connected.
    #[default]
    Live,
    /// The charger's own voltage history (0x98), read after the fact. Voltage only: the
    /// charger keeps no current history it can send.
    ChargerHistory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementPoint {
    pub timestamp: DateTime<Utc>,
    pub slot_id: SlotId,
    pub voltage: f32,
    /// mA; `None` when unknown (charger history).
    pub current: Option<f32>,
    pub state: String,
    pub mode: String,
    pub resistance_milliohm: u16,
    pub elapsed_seconds: u32,
    #[serde(default)]
    pub source: SampleSource,
}

impl MeasurementPoint {
    #[allow(dead_code)]
    pub fn new(slot_id: SlotId, voltage: f32, current: f32) -> Self {
        Self::new_full(slot_id, voltage, current, String::new(), String::new(), 0, 0)
    }

    pub fn new_full(
        slot_id: SlotId,
        voltage: f32,
        current: f32,
        state: String,
        mode: String,
        resistance_milliohm: u16,
        elapsed_seconds: u32,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            slot_id,
            voltage,
            current: Some(current),
            state,
            mode,
            resistance_milliohm,
            elapsed_seconds,
            source: SampleSource::Live,
        }
    }
}

/// Data for a single slot at a specific second (averaged if multiple samples)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlotSecondData {
    pub voltage: f32,
    /// mA; `None` when no sample of that second had a current.
    pub current: Option<f32>,
    pub state: String,
    pub mode: String,
    pub resistance_milliohm: u16,
    pub elapsed_seconds: u32,
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

/// Measurements per slot, each slot's kept in time order. The graph and both CSV exports
/// read this one buffer.
pub struct DataLogger {
    slots: [Vec<MeasurementPoint>; 4],
    max_points_per_slot: usize,
}

/// History samples closer than this to a live sample are not added: the live data is
/// finer and has the current.
const HISTORY_GAP_MIN_S: i64 = 10;

impl DataLogger {
    pub fn new() -> Self {
        Self {
            slots: Default::default(),
            max_points_per_slot: 250_000, // about 70 hours at one sample per second
        }
    }

    pub fn add_measurement(&mut self, measurement: MeasurementPoint) {
        let Some(points) = self.slots.get_mut(measurement.slot_id.0) else {
            return;
        };
        match points.last() {
            Some(last) if last.timestamp > measurement.timestamp => {
                let at = points.partition_point(|p| p.timestamp <= measurement.timestamp);
                points.insert(at, measurement);
            }
            _ => points.push(measurement),
        }
        if points.len() > self.max_points_per_slot {
            let excess = points.len() - self.max_points_per_slot;
            points.drain(..excess);
        }
    }

    /// Adds the charger's voltage history (0x98) of the program that started at
    /// `program_start` where live data is missing: before the first live sample (the app
    /// connected late) and in gaps (a reconnect). Sample `i` is placed at
    /// `program_start + (i + 1) * interval_s`. History added earlier for the same program is
    /// replaced. Returns how many samples were added.
    pub fn backfill_history(
        &mut self,
        slot_id: SlotId,
        program_start: DateTime<Utc>,
        interval_s: u8,
        samples_mv: &[u16],
    ) -> usize {
        let Some(points) = self.slots.get_mut(slot_id.0) else {
            return 0;
        };
        if interval_s == 0 {
            return 0;
        }
        points.retain(|p| !(p.source == SampleSource::ChargerHistory && p.timestamp >= program_start));
        let live: Vec<DateTime<Utc>> = points
            .iter()
            .filter(|p| p.timestamp >= program_start)
            .map(|p| p.timestamp)
            .collect();
        let gap = chrono::Duration::seconds(HISTORY_GAP_MIN_S.max(i64::from(interval_s)));
        let far_from_live = |t: DateTime<Utc>| {
            let at = live.partition_point(|&l| l < t);
            let near = |i: usize| live.get(i).is_some_and(|&l| (l - t).abs() < gap);
            !(near(at) || (at > 0 && near(at - 1)))
        };
        let mut added = 0;
        for (i, &mv) in samples_mv.iter().enumerate() {
            let elapsed = (i as u32 + 1) * u32::from(interval_s);
            let timestamp = program_start + chrono::Duration::seconds(i64::from(elapsed));
            if !far_from_live(timestamp) {
                continue;
            }
            points.push(MeasurementPoint {
                timestamp,
                slot_id,
                voltage: f32::from(mv) / 1000.0,
                current: None,
                state: String::new(),
                mode: String::new(),
                resistance_milliohm: 0,
                elapsed_seconds: elapsed,
                source: SampleSource::ChargerHistory,
            });
            added += 1;
        }
        points.sort_by_key(|p| p.timestamp); // stable: live order is kept
        added
    }

    /// Total number of measurements over all slots.
    pub fn len(&self) -> usize {
        self.slots.iter().map(Vec::len).sum()
    }

    /// All measurements in time order.
    fn all_in_time_order(&self) -> Vec<&MeasurementPoint> {
        let mut all: Vec<&MeasurementPoint> = self.slots.iter().flatten().collect();
        all.sort_by_key(|p| p.timestamp);
        all
    }

    /// Generate time-aligned data with all slots per row, averaged by second.
    /// Uses forward-fill: if a slot has no data for a specific second, 
    /// the last known value for that slot is carried forward.
    pub fn get_time_aligned_data(&self) -> Vec<TimeAlignedRow> {
        if self.len() == 0 {
            return Vec::new();
        }

        // Group measurements by second (truncate to second precision)
        let mut by_second: BTreeMap<i64, HashMap<u8, Vec<&MeasurementPoint>>> = BTreeMap::new();
        
        for m in self.slots.iter().flatten() {
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
                    let currents: Vec<f32> = measurements.iter().filter_map(|m| m.current).collect();
                    let avg_current = (!currents.is_empty())
                        .then(|| currents.iter().sum::<f32>() / currents.len() as f32);
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

    /// The slot's measurements, oldest first.
    pub fn get_measurements_for_slot(&self, slot_id: SlotId) -> &[MeasurementPoint] {
        self.slots.get(slot_id.0).map_or(&[], Vec::as_slice)
    }

    /// Every measurement of every slot, in time order.
    pub fn get_all_measurements(&self) -> Vec<&MeasurementPoint> {
        self.all_in_time_order()
    }

    /// The newest `count` measurements of all slots, oldest first.
    pub fn get_recent_measurements(&self, count: usize) -> Vec<&MeasurementPoint> {
        let mut all = self.all_in_time_order();
        let start = all.len().saturating_sub(count);
        all.drain(..start);
        all
    }

    #[allow(dead_code)]
    pub fn get_measurements_since(&self, since: DateTime<Utc>) -> Vec<&MeasurementPoint> {
        self.all_in_time_order()
            .into_iter()
            .filter(|m| m.timestamp >= since)
            .collect()
    }

    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(Vec::clear);
    }

    #[allow(dead_code)]
    pub fn clear_slot(&mut self, slot_id: SlotId) {
        if let Some(points) = self.slots.get_mut(slot_id.0) {
            points.clear();
        }
    }

    pub fn get_statistics(&self) -> DataStatistics {
        if self.len() == 0 {
            return DataStatistics::default();
        }

        let mut slot_stats: HashMap<SlotId, SlotStatistics> = HashMap::new();

        for measurement in self.slots.iter().flatten() {
            let slot_stat = slot_stats.entry(measurement.slot_id).or_insert_with(|| SlotStatistics {
                slot_id: measurement.slot_id,
                measurement_count: 0,
                min_voltage: measurement.voltage,
                max_voltage: measurement.voltage,
                avg_voltage: 0.0,
                min_current: f32::INFINITY,
                max_current: f32::NEG_INFINITY,
                avg_current: 0.0,
                total_voltage: 0.0,
                total_current: 0.0,
                current_count: 0,
            });

            slot_stat.measurement_count += 1;
            slot_stat.total_voltage += measurement.voltage;
            slot_stat.min_voltage = slot_stat.min_voltage.min(measurement.voltage);
            slot_stat.max_voltage = slot_stat.max_voltage.max(measurement.voltage);
            if let Some(current) = measurement.current {
                slot_stat.current_count += 1;
                slot_stat.total_current += current;
                slot_stat.min_current = slot_stat.min_current.min(current);
                slot_stat.max_current = slot_stat.max_current.max(current);
            }
        }

        // Calculate averages
        for slot_stat in slot_stats.values_mut() {
            slot_stat.avg_voltage = slot_stat.total_voltage / slot_stat.measurement_count as f32;
            if slot_stat.current_count > 0 {
                slot_stat.avg_current = slot_stat.total_current / slot_stat.current_count as f32;
            } else {
                (slot_stat.min_current, slot_stat.max_current) = (0.0, 0.0);
            }
        }

        let ends = || self.slots.iter().flat_map(|p| [p.first(), p.last()]).flatten().map(|m| m.timestamp);
        DataStatistics {
            total_measurements: self.len(),
            slots: slot_stats,
            oldest_measurement: ends().min(),
            newest_measurement: ends().max(),
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
    /// Measurements with a known current (charger history has none).
    pub current_count: usize,
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
            voltage: 1.296, current: Some(0.0),
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0, source: SampleSource::Live,
        });
        logger.add_measurement(MeasurementPoint {
            timestamp: base,
            slot_id: SlotId(3),
            voltage: 4.083, current: Some(0.0),
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0, source: SampleSource::Live,
        });
        
        // Second 1: only slot 0 reports
        let t1 = base + chrono::Duration::seconds(1);
        logger.add_measurement(MeasurementPoint {
            timestamp: t1,
            slot_id: SlotId(0),
            voltage: 1.296, current: Some(0.0),
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0, source: SampleSource::Live,
        });
        
        // Second 2: slot 0 and slot 1 report
        let t2 = base + chrono::Duration::seconds(2);
        logger.add_measurement(MeasurementPoint {
            timestamp: t2,
            slot_id: SlotId(0),
            voltage: 1.296, current: Some(0.0),
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0, source: SampleSource::Live,
        });
        logger.add_measurement(MeasurementPoint {
            timestamp: t2,
            slot_id: SlotId(1),
            voltage: 1.341, current: Some(0.0),
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0, source: SampleSource::Live,
        });
        
        // Second 3: only slot 0 reports
        let t3 = base + chrono::Duration::seconds(3);
        logger.add_measurement(MeasurementPoint {
            timestamp: t3,
            slot_id: SlotId(0),
            voltage: 1.296, current: Some(0.0),
            state: "Idle".into(), mode: "None".into(),
            resistance_milliohm: 0, elapsed_seconds: 0, source: SampleSource::Live,
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

    fn live_at(base: DateTime<Utc>, s: i64) -> MeasurementPoint {
        MeasurementPoint {
            timestamp: base + chrono::Duration::seconds(s),
            slot_id: SlotId(3),
            voltage: 4.2, current: Some(500.0),
            state: "Charging".into(), mode: "Charge".into(),
            resistance_milliohm: 30, elapsed_seconds: s as u32, source: SampleSource::Live,
        }
    }

    /// (seconds after `start`, source) of every slot-4 sample.
    fn sources(logger: &DataLogger, start: DateTime<Utc>) -> Vec<(i64, SampleSource)> {
        logger
            .get_measurements_for_slot(SlotId(3))
            .iter()
            .map(|m| ((m.timestamp - start).num_seconds(), m.source))
            .collect()
    }

    #[test]
    fn history_fills_what_live_data_misses_and_nothing_else() {
        let start = Utc.with_ymd_and_hms(2026, 11, 26, 10, 0, 0).unwrap();
        let mut logger = DataLogger::new();
        // Live from 100 s to 110 s, then a reconnect gap, then 200 s to 210 s.
        for s in (100..=110).chain(200..=210) {
            logger.add_measurement(live_at(start, s));
        }
        // 4 s history over 0..=240 s: samples at 4, 8, .., 240 s.
        let added = logger.backfill_history(SlotId(3), start, 4, &[4150; 60]);
        let points = sources(&logger, start);
        let history: Vec<i64> = points.iter().filter(|p| p.1 == SampleSource::ChargerHistory).map(|p| p.0).collect();
        assert_eq!(added, history.len());
        assert_eq!(history.first(), Some(&4), "from the start of the program");
        // At least 10 s from any live sample: 88 s is the last before the first one (100 s).
        assert_eq!(history.iter().filter(|&&t| t < 100).max(), Some(&88));
        assert!(history.iter().all(|&t| !(91..=119).contains(&t) && !(191..=219).contains(&t)),
            "history keeps its distance from live data: {history:?}");
        assert!(history.contains(&152), "the reconnect gap is filled");
        assert!(points.windows(2).all(|w| w[0].0 <= w[1].0), "time order");
        let live = points.iter().filter(|p| p.1 == SampleSource::Live).count();
        assert_eq!(live, 22, "live data untouched");
        let history_only = logger.get_measurements_for_slot(SlotId(3)).iter().find(|m| m.source == SampleSource::ChargerHistory).unwrap();
        assert_eq!(history_only.current, None);

        // Reading the history again replaces the earlier backfill instead of adding to it.
        let again = logger.backfill_history(SlotId(3), start, 4, &[4150; 60]);
        assert_eq!(again, added);
        assert_eq!(sources(&logger, start).len(), points.len());
    }

    #[test]
    fn measurements_stay_in_time_order_per_slot() {
        let start = Utc.with_ymd_and_hms(2026, 11, 26, 10, 0, 0).unwrap();
        let mut logger = DataLogger::new();
        logger.add_measurement(live_at(start, 5));
        logger.add_measurement(live_at(start, 1));
        logger.add_measurement(live_at(start, 3));
        let times: Vec<i64> = sources(&logger, start).iter().map(|p| p.0).collect();
        assert_eq!(times, [1, 3, 5]);
        assert_eq!(logger.len(), 3);
        assert_eq!(logger.get_all_measurements().len(), 3);
    }

    #[test]
    fn statistics_ignore_unknown_currents() {
        let start = Utc.with_ymd_and_hms(2026, 11, 26, 10, 0, 0).unwrap();
        let mut logger = DataLogger::new();
        logger.add_measurement(live_at(start, 100));
        logger.backfill_history(SlotId(3), start, 8, &[4100; 5]);
        let stats = logger.get_statistics();
        let slot = &stats.slots[&SlotId(3)];
        assert_eq!((slot.measurement_count, slot.current_count), (6, 1));
        assert_eq!(slot.avg_current, 500.0);
    }
}
