use csv::Writer;
use std::fs::File;
use std::path::Path;
use std::io::Write;
use thiserror::Error;

use crate::data::{MeasurementPoint, TimeAlignedRow};

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("File I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("CSV serialization error: {0}")]
    CsvError(#[from] csv::Error),
    #[error("No data to export")]
    NoData,
}

/// Export mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportMode {
    /// Time-aligned format: all slots in one row, averaged per second (default)
    #[default]
    TimeAligned,
    /// Raw format: all individual samples with full detail
    AllSamples,
}

/// Export format trait for different output formats
pub trait ExportFormat {
    fn write_header<W: Write>(&self, writer: &mut W) -> Result<(), ExportError>;
    fn write_measurement<W: Write>(&self, writer: &mut W, measurement: &MeasurementPoint) -> Result<(), ExportError>;
    fn finalize<W: Write>(&self, writer: &mut W) -> Result<(), ExportError>;
}

/// CSV format exporter with full measurement details (all samples mode)
pub struct CsvFormat {
    include_header: bool,
}

impl CsvFormat {
    pub fn new() -> Self {
        Self { include_header: true }
    }
}

/// Time-aligned CSV format exporter (all slots per row, averaged by second)
pub struct TimeAlignedCsvFormat {
    include_header: bool,
}

impl TimeAlignedCsvFormat {
    pub fn new() -> Self {
        Self { include_header: true }
    }
    
    pub fn write_header_for_aligned<W: Write>(&self, writer: &mut W) -> Result<(), ExportError> {
        if self.include_header {
            let mut csv_writer = Writer::from_writer(writer);
            csv_writer.write_record(&[
                "timestamp",
                // Slot 1
                "slot1_current_mA", "slot1_voltage_V", "slot1_state", "slot1_mode", "slot1_resistance_mOhm", "slot1_elapsed_time_sec",
                // Slot 2
                "slot2_current_mA", "slot2_voltage_V", "slot2_state", "slot2_mode", "slot2_resistance_mOhm", "slot2_elapsed_time_sec",
                // Slot 3
                "slot3_current_mA", "slot3_voltage_V", "slot3_state", "slot3_mode", "slot3_resistance_mOhm", "slot3_elapsed_time_sec",
                // Slot 4
                "slot4_current_mA", "slot4_voltage_V", "slot4_state", "slot4_mode", "slot4_resistance_mOhm", "slot4_elapsed_time_sec",
            ])?;
            csv_writer.flush()?;
        }
        Ok(())
    }
    
    pub fn write_aligned_row<W: Write>(&self, writer: &mut W, row: &TimeAlignedRow) -> Result<(), ExportError> {
        let mut csv_writer = Writer::from_writer(writer);
        
        let mut record: Vec<String> = vec![
            row.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
        ];
        
        // Add data for each of the 4 slots (current_mA, voltage_V, state, mode, resistance_mOhm, elapsed_time_sec)
        for slot_idx in 0..4 {
            if let Some(data) = &row.slots[slot_idx] {
                record.push(format!("{:.1}", data.current * 1000.0)); // current in mA
                record.push(format!("{:.3}", data.voltage));
                record.push(data.state.clone());
                record.push(data.mode.clone());
                record.push(data.resistance_milliohm.to_string());
                record.push(data.elapsed_seconds.to_string());
            } else {
                // Empty slot - add empty values
                record.push(String::new());
                record.push(String::new());
                record.push(String::new());
                record.push(String::new());
                record.push(String::new());
                record.push(String::new());
            }
        }
        
        csv_writer.write_record(&record)?;
        csv_writer.flush()?;
        Ok(())
    }
}

impl ExportFormat for CsvFormat {
    fn write_header<W: Write>(&self, writer: &mut W) -> Result<(), ExportError> {
        if self.include_header {
            let mut csv_writer = Writer::from_writer(writer);
            csv_writer.write_record(&[
                "timestamp",
                "slot",
                "current_mA",
                "voltage_V",
                "state",
                "mode",
                "resistance_mOhm",
                "elapsed_time_sec"
            ])?;
            csv_writer.flush()?;
        }
        Ok(())
    }

    fn write_measurement<W: Write>(&self, writer: &mut W, measurement: &MeasurementPoint) -> Result<(), ExportError> {
        let mut csv_writer = Writer::from_writer(writer);
        csv_writer.write_record(&[
            measurement.timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            (measurement.slot_id.0 + 1).to_string(),  // 1-based slot numbering
            format!("{:.1}", measurement.current * 1000.0),  // Convert A to mA
            format!("{:.3}", measurement.voltage),
            measurement.state.clone(),
            measurement.mode.clone(),
            measurement.resistance_milliohm.to_string(),
            measurement.elapsed_seconds.to_string(),
        ])?;
        csv_writer.flush()?;
        Ok(())
    }

    fn finalize<W: Write>(&self, _writer: &mut W) -> Result<(), ExportError> {
        Ok(())
    }
}

/// Generic data exporter that can use different formats
pub struct DataExporter<F: ExportFormat> {
    format: F,
}

impl<F: ExportFormat> DataExporter<F> {
    pub fn new(format: F) -> Self {
        Self { format }
    }

    pub fn export_to_file<P: AsRef<Path>>(
        &self,
        path: P,
        measurements: &[MeasurementPoint],
    ) -> Result<(), ExportError> {
        // Filter out Empty slots
        let filtered_measurements: Vec<_> = measurements
            .iter()
            .filter(|m| m.state != "Empty")
            .collect();

        if filtered_measurements.is_empty() {
            return Err(ExportError::NoData);
        }

        let count = filtered_measurements.len();
        let file = File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);

        self.format.write_header(&mut writer)?;
        
        for measurement in filtered_measurements {
            self.format.write_measurement(&mut writer, measurement)?;
        }
        
        self.format.finalize(&mut writer)?;
        writer.flush()?;
        
        log::info!("Exported {} measurements (non-empty slots)", count);
        Ok(())
    }

    pub fn export_slot_to_file<P: AsRef<Path>>(
        &self,
        path: P,
        measurements: &[MeasurementPoint],
        slot_id: crate::slot::SlotId,
    ) -> Result<(), ExportError> {
        let slot_measurements: Vec<_> = measurements
            .iter()
            .filter(|m| m.slot_id.0 == slot_id.0 && m.state != "Empty")
            .collect();

        if slot_measurements.is_empty() {
            return Err(ExportError::NoData);
        }

        let count = slot_measurements.len();
        let file = File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);

        self.format.write_header(&mut writer)?;
        
        for measurement in &slot_measurements {
            self.format.write_measurement(&mut writer, measurement)?;
        }
        
        self.format.finalize(&mut writer)?;
        writer.flush()?;
        
        log::info!("Exported {} measurements for slot {} (non-empty)", count, slot_id.0);
        Ok(())
    }

    pub fn export_to_string(&self, measurements: &[MeasurementPoint]) -> Result<String, ExportError> {
        // Filter out Empty slots
        let filtered_measurements: Vec<_> = measurements
            .iter()
            .filter(|m| m.state != "Empty")
            .collect();

        if filtered_measurements.is_empty() {
            return Err(ExportError::NoData);
        }

        let mut output = Vec::new();
        
        self.format.write_header(&mut output)?;
        
        for measurement in &filtered_measurements {
            self.format.write_measurement(&mut output, measurement)?;
        }
        
        self.format.finalize(&mut output)?;

        String::from_utf8(output)
            .map_err(|e| ExportError::IoError(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
    }
}

// Type alias for backward compatibility
pub type CsvExporter = DataExporter<CsvFormat>;

// Helper function for creating a CsvExporter
impl Default for CsvExporter {
    fn default() -> Self {
        DataExporter::new(CsvFormat::new())
    }
}

/// Export time-aligned data to a file (all slots per row, averaged by second)
pub fn export_time_aligned<P: AsRef<Path>>(
    path: P,
    rows: &[TimeAlignedRow],
) -> Result<(), ExportError> {
    if rows.is_empty() {
        return Err(ExportError::NoData);
    }
    
    let format = TimeAlignedCsvFormat::new();
    let file = File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    
    format.write_header_for_aligned(&mut writer)?;
    
    for row in rows {
        // Skip rows where all slots are empty
        if row.slots.iter().all(|s| s.is_none()) {
            continue;
        }
        format.write_aligned_row(&mut writer, row)?;
    }
    
    writer.flush()?;
    log::info!("Exported {} time-aligned rows", rows.len());
    Ok(())
}