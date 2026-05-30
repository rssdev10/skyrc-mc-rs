use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use std::collections::BTreeSet;
use std::time::Duration;
use thiserror::Error;
use tokio::time;
use uuid::Uuid;

/// Check if debug output is enabled via MC5000_DEBUG env var
fn debug_enabled() -> bool {
    std::env::var("MC5000_DEBUG").is_ok()
}

/// Check if verbose output is enabled via MC5000_VERBOSE env var
fn verbose_enabled() -> bool {
    std::env::var("MC5000_VERBOSE").is_ok() || debug_enabled()
}

/// Print debug message if debugging is enabled
macro_rules! debug_print {
    ($($arg:tt)*) => {
        if debug_enabled() {
            println!($($arg)*);
        }
    };
}

/// Print verbose message if verbose mode is enabled
macro_rules! verbose_print {
    ($($arg:tt)*) => {
        if verbose_enabled() {
            println!("[VERBOSE] {}", format!($($arg)*));
        }
    };
}

#[derive(Debug, Error)]
pub enum BluetoothError {
    #[error("Bluetooth adapter not found")]
    AdapterNotFound,
    #[error("Device not found: {0}")]
    DeviceNotFound(String),
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Communication error: {0}")]
    CommunicationError(String),
    #[error("Service not found: {0}")]
    ServiceNotFound(String),
    #[error("Characteristic not found: {0}")]
    CharacteristicNotFound(String),
    #[error("Bluetooth error: {0}")]
    BluetoothError(String),
}

impl From<btleplug::Error> for BluetoothError {
    fn from(error: btleplug::Error) -> Self {
        BluetoothError::BluetoothError(error.to_string())
    }
}


#[derive(Debug, Clone)]
pub struct DiscoveredBluetoothDevice {
    pub id: String,
    pub name: String,
    pub address: String,
    pub rssi: Option<i16>,
    pub is_mc5000: bool,
    pub peripheral: Peripheral,
}

#[derive(Debug, Clone)]
pub struct MC5000Protocol {
    peripheral: Option<Peripheral>,
    service_uuid: Option<Uuid>,
    command_char: Option<Uuid>,
    response_char: Option<Uuid>,
    notify_char: Option<Uuid>,
}

// MC5000 specific UUIDs (reverse-engineered from BLE traffic, confirmed captures 3-4)
#[allow(dead_code)]
const MC5000_SERVICE_UUID: &str = "0000ffe0-0000-1000-8000-00805f9b34fb";
#[allow(dead_code)]
const MC5000_COMMAND_CHAR_UUID: &str = "0000ffe1-0000-1000-8000-00805f9b34fb";
#[allow(dead_code)]
const MC5000_RESPONSE_CHAR_UUID: &str = "0000ffe1-0000-1000-8000-00805f9b34fb";
#[allow(dead_code)]
const MC5000_NOTIFY_CHAR_UUID: &str = "0000ffe1-0000-1000-8000-00805f9b34fb";

// Protocol constants
const PACKET_START: u8 = 0x0F;
const CMD_KEEPALIVE: u8 = 0x02;     // Small keep-alive/ack packet (0f 02 02 02)
const CMD_GREETING: u8 = 0x06;      // Device greeting (unsolicited)
const CMD_DEVICE_INFO: u8 = 0x57;   // Device handshake / info request
const CMD_VERSION: u8 = 0x74;       // Version info
const CMD_CHANNEL_STATUS: u8 = 0x91; // Channel status request
const CMD_START_STOP: u8 = 0x93;    // Start/stop charging command
const CMD_CHARGE_CONFIG: u8 = 0x94; // Charging configuration command
const CMD_SETTINGS: u8 = 0x65;      // Settings
#[allow(dead_code)]
const CMD_UNKNOWN_25: u8 = 0x25;    // Large opaque blob (observed once)
#[allow(dead_code)]
const CMD_UNKNOWN_EA: u8 = 0xEA;    // Large opaque blob (observed once)

/// High-level start/stop intent for 0x93 commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartStopAction {
    /// Stop all channels (0x00)
    StopAll,
    /// Start all channels (0x03)
    StartAll,
    /// Apply action to a channel bitmask (e.g., 0x02 to affect slot 2 only)
    ChannelMask(u8),
}

/// Operation mode for charging configuration (0x94 command, byte 4)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationMode {
    /// Normal charging mode (0x00)
    Charge,
    /// Storage mode: discharge/charge to storage voltage (0x01)
    Storage,
    /// Discharge only mode (0x02)
    Discharge,
    /// Cycle mode: charge/discharge cycles for capacity testing (0x03)
    Cycle,
    /// Refresh mode: deep cycle for NiMH/NiCd (0x04)
    Refresh,
    /// Break-in mode: conditioning cycles for new batteries (0x05)
    BreakIn,
}

impl OperationMode {
    /// Convert to protocol byte. The byte value depends on chemistry because
    /// NiMH/NiCd have Break-In and Refresh modes that shift the values.
    ///
    /// Li-Ion/LiFePO4: Charge=0x00, Storage=0x01, Discharge=0x02, Cycle=0x03
    /// NiMH/NiCd:     Charge=0x00, BreakIn=0x02, Discharge=0x03, Cycle=0x04, Refresh=0x05
    pub fn to_byte_for_chemistry(self, chemistry: BatteryChemistry) -> u8 {
        match chemistry {
            BatteryChemistry::NiMH | BatteryChemistry::NiCd |
            BatteryChemistry::Eneloop | BatteryChemistry::NiZn => match self {
                OperationMode::Charge => 0x00,
                OperationMode::Refresh => 0x01,
                OperationMode::BreakIn => 0x02,
                OperationMode::Discharge => 0x03,
                OperationMode::Cycle => 0x04,
                OperationMode::Storage => 0x01, // Not typical for NiMH
            },
            _ => match self {
                // Li-Ion, LiFePO4
                OperationMode::Charge => 0x00,
                OperationMode::Storage => 0x01,
                OperationMode::Discharge => 0x02,
                OperationMode::Cycle => 0x03,
                OperationMode::BreakIn => 0x02,  // Not available for Li-Ion, fallback
                OperationMode::Refresh => 0x03,  // Not available for Li-Ion, fallback
            },
        }
    }

    /// Legacy byte conversion (uses Li-Ion mapping for backward compatibility)
    pub fn to_byte(self) -> u8 {
        match self {
            OperationMode::Charge => 0x00,
            OperationMode::Storage => 0x01,
            OperationMode::Discharge => 0x02,
            OperationMode::Cycle => 0x03,
            OperationMode::Refresh => 0x04,
            OperationMode::BreakIn => 0x05,
        }
    }

    /// Parse operation mode from protocol byte (Li-Ion mapping)
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(OperationMode::Charge),
            0x01 => Some(OperationMode::Storage),
            0x02 => Some(OperationMode::Discharge),
            0x03 => Some(OperationMode::Cycle),
            0x04 => Some(OperationMode::Refresh),
            0x05 => Some(OperationMode::BreakIn),
            _ => None,
        }
    }

    /// Human-readable name
    pub fn name(self) -> &'static str {
        match self {
            OperationMode::Charge => "Charge",
            OperationMode::Storage => "Storage",
            OperationMode::Discharge => "Discharge",
            OperationMode::Cycle => "Cycle",
            OperationMode::Refresh => "Refresh",
            OperationMode::BreakIn => "Break-In",
        }
    }
}

impl std::fmt::Display for OperationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Battery chemistry type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryChemistry {
    /// Standard lithium-ion (4.2V)
    LiIon,
    /// High-voltage lithium-ion (4.35V)
    LiIonHV,
    /// Lithium iron phosphate (3.65V)
    LiFePO4,
    /// Nickel metal hydride (1.65V)
    NiMH,
    /// Nickel cadmium (1.65V)
    NiCd,
    /// eneloop NiMH (1.65V) — optimized program for eneloop batteries
    Eneloop,
    /// Nickel zinc (1.9V)
    NiZn,
    /// Rechargeable Alkaline Manganese (1.65V)
    RAM,
    /// Lithium titanate oxide (2.85V)
    LTO,
    /// Sodium-ion (4.0V)
    NaIon,
}

impl BatteryChemistry {
    /// Get standard target voltage in mV for this chemistry
    pub fn target_voltage_mv(self) -> u16 {
        match self {
            BatteryChemistry::LiIon => 4200,
            BatteryChemistry::LiIonHV => 4350,
            BatteryChemistry::LiFePO4 => 3650,
            BatteryChemistry::NiMH => 1650,
            BatteryChemistry::NiCd => 1650,
            BatteryChemistry::Eneloop => 1650,
            BatteryChemistry::NiZn => 1900,   // Confirmed series 5
            BatteryChemistry::RAM => 1650,
            BatteryChemistry::LTO => 2850,     // Confirmed series 5
            BatteryChemistry::NaIon => 4000,   // Confirmed series 5
        }
    }

    /// Get standard cutoff voltage in mV for this chemistry
    pub fn cutoff_voltage_mv(self) -> u16 {
        match self {
            BatteryChemistry::LiIon => 3200,
            BatteryChemistry::LiIonHV => 3400,
            BatteryChemistry::LiFePO4 => 2900,
            BatteryChemistry::NiMH => 900,
            BatteryChemistry::NiCd => 900,
            BatteryChemistry::Eneloop => 900,
            BatteryChemistry::NiZn => 1100,    // Confirmed series 5
            BatteryChemistry::RAM => 900,
            BatteryChemistry::LTO => 1800,     // Confirmed series 5
            BatteryChemistry::NaIon => 2000,   // Confirmed series 5
        }
    }

    /// Check if this chemistry uses delta-peak detection for charge termination.
    /// NiZn uses CV termination (not delta-peak), despite being nickel-based.
    pub fn uses_delta_peak(self) -> bool {
        matches!(self, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop)
    }

    /// Protocol byte for the chemistry field in 0x94 config command (data byte 32).
    ///
    /// Confirmed from series 5 capture (all 10 chemistries tested):
    /// - 0x00 = Li-Ion, 0x01 = Li-Ion HV, 0x02 = LiFePO4
    /// - 0x03 = NiMH, 0x04 = NiCd, 0x05 = eneloop
    /// - 0x06 = NiZn, 0x07 = RAM, 0x08 = LTO, 0x09 = Na-Ion
    pub fn to_protocol_byte(self) -> u8 {
        match self {
            BatteryChemistry::LiIon => 0x00,
            BatteryChemistry::LiIonHV => 0x01,
            BatteryChemistry::LiFePO4 => 0x02,
            BatteryChemistry::NiMH => 0x03,
            BatteryChemistry::NiCd => 0x04,
            BatteryChemistry::Eneloop => 0x05,
            BatteryChemistry::NiZn => 0x06,
            BatteryChemistry::RAM => 0x07,
            BatteryChemistry::LTO => 0x08,
            BatteryChemistry::NaIon => 0x09,
        }
    }

    /// Check if this is a nickel-based or alkaline chemistry.
    /// For these chemistries, the secondary value field = 110% of rated capacity.
    /// For lithium-class chemistries (Li-Ion, LiHV, LiFePO4, LTO, Na-Ion), it's the storage voltage.
    pub fn is_nickel_based(self) -> bool {
        matches!(self, BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop
                     | BatteryChemistry::NiZn | BatteryChemistry::RAM)
    }

    /// Parse from protocol byte (confirmed series 5 capture)
    pub fn from_protocol_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(BatteryChemistry::LiIon),
            0x01 => Some(BatteryChemistry::LiIonHV),
            0x02 => Some(BatteryChemistry::LiFePO4),
            0x03 => Some(BatteryChemistry::NiMH),
            0x04 => Some(BatteryChemistry::NiCd),
            0x05 => Some(BatteryChemistry::Eneloop),
            0x06 => Some(BatteryChemistry::NiZn),
            0x07 => Some(BatteryChemistry::RAM),
            0x08 => Some(BatteryChemistry::LTO),
            0x09 => Some(BatteryChemistry::NaIon),
            _ => None,
        }
    }

    /// Get standard storage voltage in mV for lithium-class chemistries.
    /// For nickel/alkaline chemistries this value is not used (cap2 = 110% capacity).
    /// Confirmed from capture 4: Li-Ion=3800, LiHV=3900, LiFePO4=3300.
    /// Confirmed from series 5: LTO=2400, Na-Ion=3500.
    pub fn storage_voltage_mv(self) -> Option<u16> {
        match self {
            BatteryChemistry::LiIon => Some(3800),
            BatteryChemistry::LiIonHV => Some(3900),
            BatteryChemistry::LiFePO4 => Some(3300),
            BatteryChemistry::LTO => Some(2400),    // Confirmed series 5
            BatteryChemistry::NaIon => Some(3500),   // Confirmed series 5
            _ => None,
        }
    }

    /// Human-readable name
    pub fn name(self) -> &'static str {
        match self {
            BatteryChemistry::LiIon => "Li-Ion",
            BatteryChemistry::LiIonHV => "Li-Ion HV",
            BatteryChemistry::LiFePO4 => "LiFePO4",
            BatteryChemistry::NiMH => "NiMH",
            BatteryChemistry::NiCd => "NiCd",
            BatteryChemistry::Eneloop => "eneloop",
            BatteryChemistry::NiZn => "NiZn",
            BatteryChemistry::RAM => "RAM",
            BatteryChemistry::LTO => "LTO",
            BatteryChemistry::NaIon => "Na-Ion",
        }
    }
}

impl Default for MC5000Protocol {
    fn default() -> Self {
        Self::new()
    }
}

impl MC5000Protocol {
    pub fn new() -> Self {
        Self {
            peripheral: None,
            service_uuid: None,
            command_char: None,
            response_char: None,
            notify_char: None,
        }
    }

    /// Calculate sum checksum for MC5000 protocol packets (sum of all bytes mod 256)
    fn calculate_checksum(data: &[u8]) -> u8 {
        data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
    }

    /// Build a protocol packet: [0x0F, length, command, data..., checksum]
    /// Length field = number of bytes after length (cmd + data + checksum)
    /// Checksum = sum of bytes from command to end of data, mod 256
    pub fn build_packet(command: u8, data: &[u8]) -> Vec<u8> {
        let length = 2 + data.len(); // command + data + checksum
        let mut packet = Vec::with_capacity(2 + length);
        packet.push(PACKET_START);
        packet.push(length as u8);
        packet.push(command);
        packet.extend_from_slice(data);
        // Checksum is sum of bytes from index 2 (command) onwards
        let checksum = Self::calculate_checksum(&packet[2..]);
        packet.push(checksum);
        packet
    }

    /// Build device info handshake command (0x57)
    /// Exact format from captured traffic: 0f 13 57 00 ff 34 37 38 c1 a4 00 00 00 00 00 00 00 00 00 00 00 5e
    pub fn build_device_info_command(bt_suffix: &str) -> Vec<u8> {
        // Build the exact packet format seen in captures
        // Total: 22 bytes: start(1) + length(1) + cmd(1) + data(18) + checksum(1)
        // Length field = 0x13 = 19 = cmd(1) + data(17) + checksum(1)
        let mut packet = Vec::with_capacity(22);
        
        packet.push(PACKET_START);  // 0x0F
        packet.push(0x13);          // Length = 19 (cmd + 17 data bytes + checksum)
        packet.push(CMD_DEVICE_INFO); // 0x57
        packet.push(0x00);          // Subcommand
        
        // Add BT address bytes from suffix like "FF34"
        if bt_suffix.len() >= 4 {
            if let Ok(b1) = u8::from_str_radix(&bt_suffix[0..2], 16) {
                packet.push(b1);
            } else {
                packet.push(0xFF);
            }
            if let Ok(b2) = u8::from_str_radix(&bt_suffix[2..4], 16) {
                packet.push(b2);
            } else {
                packet.push(0x34);
            }
        } else {
            packet.push(0xFF);
            packet.push(0x34);
        }
        
        // Fixed bytes from capture (possibly app identifier or magic values)
        packet.extend_from_slice(&[0x37, 0x38, 0xc1, 0xa4]);
        
        // Padding - need 11 zeros to reach 21 bytes before checksum
        // We have: start(1) + len(1) + cmd(1) + sub(1) + addr(2) + magic(4) = 10 bytes
        // Need 21 bytes before checksum, so add 11 zeros
        while packet.len() < 21 {
            packet.push(0x00);
        }
        
        // Calculate checksum: sum of bytes from index 2 (cmd) to end, mod 256
        let checksum = Self::calculate_checksum(&packet[2..]);
        packet.push(checksum);
        
        packet
    }

    /// Build channel status request command (0x91)
    pub fn build_channel_status_command(channel_bitmask: u8) -> Vec<u8> {
        Self::build_packet(CMD_CHANNEL_STATUS, &[channel_bitmask])
    }

    /// Build version request command (0x74)  
    pub fn build_version_command() -> Vec<u8> {
        Self::build_packet(CMD_VERSION, &[0x00, 0x00, 0x00, 0x00, 0x00])
    }

    /// Build settings query command (0x65)
    pub fn build_settings_command() -> Vec<u8> {
        Self::build_packet(CMD_SETTINGS, &[0x00, 0x00])
    }

    /// Build 0xFE slot query command (per-slot status check, confirmed capture 4)
    /// channel: slot bitmask (0x01-0x08) or 0x00 for general query
    pub fn build_fe_query_command_for(channel: u8) -> Vec<u8> {
        Self::build_packet(0xFE, &[channel])
    }

    /// Build 0xFE general query command (channel 0x00)
    pub fn build_fe_query_command() -> Vec<u8> {
        Self::build_packet(0xFE, &[0x00])
    }

    /// Build start/stop charging command (0x93)
    ///
    /// Observed actions:
    /// - 0x00: stop all channels
    /// - 0x03: start all channels
    /// - Bitmask (e.g., 0x02) to act on a single slot when not affecting all
    pub fn build_start_stop_command(action: StartStopAction) -> Vec<u8> {
        let byte = match action {
            StartStopAction::StopAll => 0x00,
            StartStopAction::StartAll => 0x03,
            StartStopAction::ChannelMask(mask) => mask,
        };

        Self::build_packet(CMD_START_STOP, &[byte])
    }

    /// Build keep-alive/ping command (0x02). Observed as 0f 02 02 02.
    pub fn build_keepalive_command() -> Vec<u8> {
        Self::build_packet(CMD_KEEPALIVE, &[])
    }

    /// Build charging configuration command (0x94)
    /// Sets up charging parameters for a slot.
    ///
    /// Field layout CORRECTED from capture 4 binary analysis.
    /// Key correction: data[22] is single-byte delta-peak (not 2-byte LE),
    /// data[23] is trickle/10, data[24-25] is keep voltage BE,
    /// data[26] is constant 0x3C, data[27-28] is timer BE.
    pub fn build_charge_config_command(config: &ChargeConfig) -> Vec<u8> {
        let mut data = Vec::with_capacity(41);
        
        // Byte 0: Channel bitmask (0x01/0x02/0x04/0x08, or 0x00 for all slots)
        data.push(config.channel_bitmask);
        
        // Byte 1: Mode (chemistry-dependent byte mapping)
        data.push(config.mode.to_byte_for_chemistry(config.chemistry));
        
        // Bytes 2-3: Charge current (mA, big-endian)
        data.push((config.charge_current_ma >> 8) as u8);
        data.push((config.charge_current_ma & 0xFF) as u8);
        
        // Bytes 4-5: Discharge current (mA, big-endian)
        data.push((config.discharge_current_ma >> 8) as u8);
        data.push((config.discharge_current_ma & 0xFF) as u8);
        
        // Bytes 6-7: Capacity (mAh, big-endian)
        data.push((config.capacity_mah >> 8) as u8);
        data.push((config.capacity_mah & 0xFF) as u8);
        
        // Bytes 8-9: CV/Target voltage (mV, big-endian)
        // For Storage mode in capture 4: this is the charge LIMIT (e.g. 4200mV for Li-Ion),
        // NOT the storage voltage. Storage voltage goes in the secondary value field (data[33-34]).
        data.push((config.target_voltage_mv >> 8) as u8);
        data.push((config.target_voltage_mv & 0xFF) as u8);
        
        // Bytes 10-11: Cutoff voltage (mV, big-endian)
        data.push((config.cutoff_voltage_mv >> 8) as u8);
        data.push((config.cutoff_voltage_mv & 0xFF) as u8);
        
        // Bytes 12-13: Charge cutoff current (mA, big-endian)
        data.push((config.charge_cutoff_current_ma >> 8) as u8);
        data.push((config.charge_cutoff_current_ma & 0xFF) as u8);
        
        // Bytes 14-15: Discharge cutoff current (mA, big-endian)
        data.push((config.discharge_cutoff_current_ma >> 8) as u8);
        data.push((config.discharge_cutoff_current_ma & 0xFF) as u8);
        
        // Bytes 16-17: Charge resting time (minutes, big-endian)
        //   Time to pause between charge→discharge phases in cycle modes.
        //   Confirmed series 5: 10-15 min typical, 0 when not cycling.
        data.push((config.charge_resting_min >> 8) as u8);
        data.push((config.charge_resting_min & 0xFF) as u8);
        
        // Bytes 18-19: Discharge resting time (minutes, big-endian)
        //   Time to pause between discharge→charge phases in cycle modes.
        //   Confirmed series 5: 10-11 min typical, 0 when not cycling.
        data.push((config.discharge_resting_min >> 8) as u8);
        data.push((config.discharge_resting_min & 0xFF) as u8);
        
        // Byte 20: Cycle count (number of charge/discharge cycles)
        //   Default 1 in most configs. Observed 2 in NiZn Cycle (series 5).
        data.push(config.cycle_count);

        // Byte 21: Cycle direction flag (confirmed series 5)
        //   0x00 = C→D (charge first, default)
        //   0x01 = D→C (discharge first, confirmed capture 3)
        //   0x02 = C→D→C (charge, discharge, charge)
        //   0x03 = D→C→D (discharge, charge, discharge)
        data.push(config.cycle_direction);
        
        // Byte 22: Delta-peak voltage (mV, single byte)
        //   CORRECTED: Was 2-byte LE in old code. Capture 4 Config #3 proves single byte:
        //   0x06 at data[22] + 0x05 at data[23] → old LE would give 0x0506=1286mV (absurd).
        //   Typical: 6mV for NiMH/NiCd, 0 for lithium.
        data.push(config.delta_peak_mv as u8);
        
        // Byte 23: Trickle charge current (×10 mA, single byte)
        //   0x05 = 50mA trickle for NiMH. 0x00 when unused.
        //   CORRECTED: Was part of delta-peak LE pair in old code.
        data.push((config.trickle_charge_ma / 10) as u8);
        
        // Bytes 24-25: Keep/float voltage (mV, big-endian)
        //   NiMH example: 0x0514 = 1300mV (1.3V float). 0x0000 when unused.
        //   CORRECTED: Was "reserved 0x0000" in old code.
        data.push((config.keep_voltage_mv >> 8) as u8);
        data.push((config.keep_voltage_mv & 0xFF) as u8);
        
        // Byte 26: Always 0x3C (= 60 decimal) in all capture 4 configs.
        //   CORRECTED: Was low byte of LE cutoff_timer in old code. The coincidental value
        //   60 (0x3C) in simple configs caused misidentification as LE timer.
        data.push(0x3C);
        
        // Bytes 27-28: Cutoff timer (minutes, BIG-endian)
        //   CORRECTED: Was 2-byte LE at data[26-27] in old code. Capture 4 Config #5
        //   (Li-Ion storage, timer=120) has bytes 0x00,0x78 at data[27-28] → BE 0x0078=120 ✓.
        //   Config #3 (NiMH, timer=90) has 0x00,0x5A → BE 0x005A=90 ✓.
        data.push((config.cutoff_timer_min >> 8) as u8);
        data.push((config.cutoff_timer_min & 0xFF) as u8);
        
        // Bytes 29-30: Max time (minutes, BIG-endian)
        //   No reserved byte between timer and max_time (confirmed from capture 4).
        data.push((config.max_time_min >> 8) as u8);
        data.push((config.max_time_min & 0xFF) as u8);
        
        // Byte 31: Chemistry prefix (always 0x00)
        data.push(0x00);
        
        // Byte 32: Chemistry byte (confirmed series 5 — all 10 chemistries)
        //   0x00=Li-Ion, 0x01=LiHV, 0x02=LiFePO4, 0x03=NiMH,
        //   0x04=NiCd, 0x05=eneloop, 0x06=NiZn, 0x07=RAM,
        //   0x08=LTO, 0x09=Na-Ion
        data.push(config.chemistry.to_protocol_byte());
        
        // Bytes 33-34: Secondary value field (big-endian)
        //   For lithium chemistries: storage voltage in mV
        //     Li-Ion=3800, LiHV=3900, LiFePO4=3300 (confirmed capture 4)
        //   For nickel/alkaline chemistries: 110% of rated capacity in mAh
        //     3000mAh → 3300 (confirmed captures 3-4)
        let secondary_value = if config.chemistry.is_nickel_based() {
            (config.capacity_mah as f32 * 1.1) as u16
        } else {
            // Lithium: use storage voltage
            config.chemistry.storage_voltage_mv().unwrap_or(config.capacity_mah)
        };
        data.push((secondary_value >> 8) as u8);
        data.push((secondary_value & 0xFF) as u8);
        
        // Bytes 35-39: Padding (always zeros in captures, 5 bytes to reach 40 total data bytes)
        // Verified: capture packets are 44 bytes total = start(1) + length(1) + cmd(1) + data(40) + checksum(1)
        data.extend([0x00u8; 5]);
        
        debug_assert_eq!(data.len(), 40, "Config data must be exactly 40 bytes");
        
        Self::build_packet(CMD_CHARGE_CONFIG, &data)
    }

    /// Scan for available Bluetooth devices
    pub async fn scan_devices(timeout_secs: u64) -> Result<Vec<DiscoveredBluetoothDevice>, BluetoothError> {
        let manager = Manager::new().await?;
        let adapters = manager.adapters().await?;
        
        if adapters.is_empty() {
            return Err(BluetoothError::AdapterNotFound);
        }

        let central = &adapters[0];
        
        // Start scanning
        central.start_scan(ScanFilter::default()).await?;
        
        // Wait for scan timeout
        time::sleep(Duration::from_secs(timeout_secs)).await;
        
        // Stop scanning
        central.stop_scan().await?;

        // Get discovered devices
        let peripherals = central.peripherals().await?;
        let mut devices = Vec::new();
        for peripheral in peripherals {
            let properties = peripheral.properties().await?;
            if let Some(properties) = properties {
                let name = properties.local_name.clone().unwrap_or_else(|| "Unknown Device".to_string());
                let address = properties.address.to_string();
                let is_mc5000 = Self::is_mc5000_device(&name, &properties.address);
                devices.push(DiscoveredBluetoothDevice {
                    id: peripheral.id().to_string(),
                    name: name.clone(),
                    address,
                    rssi: properties.rssi,
                    is_mc5000,
                    peripheral: peripheral.clone(),
                });
                log::info!("Found device: {} ({})", name, peripheral.id());
            }
        }
        Ok(devices)
    }

    /// Quick scan targeting a specific peripheral ID. Returns early as soon as the
    /// target is seen, so it is much faster than a full scan when the device is nearby.
    /// Returns `None` when the device was not seen within `timeout_secs`.
    pub async fn scan_for_device(target_id: &str, timeout_secs: u64) -> Result<Option<DiscoveredBluetoothDevice>, BluetoothError> {
        let manager = Manager::new().await?;
        let adapters = manager.adapters().await?;

        if adapters.is_empty() {
            return Err(BluetoothError::AdapterNotFound);
        }

        let central = &adapters[0];
        central.start_scan(ScanFilter::default()).await?;

        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        let poll = Duration::from_millis(500);
        let mut found = None;

        while std::time::Instant::now() < deadline {
            time::sleep(poll).await;
            let peripherals = central.peripherals().await?;
            for peripheral in &peripherals {
                if peripheral.id().to_string() == target_id {
                    if let Ok(Some(properties)) = peripheral.properties().await {
                        let name = properties.local_name.clone()
                            .unwrap_or_else(|| "Unknown Device".to_string());
                        let address = properties.address.to_string();
                        let is_mc5000 = Self::is_mc5000_device(&name, &properties.address);
                        found = Some(DiscoveredBluetoothDevice {
                            id: peripheral.id().to_string(),
                            name,
                            address,
                            rssi: properties.rssi,
                            is_mc5000,
                            peripheral: peripheral.clone(),
                        });
                    }
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        central.stop_scan().await?;
        Ok(found)
    }

    /// Check if a device is an MC5000 based on name or advertised services
    fn is_mc5000_device(name: &str, _properties: &btleplug::api::BDAddr) -> bool {
        // Check device name patterns based on actual CSV log findings
        // Be more specific to avoid matching TVs and other devices
        let name_lower = name.to_lowercase();
        if name_lower.contains("mc5000") || 
           name_lower.contains("skyrc") ||
           name_lower.starts_with("#charger") ||
           (name_lower.starts_with("charger") && (name_lower.contains("ff-") || name_lower.contains("ff34"))) ||
           name_lower.contains("telinkse") {
            return true;
        }

        // In a real implementation, you would also check advertised services
        // properties.services.contains(&MC5000_SERVICE_UUID.parse().unwrap())
        
        false
    }

    /// Connect to a specific MC5000 device (with full init sequence)
    pub async fn connect(&mut self, peripheral: &Peripheral) -> Result<(), BluetoothError> {
        self.connect_impl(peripheral, true).await
    }
    
    /// Connect to a specific MC5000 device without init sequence.
    /// Use for status-only queries to avoid disrupting running operations.
    pub async fn connect_without_init(&mut self, peripheral: &Peripheral) -> Result<(), BluetoothError> {
        self.connect_impl(peripheral, false).await
    }
    
    async fn connect_impl(&mut self, peripheral: &Peripheral, send_init: bool) -> Result<(), BluetoothError> {
        verbose_print!("connect() called for device: {}", peripheral.id());
        debug_print!("[DEBUG] Starting connection to device: {}", peripheral.id());
        
        // Check if already connected (with timeout)
        let is_connected = tokio::time::timeout(
            Duration::from_secs(2),
            peripheral.is_connected()
        ).await.unwrap_or(Ok(false)).unwrap_or(false);
        
        if is_connected {
            verbose_print!("Already connected, skipping connection attempt");
        } else {
            // Connect to device (with timeout)
            verbose_print!("Initiating BLE connection...");
            debug_print!("[DEBUG] Attempting BLE connection...");
            tokio::time::timeout(
                Duration::from_secs(10),
                peripheral.connect()
            ).await
                .map_err(|_| {
                    verbose_print!("✗ BLE connection timed out");
                    BluetoothError::ConnectionFailed("Connection timeout".to_string())
                })?
                .map_err(|e| {
                    verbose_print!("✗ BLE connection failed: {}", e);
                    println!("[ERROR] BLE connection failed: {}", e);
                    BluetoothError::ConnectionFailed(e.to_string())
                })?;
            verbose_print!("✓ BLE connection established");
            debug_print!("[DEBUG] BLE connection successful");
        }

        // Wait for connection to establish
        debug_print!("[DEBUG] Waiting for connection to stabilize...");
        time::sleep(Duration::from_millis(1000)).await;
        
        // Verify connection status
        let is_connected = peripheral.is_connected().await.unwrap_or(false);
        debug_print!("[DEBUG] Connection status verified: {}", is_connected);
        if !is_connected {
            return Err(BluetoothError::ConnectionFailed("Device reports not connected after connection attempt".to_string()));
        }

        // Discover services
        debug_print!("[DEBUG] Starting service discovery...");
        peripheral.discover_services().await
            .map_err(|e| {
                println!("[ERROR] Service discovery failed: {}", e);
                BluetoothError::BluetoothError(e.to_string())
            })?;
        debug_print!("[DEBUG] Service discovery completed");

        // Debug: print all discovered service UUIDs
        let services = peripheral.services();
        debug_print!("[DEBUG] Discovered {} services:", services.len());
        println!("[INFO] Discovered {} services:", services.len());
        for (i, s) in services.iter().enumerate() {
            debug_print!("[DEBUG]   Service {}: {}", i+1, s.uuid);
            println!("[INFO]   Service {}: {} ({} characteristics)", i+1, s.uuid, s.characteristics.len());
            for (j, c) in s.characteristics.iter().enumerate() {
                println!("[INFO]     Char {}: {} - Properties: {:?}", j+1, c.uuid, c.properties);
            }
        }

        // Find MC5000 service - look for service with WRITE and NOTIFY characteristics
        // The MC5000 typically uses service 0000ffe0 or similar
        let service = services
            .iter()
            .find(|s| {
                // Look for a service with at least one characteristic that has WRITE
                s.characteristics.iter().any(|c| {
                    c.properties.contains(btleplug::api::CharPropFlags::WRITE) ||
                    c.properties.contains(btleplug::api::CharPropFlags::WRITE_WITHOUT_RESPONSE)
                })
            })
            .ok_or_else(|| {
                println!("[ERROR] No suitable MC5000 service found. Available services:");
                for s in &services {
                    println!("[ERROR]   - {} ({} chars)", s.uuid, s.characteristics.len());
                    for c in &s.characteristics {
                        println!("[ERROR]     - {}: {:?}", c.uuid, c.properties);
                    }
                }
                BluetoothError::ServiceNotFound("No service with WRITE characteristics".to_string())
            })?;
        
        let service_uuid = service.uuid;
        debug_print!("[DEBUG] Selected MC5000 service: {}", service.uuid);
        println!("[INFO] Using service: {}", service.uuid);

        // Debug: print all discovered characteristic UUIDs for this service
        debug_print!("[DEBUG] Discovered {} characteristics for service {}:", service.characteristics.len(), service_uuid);
        println!("[INFO] Discovered characteristics:");
        for (i, c) in service.characteristics.iter().enumerate() {
            debug_print!("[DEBUG]   Char {}: {} Properties: {:?}", i+1, c.uuid, c.properties);
            println!("[INFO]   - {}: Properties: {:?}", c.uuid, c.properties);
        }

        // Find characteristics dynamically based on properties
        let command_char = service.characteristics
            .iter()
            .find(|c| {
                c.properties.contains(btleplug::api::CharPropFlags::WRITE) ||
                c.properties.contains(btleplug::api::CharPropFlags::WRITE_WITHOUT_RESPONSE)
            })
            .map(|c| c.uuid);
        
        if command_char.is_some() {
            verbose_print!("✓ Command characteristic found: {:?}", command_char);
            println!("[INFO] Command characteristic: {:?}", command_char);
        } else {
            verbose_print!("✗ Command characteristic NOT found");
            println!("[ERROR] No WRITE characteristic found!");
        }

        let notify_char = service.characteristics
            .iter()
            .find(|c| c.properties.contains(btleplug::api::CharPropFlags::NOTIFY))
            .map(|c| c.uuid);
        
        if notify_char.is_some() {
            println!("[INFO] Notify characteristic: {:?}", notify_char);
        } else {
            // If no NOTIFY, try INDICATE
            let indicate_char = service.characteristics
                .iter()
                .find(|c| c.properties.contains(btleplug::api::CharPropFlags::INDICATE))
                .map(|c| c.uuid);
            if indicate_char.is_some() {
                println!("[INFO] Indicate characteristic (using instead of notify): {:?}", indicate_char);
            }
        }

        // Use the same characteristic for response (typically the same as notify for MC5000)
        let response_char = notify_char;

        // Subscribe to notifications if available
        if let Some(notify_uuid) = notify_char {
            verbose_print!("Subscribing to notifications on characteristic: {}", notify_uuid);
            peripheral.subscribe(&btleplug::api::Characteristic {
                uuid: notify_uuid,
                service_uuid,
                properties: btleplug::api::CharPropFlags::NOTIFY,
                descriptors: BTreeSet::new(),
            }).await
                .map_err(|e| {
                    println!("[ERROR] Notification subscription failed: {}", e);
                    BluetoothError::BluetoothError(e.to_string())
                })?;
            verbose_print!("✓ Successfully subscribed to notifications");
        } else {
            verbose_print!("WARNING: No notification characteristic found!");
        }

        self.peripheral = Some(peripheral.clone());
        self.service_uuid = Some(service_uuid);
        self.command_char = command_char;
        self.response_char = response_char;
        self.notify_char = notify_char;

        debug_print!("[DEBUG] Connection setup complete:");
        debug_print!("[DEBUG]   Service UUID: {:?}", service_uuid);
        debug_print!("[DEBUG]   Command char: {:?}", command_char);
        debug_print!("[DEBUG]   Response char: {:?}", response_char);
        debug_print!("[DEBUG]   Notify char: {:?}", notify_char);
        
        // Send handshake to activate device communication
        // Use generic handshake since we don't have BT suffix yet
        verbose_print!("Sending handshake (0x57)...");
        let handshake = Self::build_device_info_command("FF34");  // Default suffix
        self.send_command(&handshake).await?;
        verbose_print!("✓ Handshake sent, waiting for device response...");
        
        // Wait briefly for device to process handshake
        time::sleep(Duration::from_millis(500)).await;

        // Send initialization commands matching official app sequence:
        // version(0x74) → settings(0x65) → 0xFE query
        // Without these, the device may ignore control commands like stop.
        verbose_print!("Sending init sequence (0x74, 0x65, 0xFE)...");
        if send_init {
            self.send_init_sequence().await?;
            verbose_print!("✓ Init sequence complete");
        } else {
            verbose_print!("(init sequence skipped — read-only mode)");
        }
        
        log::info!("Successfully connected to MC5000 device: {}", peripheral.id());
        debug_print!("[SUCCESS] MC5000 connection established and ready for commands");
        Ok(())
    }

    /// Send the initialization sequence (0x74, 0x65, 0xFE).
    /// Required before sending control commands (start/stop/config).
    /// NOTE: Sending this on a new session may restart previously configured operations.
    pub async fn send_init_sequence(&self) -> Result<(), BluetoothError> {
        let version_cmd = Self::build_version_command();
        self.send_command(&version_cmd).await?;
        time::sleep(Duration::from_millis(200)).await;
        
        let settings_cmd = Self::build_settings_command();
        self.send_command(&settings_cmd).await?;
        time::sleep(Duration::from_millis(200)).await;
        
        let fe_cmd = Self::build_fe_query_command();
        self.send_command(&fe_cmd).await?;
        time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }

    /// Check if device is connected
    pub async fn is_connected(&self) -> bool {
        if let Some(ref peripheral) = self.peripheral {
            peripheral.is_connected().await.unwrap_or(false)
        } else {
            false
        }
    }
    
    /// Get peripheral reference for notification listening
    pub fn get_peripheral(&self) -> Option<&Peripheral> {
        self.peripheral.as_ref()
    }

    /// Disconnect from device
    pub async fn disconnect(&mut self) -> Result<(), BluetoothError> {
        if let Some(ref peripheral) = self.peripheral {
            peripheral.disconnect().await?;
            log::info!("Disconnected from MC5000 device");
        }
        
        self.peripheral = None;
        self.service_uuid = None;
        self.command_char = None;
        self.response_char = None;
        self.notify_char = None;
        
        Ok(())
    }

    /// Send command to device
    pub async fn send_command(&self, command: &[u8]) -> Result<(), BluetoothError> {
        verbose_print!("send_command called with {} bytes", command.len());
        
        let peripheral = self.peripheral.as_ref()
            .ok_or_else(|| {
                verbose_print!("ERROR: Not connected to device");
                BluetoothError::CommunicationError("Not connected".to_string())
            })?;
        verbose_print!("Peripheral reference obtained");

        let command_uuid = self.command_char
            .ok_or_else(|| {
                verbose_print!("ERROR: Command characteristic not found");
                BluetoothError::CharacteristicNotFound("Command characteristic".to_string())
            })?;
        verbose_print!("Command characteristic UUID: {}", command_uuid);

        let service_uuid = self.service_uuid
            .ok_or_else(|| {
                verbose_print!("ERROR: Service UUID not stored");
                BluetoothError::ServiceNotFound("Service UUID not initialized".to_string())
            })?;
        verbose_print!("Service UUID: {}", service_uuid);

        // Use WRITE_WITHOUT_RESPONSE which matches the actual characteristic properties
        let characteristic = btleplug::api::Characteristic {
            uuid: command_uuid,
            service_uuid,
            properties: btleplug::api::CharPropFlags::WRITE_WITHOUT_RESPONSE,
            descriptors: BTreeSet::new(),
        };

        debug_print!("[DEBUG] Outgoing BLE command: {:02X?}", command);
        if command.len() == 21 && debug_enabled() {
            debug_print!("  Start byte: 0x{:02X}", command[0]);
            debug_print!("  Command: 0x{:02X}", command[1]);
            match command[1] {
                0x57 => debug_print!("  (Version Info Request)"),
                0x61 => debug_print!("  (Get Basic Data)"),
                0x55 => debug_print!("  (Get Channel Data, channel: 0x{:02X})", command[2]),
                0x05 => debug_print!("  (Start Charging, channel bitmask: 0x{:02X})", command[2]),
                _ => debug_print!("  (Unknown/Other Command)"),
            }
            debug_print!("  Payload: {:02X?}", &command[2..20]);
            debug_print!("  Checksum: 0x{:02X}", command[20]);
        } else if command.len() != 21 {
            debug_print!("[WARN] Outgoing command length is not 21 bytes: {}", command.len());
        }
        
        verbose_print!("Writing {} bytes to characteristic UUID: {}", command.len(), command_uuid);
        verbose_print!("Command data: {:02X?}", command);
        debug_print!("[DEBUG] Writing to characteristic...");
        
        match peripheral.write(&characteristic, command, WriteType::WithoutResponse).await {
            Ok(_) => {
                verbose_print!("✓ Write completed successfully");
                debug_print!("[DEBUG] Write completed");
                log::debug!("Sent command: {:?}", command);
                Ok(())
            }
            Err(e) => {
                verbose_print!("✗ Write failed: {}", e);
                Err(e.into())
            }
        }
    }

    /// Read response from device
    pub async fn read_response(&self) -> Result<Vec<u8>, BluetoothError> {
        verbose_print!("read_response called");
        
        // MC5000 only sends responses via NOTIFICATIONS, not via READ
        // This function should not be used for MC5000 - use notification stream instead
        verbose_print!("WARNING: MC5000 does not support read responses - use notifications!");
        
        Err(BluetoothError::CommunicationError(
            "MC5000 only responds via notifications, not read operations".to_string()
        ))
    }

    /// Get device information
    pub async fn get_device_info(&self) -> Result<MC5000DeviceInfo, BluetoothError> {
        debug_print!("[DEBUG] Sending device info request command...");
        let info_command = [0x01, 0x10]; // Example command for device info
        self.send_command(&info_command).await?;
        debug_print!("[DEBUG] Command sent, waiting for response...");
        time::sleep(Duration::from_millis(100)).await;
        let response = self.read_response().await?;
        debug_print!("[DEBUG] Raw device info response: {:?}", response);
        let device_info = MC5000DeviceInfo::parse_from_response(&response)?;
        Ok(device_info)
    }

    /// Get slot status
    pub async fn get_slot_status(&self, slot_id: u8) -> Result<MC5000SlotStatus, BluetoothError> {
        verbose_print!("get_slot_status called for slot {}", slot_id);
        
        // Convert slot ID to channel bitmask (slot 1=0x01, 2=0x02, 3=0x04, 4=0x08)
        let channel_mask = 1u8 << (slot_id - 1);
        verbose_print!("Channel mask for slot {}: 0x{:02X}", slot_id, channel_mask);
        
        // Build proper 0x91 status request command
        let status_command = Self::build_channel_status_command(channel_mask);
        verbose_print!("Status command: {:02X?}", status_command);
        
        self.send_command(&status_command).await?;
        verbose_print!("Status command sent, device will respond via notification");
        
        // MC5000 only responds via notifications, not via read
        // Return error immediately - caller must use notification stream
        Err(BluetoothError::CommunicationError(
            "MC5000 only responds via notifications - use notification stream to receive status".to_string()
        ))
    }
    
    /// Request slot status (fire-and-forget, response comes via notification)
    pub async fn request_slot_status(&self, slot_id: u8) -> Result<(), BluetoothError> {
        verbose_print!("request_slot_status called for slot {}", slot_id);
        
        // Convert slot ID to channel bitmask (slot 1=0x01, 2=0x02, 3=0x04, 4=0x08)
        let channel_mask = 1u8 << (slot_id - 1);
        verbose_print!("Channel mask for slot {}: 0x{:02X}", slot_id, channel_mask);
        
        // Build proper 0x91 status request command
        let status_command = Self::build_channel_status_command(channel_mask);
        verbose_print!("Status command: {:02X?}", status_command);
        
        self.send_command(&status_command).await?;
        verbose_print!("Status request sent successfully");
        
        Ok(())
    }

    /// Start charging on a slot
    pub async fn start_charge(&self, slot_id: u8, voltage: f32, current: f32) -> Result<(), BluetoothError> {
        // Convert float values to protocol format
        let voltage_mv = (voltage * 1000.0) as u16;
        let current_ma = (current * 1000.0) as u16;
        
        let command = [
            0x03, // Start charge command
            slot_id,
            (voltage_mv >> 8) as u8,
            (voltage_mv & 0xFF) as u8,
            (current_ma >> 8) as u8,
            (current_ma & 0xFF) as u8,
        ];
        
        self.send_command(&command).await?;
        log::info!("Started charging slot {} at {}V/{}A", slot_id, voltage, current);
        
        Ok(())
    }

    /// Stop operation on a slot
    pub async fn stop_slot(&self, slot_id: u8) -> Result<(), BluetoothError> {
        let command = [0x04, slot_id]; // Stop command
        self.send_command(&command).await?;
        log::info!("Stopped operation on slot {}", slot_id);
        
        Ok(())
    }
}

/// Charging configuration for a slot
#[derive(Debug, Clone)]
pub struct ChargeConfig {
    /// Channel bitmask (0x01=Slot1, 0x02=Slot2, 0x04=Slot3, 0x08=Slot4, 0x00=All)
    pub channel_bitmask: u8,
    /// Operation mode (Charge, Storage, Discharge, Cycle)
    pub mode: OperationMode,
    /// Battery chemistry (used for protocol byte in config command)
    pub chemistry: BatteryChemistry,
    /// Charge current in mA (e.g., 1000 = 1A, 3000 = 3A)
    pub charge_current_ma: u16,
    /// Discharge current in mA
    pub discharge_current_ma: u16,
    /// Battery capacity in mAh
    pub capacity_mah: u16,
    /// Target/CV voltage in mV.
    /// For Storage mode (capture 4): this is the charge LIMIT (e.g. 4200mV for Li-Ion),
    /// NOT the storage voltage. The storage voltage goes in the secondary field (cap2).
    pub target_voltage_mv: u16,
    /// Cutoff voltage in mV
    pub cutoff_voltage_mv: u16,
    /// Charge cutoff current in mA (CV termination threshold, typ. 100mA)
    pub charge_cutoff_current_ma: u16,
    /// Discharge cutoff current in mA (typ. 100mA)
    pub discharge_cutoff_current_ma: u16,
    /// Trickle charge current in mA (for NiMH/NiCd maintenance, typ. 50mA)
    /// Encoded as ÷10 single byte at data[23] (0x05 = 50mA)
    pub trickle_charge_ma: u16,
    /// Keep/float voltage in mV (NiMH float voltage, e.g. 1300mV = 1.3V)
    /// Encoded as BE u16 at data[24-25]
    pub keep_voltage_mv: u16,
    /// Delta-peak voltage in mV (for NiMH/NiCd charge termination, typ. 6mV)
    /// Encoded as single byte at data[22]
    pub delta_peak_mv: u16,
    /// Cutoff timer in minutes (safety timeout, typ. 60-120, 0=off)
    /// Encoded as BE u16 at data[27-28]
    pub cutoff_timer_min: u16,
    /// Maximum operation time in minutes (typ. 300 = 5 hours)
    pub max_time_min: u16,
    /// Cycle direction:
    ///   0x00 = C→D (charge first, then discharge)
    ///   0x01 = D→C (discharge first, then charge)
    ///   0x02 = C→D→C (charge, discharge, charge)
    ///   0x03 = D→C→D (discharge, charge, discharge)
    /// Confirmed all four values from series 5 captures.
    pub cycle_direction: u8,
    /// Charge resting time in minutes (pause between charge→discharge phases).
    /// Encoded at data[16-17] BE. Confirmed series 5: 10-15 min typical.
    pub charge_resting_min: u16,
    /// Discharge resting time in minutes (pause between discharge→charge phases).
    /// Encoded at data[18-19] BE. Confirmed series 5: 10-11 min typical.
    pub discharge_resting_min: u16,
    /// Number of charge/discharge cycles (for Cycle mode).
    /// Encoded at data[20] as single byte. Default 1, confirmed up to 2 in series 5.
    pub cycle_count: u8,
}

impl ChargeConfig {
    /// Create a new configuration with sensible defaults matching capture 4 observations
    pub fn new(slot: u8, chemistry: BatteryChemistry, mode: OperationMode, capacity_mah: u16, charge_current_ma: u16) -> Self {
        let channel_bitmask = Self::slot_to_bitmask(slot);
        
        Self {
            channel_bitmask,
            mode,
            chemistry,
            charge_current_ma,
            discharge_current_ma: charge_current_ma.min(2000), // Default discharge to min of charge or 2A
            capacity_mah,
            target_voltage_mv: chemistry.target_voltage_mv(),
            cutoff_voltage_mv: chemistry.cutoff_voltage_mv(),
            charge_cutoff_current_ma: 100,
            discharge_cutoff_current_ma: 100,
            trickle_charge_ma: 0, // Default off; NiMH typically 50mA when configured
            keep_voltage_mv: 0,   // Default off; NiMH typically 1300mV when configured
            delta_peak_mv: 6, // App always sends 6, even for lithium
            cutoff_timer_min: 0,  // Default off (0 = no timer); set explicitly when needed
            max_time_min: 300,    // 5 hours
            cycle_direction: 0x00, // C→D default
            charge_resting_min: 10,   // Default 10 min (observed in all captures)
            discharge_resting_min: 10, // Default 10 min
            cycle_count: 1,           // Default 1 cycle
        }
    }
    
    /// Convert slot number (1-4) to bitmask
    fn slot_to_bitmask(slot: u8) -> u8 {
        match slot {
            1 => 0x01,
            2 => 0x02,
            3 => 0x04,
            4 => 0x08,
            _ => 0x01,
        }
    }
    
    /// Create a Li-Ion charging configuration
    pub fn liion(slot: u8, capacity_mah: u16, charge_current_ma: u16) -> Self {
        Self::new(slot, BatteryChemistry::LiIon, OperationMode::Charge, capacity_mah, charge_current_ma)
    }
    
    /// Create a Li-Ion HV charging configuration
    pub fn liion_hv(slot: u8, capacity_mah: u16, charge_current_ma: u16) -> Self {
        Self::new(slot, BatteryChemistry::LiIonHV, OperationMode::Charge, capacity_mah, charge_current_ma)
    }
    
    /// Create a LiFePO4 charging configuration
    pub fn lifepo4(slot: u8, capacity_mah: u16, charge_current_ma: u16) -> Self {
        Self::new(slot, BatteryChemistry::LiFePO4, OperationMode::Charge, capacity_mah, charge_current_ma)
    }
    
    /// Create a NiMH charging configuration
    pub fn nimh(slot: u8, capacity_mah: u16, charge_current_ma: u16) -> Self {
        Self::new(slot, BatteryChemistry::NiMH, OperationMode::Charge, capacity_mah, charge_current_ma)
    }
    
    /// Create a NiCd charging configuration
    pub fn nicd(slot: u8, capacity_mah: u16, charge_current_ma: u16) -> Self {
        Self::new(slot, BatteryChemistry::NiCd, OperationMode::Charge, capacity_mah, charge_current_ma)
    }
    
    /// Create a Li-Ion storage mode configuration
    ///
    /// In capture 4, storage mode sends:
    /// - target_voltage = charge limit (4200mV for Li-Ion), NOT the storage voltage
    /// - secondary value (cap2) = storage voltage (3800mV for Li-Ion)
    ///
    /// The storage_voltage_mv parameter is informational; the actual storage voltage
    /// is determined by chemistry.storage_voltage_mv() in the packet builder.
    pub fn liion_storage(slot: u8, capacity_mah: u16, _storage_voltage_mv: u16, charge_current_ma: u16, discharge_current_ma: u16) -> Self {
        let mut config = Self::new(slot, BatteryChemistry::LiIon, OperationMode::Storage, capacity_mah, charge_current_ma);
        // target_voltage stays at chemistry default (4200mV) - this is the charge LIMIT
        config.discharge_current_ma = discharge_current_ma;
        config.discharge_cutoff_current_ma = 90; // Storage mode often uses 90mA discharge cutoff
        config.cutoff_timer_min = 0; // Default off; set explicitly if needed
        config
    }
    
    /// Create a discharge configuration
    pub fn discharge(slot: u8, chemistry: BatteryChemistry, capacity_mah: u16, discharge_current_ma: u16) -> Self {
        let mut config = Self::new(slot, chemistry, OperationMode::Discharge, capacity_mah, 0);
        config.discharge_current_ma = discharge_current_ma;
        config
    }
    
    /// Create a cycle (charge/discharge) configuration (mode 0x04)
    ///
    /// From protocol capture 3: Cycle D→C was observed with cycle_direction=0x01 in the config
    /// packet. The charger starts with discharge (status 0x07) then transitions to charge.
    /// The discharge current defaults to the same as charge current (both 2000mA observed).
    /// Default direction is D→C (0x01) to match capture 3 observation.
    pub fn cycle(slot: u8, chemistry: BatteryChemistry, capacity_mah: u16, charge_current_ma: u16) -> Self {
        let mut config = Self::new(slot, chemistry, OperationMode::Cycle, capacity_mah, charge_current_ma);
        config.discharge_current_ma = charge_current_ma; // Cycle uses same rate for both phases
        config.cycle_direction = 0x01; // D→C default (matches capture 3)
        config
    }
    
    /// Create a refresh configuration (mode 0x05)
    ///
    /// Refresh is a deep-cycle mode available for NiMH and NiCd chemistries.
    /// It performs charge/discharge cycles to restore battery capacity.
    pub fn refresh(slot: u8, chemistry: BatteryChemistry, capacity_mah: u16, charge_current_ma: u16, discharge_current_ma: u16) -> Self {
        let mut config = Self::new(slot, chemistry, OperationMode::Refresh, capacity_mah, charge_current_ma);
        config.discharge_current_ma = discharge_current_ma;
        // Refresh mode typically uses longer timeouts
        config.cutoff_timer_min = 0; // Off (no timeout)
        config.max_time_min = 600; // 10 hours
        config
    }
    
    /// Create a break-in configuration (mode 0x02)
    ///
    /// Break-in is a conditioning mode for new NiMH/NiCd batteries.
    /// It performs gentle charge/discharge cycles (C→D→C) to condition the cells.
    /// The charge current is typically low (0.1-0.3C rate).
    pub fn break_in(slot: u8, chemistry: BatteryChemistry, capacity_mah: u16, charge_current_ma: u16) -> Self {
        let mut config = Self::new(slot, chemistry, OperationMode::BreakIn, capacity_mah, charge_current_ma);
        // Capture shows discharge current = 2× charge current for break-in
        config.discharge_current_ma = charge_current_ma.saturating_mul(2);
        // Match capture: cutoff_timer = 60min, max_time = 300min
        config.cutoff_timer_min = 60;
        config.max_time_min = 300;
        config
    }
    
    /// Builder method: set trickle charge current
    pub fn with_trickle_charge(mut self, trickle_ma: u16) -> Self {
        self.trickle_charge_ma = trickle_ma;
        self
    }
    
    /// Builder method: set keep voltage
    pub fn with_keep_voltage(mut self, keep_voltage_mv: u16) -> Self {
        self.keep_voltage_mv = keep_voltage_mv;
        self
    }
    
    /// Builder method: set delta-peak for NiMH/NiCd
    pub fn with_delta_peak(mut self, delta_peak_mv: u16) -> Self {
        self.delta_peak_mv = delta_peak_mv;
        self
    }
    
    /// Builder method: set cutoff timer
    pub fn with_cutoff_timer(mut self, cutoff_timer_min: u16) -> Self {
        self.cutoff_timer_min = cutoff_timer_min;
        self
    }
    
    /// Builder method: set max time
    pub fn with_max_time(mut self, max_time_min: u16) -> Self {
        self.max_time_min = max_time_min;
        self
    }
    
    /// Builder method: set cycle direction
    ///   0x00 = C→D, 0x01 = D→C, 0x02 = C→D→C, 0x03 = D→C→D
    pub fn with_cycle_direction(mut self, direction: u8) -> Self {
        self.cycle_direction = direction;
        self
    }
    
    /// Builder method: set charge resting time (minutes)
    pub fn with_charge_resting(mut self, minutes: u16) -> Self {
        self.charge_resting_min = minutes;
        self
    }
    
    /// Builder method: set discharge resting time (minutes)
    pub fn with_discharge_resting(mut self, minutes: u16) -> Self {
        self.discharge_resting_min = minutes;
        self
    }
    
    /// Builder method: set cycle count
    pub fn with_cycle_count(mut self, count: u8) -> Self {
        self.cycle_count = count;
        self
    }
    
    /// Builder method: configure for all slots (channel bitmask 0x00)
    /// Capture 4 shows the app sends channel=0x00 to apply config to all slots.
    /// The device ACKs with channel=0x10.
    pub fn for_all_slots(mut self) -> Self {
        self.channel_bitmask = 0x00;
        self
    }
}

/// Minimal acknowledgement for a 0x94 configuration packet
/// Example observed: 0f 04 94 01 01 96 → channel 0x01, status=0x01 (accepted)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeConfigAck {
    pub channel_bitmask: u8,
    pub accepted: bool,
}

impl ChargeConfigAck {
    /// Parse a short 0x94 ACK response
    pub fn parse_from_response(response: &[u8]) -> Result<Self, BluetoothError> {
        if response.len() < 6 {
            return Err(BluetoothError::CommunicationError(
                format!("Charge config ACK too short: {} bytes", response.len())
            ));
        }

        if response[0] != PACKET_START {
            return Err(BluetoothError::CommunicationError(
                format!("Invalid start marker for ACK: 0x{:02X}", response[0])
            ));
        }

        if response[2] != CMD_CHARGE_CONFIG {
            return Err(BluetoothError::CommunicationError(
                format!("Unexpected command in ACK: 0x{:02X}", response[2])
            ));
        }

        let expected = MC5000Protocol::calculate_checksum(&response[2..response.len()-1]);
        if expected != response[response.len()-1] {
            return Err(BluetoothError::CommunicationError(
                format!("Invalid checksum in ACK: expected 0x{:02X}, got 0x{:02X}", expected, response[response.len()-1])
            ));
        }

        Ok(Self {
            channel_bitmask: response[3],
            accepted: response[4] == 0x01,
        })
    }
}

impl ChargeConfig {
}

#[derive(Debug, Clone)]
pub struct MC5000DeviceInfo {
    pub firmware_version: String,
    pub hardware_version: String,
    pub serial_number: String,
    pub slot_count: u8,
}

impl MC5000DeviceInfo {
    /// Parse device info from 0x57 command response
    /// Response format: 0f 13 57 00 <serial:7> <??:1> <pad:4> <fw_maj:1> <fw_min:1> <hw_maj:1> <hw_min:1> <??:1> <checksum:1>
    pub fn parse_from_response(response: &[u8]) -> Result<Self, BluetoothError> {
        // Check minimum length and start marker
        if response.len() < 6 {
            return Err(BluetoothError::CommunicationError(
                format!("Response too short: {} bytes", response.len())
            ));
        }

        // Check packet start marker
        if response[0] != PACKET_START {
            return Err(BluetoothError::CommunicationError(
                format!("Invalid start marker: 0x{:02X}", response[0])
            ));
        }

        let _length = response[1];
        let command = response[2];

        // Handle different response types
        match command {
            CMD_GREETING => {
                // Greeting packet: 0f 04 06 01 01 08
                // This is the initial unsolicited notification
                Ok(MC5000DeviceInfo {
                    firmware_version: "Unknown".to_string(),
                    hardware_version: "Unknown".to_string(),
                    serial_number: "Unknown".to_string(),
                    slot_count: 8,
                })
            }
            CMD_DEVICE_INFO => {
                // Device info response: 0f 13 57 00 31 30 30 32 31 33 34 04 00 00 00 00 01 52 01 00 80 57
                if response.len() < 20 {
                    return Err(BluetoothError::CommunicationError(
                        format!("Device info response too short: {} bytes", response.len())
                    ));
                }

                // Parse serial number (ASCII bytes at offset 4-10)
                let serial_bytes = &response[4..11];
                let serial_number = String::from_utf8_lossy(serial_bytes).to_string();

                // Firmware version at offset 16-17: major, minor
                let fw_major = response[16];
                let fw_minor = response[17];
                let firmware_version = format!("{}.{:02}", fw_major, fw_minor);

                // Hardware version at offset 18-19: major, minor  
                let hw_major = response[18];
                let hw_minor = response[19];
                let hardware_version = format!("{}.{:02}", hw_major, hw_minor);

                Ok(MC5000DeviceInfo {
                    firmware_version,
                    hardware_version,
                    serial_number,
                    slot_count: 8,
                })
            }
            _ => {
                Err(BluetoothError::CommunicationError(
                    format!("Unexpected response command: 0x{:02X}", command)
                ))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct MC5000SlotStatus {
    pub slot_id: u8,
    pub state: MC5000SlotState,
    pub voltage_mv: u16,
    pub current_ma: u16,
    pub capacity_mah: u32,
    pub elapsed_seconds: u16,
    pub resistance_milliohm: u16,
    pub temperature: u8,
    pub deltav_mv: u8,  // Voltage drop for charge detection
}

#[derive(Debug, Clone, PartialEq)]
pub enum MC5000SlotState {
    Empty,
    Idle,
    Charging,     // Generic charging
    ChargingCC,   // Constant Current phase
    ChargingCV,   // Constant Voltage phase (current tapering)
    Discharging,
    Completed,
    Error,
    Paused,
}

impl MC5000SlotStatus {
    /// Parse channel status from 0x91 command response
    /// Response format: 0f 15 91 <channel:1> <current_hi:1> <current_lo:1> <voltage:2> ...
    pub fn parse_from_response(response: &[u8]) -> Result<Self, BluetoothError> {
        if response.len() < 10 {
            return Err(BluetoothError::CommunicationError(
                format!("Channel status response too short: {} bytes", response.len())
            ));
        }

        // Check packet start marker
        if response[0] != PACKET_START {
            return Err(BluetoothError::CommunicationError(
                format!("Invalid start marker: 0x{:02X}", response[0])
            ));
        }

        let command = response[2];
        if command != CMD_CHANNEL_STATUS {
            return Err(BluetoothError::CommunicationError(
                format!("Expected channel status response (0x91), got 0x{:02X}", command)
            ));
        }

        let channel = response[3];
        let slot_id = match channel {
            0x01 => 1,
            0x02 => 2,
            0x04 => 3,
            0x08 => 4,
            0x10 => 5,
            0x20 => 6,
            0x40 => 7,
            0x80 => 8,
            _ => 0,
        };

        // Bytes 4-5: current in mA as a big-endian u16.
        // Previously misinterpreted as separate "status byte" + "multiplier byte".
        // Empirically confirmed:  100mA→[0x00,0x63]=99,  150mA→[0x00,0x95]=149,
        //   200mA→[0x00,0xC5]=197,  300mA→[0x01,0x2A]=298,  700mA→[0x02,0xBD]=701.
        // The high byte is the mA hundreds/thousands; low byte is the remainder.
        let current_ma = u16::from_be_bytes([response[4], response[5]]);

        // Voltage at offset 6-7 (big-endian, in mV)
        let voltage_mv = u16::from_be_bytes([response[6], response[7]]);

        // Capacity at bytes 10-11 (big-endian u16, mAh).
        let capacity_mah = if response.len() >= 12 {
            u16::from_be_bytes([response[10], response[11]]) as u32
        } else {
            0
        };

        // Elapsed time: bytes 14-15 (big-endian, seconds)
        let elapsed_seconds = if response.len() >= 16 {
            u16::from_be_bytes([response[14], response[15]])
        } else {
            0
        };

        // Resistance: bytes 16-17 (big-endian, milliohms)
        let resistance_milliohm = if response.len() >= 18 {
            u16::from_be_bytes([response[16], response[17]])
        } else {
            0
        };

        // Delta-V: byte 18 (NiMH charge termination indicator, mV)
        let deltav_mv = if response.len() >= 19 {
            response[18]
        } else {
            0
        };

        // Derive device state from current + voltage + capacity/elapsed.
        // There is no separate status/mode byte — bytes [4-5] are the current itself.
        let state = if voltage_mv == 0 {
            MC5000SlotState::Empty
        } else if current_ma > 0 {
            // Active: charging or discharging. Direction is tracked by the app via task type.
            MC5000SlotState::Charging
        } else if elapsed_seconds > 0 && capacity_mah > 0 {
            // Current dropped to zero after some activity → cycle finished.
            MC5000SlotState::Completed
        } else {
            MC5000SlotState::Idle
        };

        Ok(MC5000SlotStatus {
            slot_id,
            state,
            voltage_mv,
            current_ma,
            capacity_mah,
            elapsed_seconds,
            resistance_milliohm,
            temperature: 0, // Temperature not available in this protocol response
            deltav_mv,
        })
    }

    /// Get voltage in volts
    pub fn voltage(&self) -> f32 {
        self.voltage_mv as f32 / 1000.0
    }

    /// Get current in amps
    pub fn current(&self) -> f32 {
        self.current_ma as f32 / 1000.0
    }

    /// Get instantaneous power in watts
    pub fn power_w(&self) -> f32 {
        // voltage (mV) * current (mA) -> µW; divide by 1e6 to watts
        (self.voltage_mv as f32 * self.current_ma as f32) / 1_000_000.0
    }
}