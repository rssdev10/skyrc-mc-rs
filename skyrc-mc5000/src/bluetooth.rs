use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use std::collections::BTreeSet;
use std::time::Duration;
use thiserror::Error;
use tokio::time;
use uuid::Uuid;


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
const CMD_GREETING: u8 = 0x06;      // Device greeting (unsolicited)
const CMD_DEVICE_INFO: u8 = 0x57;   // Device handshake / info request
const CMD_VERSION: u8 = 0x74;       // Password verification (default "0000")
const CMD_CHANNEL_STATUS: u8 = 0x91; // Channel status request
const CMD_START_STOP: u8 = 0x93;    // Read a slot's stored program (official app); never answered
const CMD_CHARGE_CONFIG: u8 = 0x94; // Program a slot and start it
const CMD_SETTINGS: u8 = 0x65;      // Firmware-update-available flag
const CMD_SYSTEM_SETTINGS: u8 = 0x96; // Read system settings (beeps, LCD, sleep)
/// Read a slot's voltage history (charge curve). Read only.
pub const CMD_VOLTAGE_HISTORY: u8 = 0x98;

/// Argument of the 0x93 frame this client sends after its configs and stops.
///
/// The official app uses 0x93 only to read a slot's stored program (argument = 0-based slot
/// index); the charger does not answer it, and starting and stopping are done by 0x94 and
/// 0xFE. The names below are historical; see docs/PROTOCOL.md.
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

/// Mode numbering families: the charger numbers the 0x94 mode byte and status byte 19
/// differently for each family (official app; every captured config and status agrees).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeFamily {
    /// Li-Ion, Li-Ion HV, LiFePO4, LTO, Na-Ion: Charge, Storage, Discharge, Cycle.
    Lithium,
    /// NiMH, NiCd, eneloop: Charge, Refresh, Break-in, Discharge, Cycle.
    Nickel,
    /// NiZn and RAM: Charge, Discharge, Cycle.
    NiZnRam,
}

impl ModeFamily {
    fn of(chemistry: BatteryChemistry) -> Self {
        match chemistry {
            BatteryChemistry::NiMH | BatteryChemistry::NiCd | BatteryChemistry::Eneloop => {
                ModeFamily::Nickel
            }
            BatteryChemistry::NiZn | BatteryChemistry::RAM => ModeFamily::NiZnRam,
            BatteryChemistry::LiIon
            | BatteryChemistry::LiIonHV
            | BatteryChemistry::LiFePO4
            | BatteryChemistry::LTO
            | BatteryChemistry::NaIon => ModeFamily::Lithium,
        }
    }

    /// The modes in protocol-byte order: index = byte value.
    fn modes(self) -> &'static [OperationMode] {
        use OperationMode::*;
        match self {
            ModeFamily::Lithium => &[Charge, Storage, Discharge, Cycle],
            ModeFamily::Nickel => &[Charge, Refresh, BreakIn, Discharge, Cycle],
            ModeFamily::NiZnRam => &[Charge, Discharge, Cycle],
        }
    }
}

impl OperationMode {
    /// The 0x94 mode byte for this mode, or `None` when the chemistry has no such mode
    /// (for example Storage for NiMH). Numbering per family:
    ///
    /// - Li-Ion, Li-Ion HV, LiFePO4, LTO, Na-Ion: Charge 0, Storage 1, Discharge 2, Cycle 3
    /// - NiMH, NiCd, eneloop: Charge 0, Refresh 1, Break-in 2, Discharge 3, Cycle 4
    /// - NiZn, RAM: Charge 0, Discharge 1, Cycle 2
    pub fn to_byte_for_chemistry(self, chemistry: BatteryChemistry) -> Option<u8> {
        ModeFamily::of(chemistry)
            .modes()
            .iter()
            .position(|&m| m == self)
            .map(|i| i as u8)
    }

    /// Parses a mode byte (0x94 byte 4, status byte 19) for the given chemistry.
    /// Inverse of [`OperationMode::to_byte_for_chemistry`].
    pub fn from_byte_for_chemistry(byte: u8, chemistry: BatteryChemistry) -> Option<Self> {
        ModeFamily::of(chemistry).modes().get(usize::from(byte)).copied()
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

    /// Build the 0x57 device-info request: `0f 13 57 00` + the charger's BLE address, least
    /// significant byte first, + 10 zero bytes + checksum (21 bytes, as the official app
    /// sends it).
    ///
    /// Only the last two address bytes are known everywhere: the charger advertises them in
    /// its name ("#Charger-FF34"). The other four are those of the unit the protocol was
    /// captured from. That is harmless: the charger answered requests carrying a wrong or an
    /// all-zero address the same way (verified live).
    pub fn build_device_info_command(bt_suffix: &str) -> Vec<u8> {
        let byte = |i: usize, fallback: u8| {
            bt_suffix
                .get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .unwrap_or(fallback)
        };
        let mut data = [0u8; 17];
        data[1..7].copy_from_slice(&[byte(0, 0xFF), byte(2, 0x34), 0x37, 0x38, 0xC1, 0xA4]);
        Self::build_packet(CMD_DEVICE_INFO, &data)
    }

    /// Build the 0x96 system-settings read (`96 04`, as the official app sends it). Read
    /// only; the reply parses with [`MC5000SystemSettings::parse_from_response`].
    pub fn build_system_settings_query() -> Vec<u8> {
        Self::build_packet(CMD_SYSTEM_SETTINGS, &[0x04])
    }

    /// Build the 0x98 voltage-history request for one slot bit (`01`, `02`, `04`, `08`).
    /// Read only. The reply is up to three [`HistoryPage`]s; a slot with no history does
    /// not answer.
    pub fn build_voltage_history_command(slot_bit: u8) -> Vec<u8> {
        Self::build_packet(CMD_VOLTAGE_HISTORY, &[slot_bit])
    }

    /// Build channel status request command (0x91)
    pub fn build_channel_status_command(channel_bitmask: u8) -> Vec<u8> {
        Self::build_packet(CMD_CHANNEL_STATUS, &[channel_bitmask])
    }

    /// Build the 0x74 password verification with the default password `0000`
    /// (`00` then one byte per digit). The charger replies `74 01 01` when accepted.
    pub fn build_version_command() -> Vec<u8> {
        Self::build_packet(CMD_VERSION, &[0x00, 0x00, 0x00, 0x00, 0x00])
    }

    /// Build 0x65 `00 00`: tells the charger that no newer firmware is available (the official
    /// app sends `00 01` when its update server offers one). The reply is not used.
    pub fn build_settings_command() -> Vec<u8> {
        Self::build_packet(CMD_SETTINGS, &[0x00, 0x00])
    }

    /// Build the 0xFE **stop** command for a slot bitmask (0x01-0x08).
    ///
    /// The official app's Stop: the charger stops that slot and replies `FE <mask> 01`.
    /// A mask with several bits stops only the lowest slot; `0x00` stops every slot.
    /// (Long misnamed a "query"; the name is kept for API stability.)
    pub fn build_fe_query_command_for(channel: u8) -> Vec<u8> {
        Self::build_packet(0xFE, &[channel])
    }

    /// Build `FE 00`: **stops every running program** on the charger.
    pub fn build_fe_query_command() -> Vec<u8> {
        Self::build_packet(0xFE, &[0x00])
    }

    /// Build a 0x93 frame. It neither starts nor stops anything (see [`StartStopAction`]);
    /// the charger does not answer it.
    pub fn build_start_stop_command(action: StartStopAction) -> Vec<u8> {
        let byte = match action {
            StartStopAction::StopAll => 0x00,
            StartStopAction::StartAll => 0x03,
            StartStopAction::ChannelMask(mask) => mask,
        };

        Self::build_packet(CMD_START_STOP, &[byte])
    }

    /// Build charging configuration command (0x94)
    /// Sets up charging parameters for a slot.
    ///
    /// The charger starts the program as soon as it acknowledges this frame.
    ///
    /// Fails, without building anything, when the chemistry has no such mode (for example
    /// Storage for NiMH): sending another mode's byte would run a different program.
    pub fn build_charge_config_command(config: &ChargeConfig) -> Result<Vec<u8>, BluetoothError> {
        let mode_byte = config.mode.to_byte_for_chemistry(config.chemistry).ok_or_else(|| {
            BluetoothError::CommunicationError(format!(
                "{:?} has no {:?} mode",
                config.chemistry, config.mode
            ))
        })?;
        let mut data = Vec::with_capacity(41);
        
        // Byte 0: Channel bitmask (0x01/0x02/0x04/0x08, or 0x00 for all slots)
        data.push(config.channel_bitmask);
        
        // Byte 1: Mode, numbered per chemistry family
        data.push(mode_byte);
        
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
        
        // Byte 26: Battery temperature cut-off (°C). The official app sends the program's
        //   value (71 when unset); 60 in every captured config, which is what we send.
        data.push(0x3C);
        
        // Bytes 27-28: Cutoff timer (minutes, BIG-endian)
        //   CORRECTED: Was 2-byte LE at data[26-27] in old code. Capture 4 Config #5
        //   (Li-Ion storage, timer=120) has bytes 0x00,0x78 at data[27-28] → BE 0x0078=120 ✓.
        //   Config #3 (NiMH, timer=90) has 0x00,0x5A → BE 0x005A=90 ✓.
        data.push((config.cutoff_timer_min >> 8) as u8);
        data.push((config.cutoff_timer_min & 0xFF) as u8);
        
        // Bytes 29-30: The official app always sends 300 here (its field `inMaxbostCapacity`;
        //   meaning unconfirmed). Callers pass 300.
        data.push((config.max_time_min >> 8) as u8);
        data.push((config.max_time_min & 0xFF) as u8);
        
        // Byte 31: Turbo mode (official app); always 0 in captures
        data.push(0x00);
        
        // Byte 32: Chemistry byte (confirmed series 5 — all 10 chemistries)
        //   0x00=Li-Ion, 0x01=LiHV, 0x02=LiFePO4, 0x03=NiMH,
        //   0x04=NiCd, 0x05=eneloop, 0x06=NiZn, 0x07=RAM,
        //   0x08=LTO, 0x09=Na-Ion
        data.push(config.chemistry.to_protocol_byte());
        
        // Bytes 33-34: Storage voltage (mV, big-endian; official app).
        //   Lithium family: Li-Ion=3800, LiHV=3900, LiFePO4=3300, LTO=2400, Na-Ion=3500.
        //   Nickel, NiZn and RAM do not use it; the app leaves it at 3300 for every capacity.
        //   This client sends 110 % of the capacity there, which equals 3300 for the
        //   3000 mAh default (kept unchanged; the charger ignores it for these chemistries).
        let secondary_value = if config.chemistry.is_nickel_based() {
            (config.capacity_mah as f32 * 1.1) as u16
        } else {
            // Lithium: use storage voltage
            config.chemistry.storage_voltage_mv().unwrap_or(config.capacity_mah)
        };
        data.push((secondary_value >> 8) as u8);
        data.push((secondary_value & 0xFF) as u8);
        
        // Byte 35: eneloop cell type (official app); 0 for other chemistries.
        data.push(if config.chemistry == BatteryChemistry::Eneloop {
            config.eneloop_type as u8
        } else {
            0
        });
        // Bytes 36-39: unused. Captured frames are 44 bytes: start, length, cmd, 40 data, sum.
        data.extend([0x00u8; 4]);
        
        debug_assert_eq!(data.len(), 40, "Config data must be exactly 40 bytes");

        Ok(Self::build_packet(CMD_CHARGE_CONFIG, &data))
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

    /// Check if a device is an MC5000 based on its advertised name.
    ///
    /// The charger does not advertise its service UUID, so the name is all there is.
    /// Units advertise as `#Charger-XXXX` (some stacks drop the `#`), where `XXXX` is
    /// the end of the unit's own address.
    fn is_mc5000_device(name: &str, _properties: &btleplug::api::BDAddr) -> bool {
        let name_lower = name.to_lowercase();
        name_lower.contains("mc5000")
            || name_lower.contains("skyrc")
            || name_lower.contains("telinkse")
            || name_lower.starts_with("#charger")
            || (name_lower.starts_with("charger") && Self::name_suffix(name).is_some())
    }

    /// The 4-hex-digit unit suffix of an advertised name such as `#Charger-E56A`.
    pub(crate) fn name_suffix(name: &str) -> Option<String> {
        let suffix = name.rsplit(['-', '_', ' ']).next()?;
        (suffix.len() == 4 && suffix.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| suffix.to_ascii_uppercase())
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
        log::debug!("connect() called for device: {}", peripheral.id());
        log::trace!("Starting connection to device: {}", peripheral.id());
        
        // Check if already connected (with timeout)
        let is_connected = tokio::time::timeout(
            Duration::from_secs(2),
            peripheral.is_connected()
        ).await.unwrap_or(Ok(false)).unwrap_or(false);
        
        if is_connected {
            log::debug!("Already connected, skipping connection attempt");
        } else {
            // Connect to device (with timeout)
            log::debug!("Initiating BLE connection...");
            log::trace!("Attempting BLE connection...");
            tokio::time::timeout(
                Duration::from_secs(10),
                peripheral.connect()
            ).await
                .map_err(|_| {
                    log::debug!("✗ BLE connection timed out");
                    BluetoothError::ConnectionFailed("Connection timeout".to_string())
                })?
                .map_err(|e| {
                    log::debug!("✗ BLE connection failed: {}", e);
                    log::error!("BLE connection failed: {}", e);
                    BluetoothError::ConnectionFailed(e.to_string())
                })?;
            log::debug!("✓ BLE connection established");
            log::trace!("BLE connection successful");
        }

        // Wait for connection to establish
        log::trace!("Waiting for connection to stabilize...");
        time::sleep(Duration::from_millis(1000)).await;
        
        // Verify connection status
        let is_connected = peripheral.is_connected().await.unwrap_or(false);
        log::trace!("Connection status verified: {}", is_connected);
        if !is_connected {
            return Err(BluetoothError::ConnectionFailed("Device reports not connected after connection attempt".to_string()));
        }

        // Discover services
        log::trace!("Starting service discovery...");
        peripheral.discover_services().await
            .map_err(|e| {
                log::error!("Service discovery failed: {}", e);
                BluetoothError::BluetoothError(e.to_string())
            })?;
        log::trace!("Service discovery completed");

        // Debug: print all discovered service UUIDs
        let services = peripheral.services();
        log::trace!("Discovered {} services:", services.len());
        log::debug!("Discovered {} services:", services.len());
        for (i, s) in services.iter().enumerate() {
            log::trace!("  Service {}: {}", i+1, s.uuid);
            log::debug!("  Service {}: {} ({} characteristics)", i+1, s.uuid, s.characteristics.len());
            for (j, c) in s.characteristics.iter().enumerate() {
                log::debug!("    Char {}: {} - Properties: {:?}", j+1, c.uuid, c.properties);
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
                log::error!("No suitable MC5000 service found. Available services:");
                for s in &services {
                    log::error!("  - {} ({} chars)", s.uuid, s.characteristics.len());
                    for c in &s.characteristics {
                        log::error!("    - {}: {:?}", c.uuid, c.properties);
                    }
                }
                BluetoothError::ServiceNotFound("No service with WRITE characteristics".to_string())
            })?;
        
        let service_uuid = service.uuid;
        log::trace!("Selected MC5000 service: {}", service.uuid);
        log::debug!("Using service: {}", service.uuid);

        // Debug: print all discovered characteristic UUIDs for this service
        log::trace!("Discovered {} characteristics for service {}:", service.characteristics.len(), service_uuid);
        log::debug!("Discovered characteristics:");
        for (i, c) in service.characteristics.iter().enumerate() {
            log::trace!("  Char {}: {} Properties: {:?}", i+1, c.uuid, c.properties);
            log::debug!("  - {}: Properties: {:?}", c.uuid, c.properties);
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
            log::debug!("✓ Command characteristic found: {:?}", command_char);
            log::debug!("Command characteristic: {:?}", command_char);
        } else {
            log::debug!("✗ Command characteristic NOT found");
            log::error!("No WRITE characteristic found!");
        }

        let notify_char = service.characteristics
            .iter()
            .find(|c| c.properties.contains(btleplug::api::CharPropFlags::NOTIFY))
            .map(|c| c.uuid);
        
        if notify_char.is_some() {
            log::debug!("Notify characteristic: {:?}", notify_char);
        } else {
            // If no NOTIFY, try INDICATE
            let indicate_char = service.characteristics
                .iter()
                .find(|c| c.properties.contains(btleplug::api::CharPropFlags::INDICATE))
                .map(|c| c.uuid);
            if indicate_char.is_some() {
                log::debug!("Indicate characteristic (using instead of notify): {:?}", indicate_char);
            }
        }

        // Use the same characteristic for response (typically the same as notify for MC5000)
        let response_char = notify_char;

        // Subscribe to notifications if available
        if let Some(notify_uuid) = notify_char {
            log::debug!("Subscribing to notifications on characteristic: {}", notify_uuid);
            peripheral.subscribe(&btleplug::api::Characteristic {
                uuid: notify_uuid,
                service_uuid,
                properties: btleplug::api::CharPropFlags::NOTIFY,
                descriptors: BTreeSet::new(),
            }).await
                .map_err(|e| {
                    log::error!("Notification subscription failed: {}", e);
                    BluetoothError::BluetoothError(e.to_string())
                })?;
            log::debug!("✓ Successfully subscribed to notifications");
        } else {
            log::debug!("WARNING: No notification characteristic found!");
        }

        self.peripheral = Some(peripheral.clone());
        self.service_uuid = Some(service_uuid);
        self.command_char = command_char;
        self.response_char = response_char;
        self.notify_char = notify_char;

        log::trace!("Connection setup complete:");
        log::trace!("  Service UUID: {:?}", service_uuid);
        log::trace!("  Command char: {:?}", command_char);
        log::trace!("  Response char: {:?}", response_char);
        log::trace!("  Notify char: {:?}", notify_char);
        
        // Send handshake to activate device communication. It carries the last two
        // address bytes, which the charger also advertises in its name ("#Charger-FF34").
        let advertised_name = peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.local_name);
        let suffix = advertised_name
            .as_deref()
            .and_then(Self::name_suffix)
            .unwrap_or_else(|| "FF34".to_string());
        log::debug!("Sending handshake (0x57) with suffix {}...", suffix);
        let handshake = Self::build_device_info_command(&suffix);
        self.send_command(&handshake).await?;
        log::debug!("✓ Handshake sent, waiting for device response...");
        
        // Wait briefly for device to process handshake
        time::sleep(Duration::from_millis(500)).await;

        // Send initialization commands matching official app sequence:
        // version(0x74) → settings(0x65); never the general 0xFE query, which stops all slots
        // Without these, the device may ignore control commands like stop.
        log::debug!("Sending init sequence (0x74, 0x65)...");
        if send_init {
            self.send_init_sequence().await?;
            log::debug!("✓ Init sequence complete");
        } else {
            log::debug!("(init sequence skipped — read-only mode)");
        }
        
        log::info!("Successfully connected to MC5000 device: {}", peripheral.id());
        log::trace!("MC5000 connection established and ready for commands");
        Ok(())
    }

    /// Send the session init queries the official app sends: 0x74 (version) and 0x65
    /// (settings). Both leave running programs untouched.
    ///
    /// The app also sends the general 0xFE query (`FE 00`), but that **stops every running
    /// program** (verified on a live charger), so it is deliberately not sent: connecting
    /// must never stop a charge. Start and stop sequences select their slot with a per-slot
    /// FE query instead, and work without any init.
    pub async fn send_init_sequence(&self) -> Result<(), BluetoothError> {
        let version_cmd = Self::build_version_command();
        self.send_command(&version_cmd).await?;
        time::sleep(Duration::from_millis(200)).await;

        let settings_cmd = Self::build_settings_command();
        self.send_command(&settings_cmd).await?;
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
        log::debug!("send_command called with {} bytes", command.len());
        
        let peripheral = self.peripheral.as_ref()
            .ok_or_else(|| {
                log::debug!("ERROR: Not connected to device");
                BluetoothError::CommunicationError("Not connected".to_string())
            })?;
        log::debug!("Peripheral reference obtained");

        let command_uuid = self.command_char
            .ok_or_else(|| {
                log::debug!("ERROR: Command characteristic not found");
                BluetoothError::CharacteristicNotFound("Command characteristic".to_string())
            })?;
        log::debug!("Command characteristic UUID: {}", command_uuid);

        let service_uuid = self.service_uuid
            .ok_or_else(|| {
                log::debug!("ERROR: Service UUID not stored");
                BluetoothError::ServiceNotFound("Service UUID not initialized".to_string())
            })?;
        log::debug!("Service UUID: {}", service_uuid);

        // Use WRITE_WITHOUT_RESPONSE which matches the actual characteristic properties
        let characteristic = btleplug::api::Characteristic {
            uuid: command_uuid,
            service_uuid,
            properties: btleplug::api::CharPropFlags::WRITE_WITHOUT_RESPONSE,
            descriptors: BTreeSet::new(),
        };

        log::trace!("TX frame: {:02X?}", command);
        
        match peripheral.write(&characteristic, command, WriteType::WithoutResponse).await {
            Ok(_) => {
                log::trace!("Write completed");
                log::debug!("Sent command: {:?}", command);
                Ok(())
            }
            Err(e) => {
                log::debug!("✗ Write failed: {}", e);
                Err(e.into())
            }
        }
    }

    /// Request slot status (fire-and-forget, response comes via notification)
    pub async fn request_slot_status(&self, slot_id: u8) -> Result<(), BluetoothError> {
        log::debug!("request_slot_status called for slot {}", slot_id);
        
        // Convert slot ID to channel bitmask (slot 1=0x01, 2=0x02, 3=0x04, 4=0x08)
        let channel_mask = 1u8 << (slot_id - 1);
        log::debug!("Channel mask for slot {}: 0x{:02X}", slot_id, channel_mask);
        
        // Build proper 0x91 status request command
        let status_command = Self::build_channel_status_command(channel_mask);
        log::debug!("Status command: {:02X?}", status_command);
        
        self.send_command(&status_command).await?;
        log::debug!("Status request sent successfully");
        
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
    /// Encoded as ÷10 single byte at `data[23]` (0x05 = 50mA)
    pub trickle_charge_ma: u16,
    /// Keep/float voltage in mV (NiMH float voltage, e.g. 1300mV = 1.3V)
    /// Encoded as BE u16 at data[24-25]
    pub keep_voltage_mv: u16,
    /// Delta-peak voltage in mV (for NiMH/NiCd charge termination, typ. 6mV)
    /// Encoded as single byte at `data[22]`
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
    /// Encoded at `data[20]` as single byte. Default 1, confirmed up to 2 in series 5.
    pub cycle_count: u8,
    /// eneloop cell type (0x94 byte 35), used only when `chemistry` is eneloop.
    pub eneloop_type: EneloopType,
}

/// The eneloop cell types the official app offers, in its order: the variant's value is
/// the 0x94 byte 35 it sends (the charger's own menu lists the same types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum EneloopType {
    #[default]
    StdAA = 0,
    StdAAA = 1,
    ProXXAA = 2,
    ProXXAAA = 3,
    LiteAA = 4,
    LiteAAA = 5,
    PlusAA = 6,
    StdC = 7,
    StdD = 8,
}

impl EneloopType {
    pub const ALL: [EneloopType; 9] = [
        EneloopType::StdAA,
        EneloopType::StdAAA,
        EneloopType::ProXXAA,
        EneloopType::ProXXAAA,
        EneloopType::LiteAA,
        EneloopType::LiteAAA,
        EneloopType::PlusAA,
        EneloopType::StdC,
        EneloopType::StdD,
    ];

    pub fn from_byte(byte: u8) -> Option<Self> {
        Self::ALL.get(usize::from(byte)).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            EneloopType::StdAA => "Std AA",
            EneloopType::StdAAA => "Std AAA",
            EneloopType::ProXXAA => "Pro/XX AA",
            EneloopType::ProXXAAA => "Pro/XX AAA",
            EneloopType::LiteAA => "Lite AA",
            EneloopType::LiteAAA => "Lite AAA",
            EneloopType::PlusAA => "Plus AA",
            EneloopType::StdC => "Std C",
            EneloopType::StdD => "Std D",
        }
    }

    /// The official app's defaults for the type: (capacity mAh, charge mA, discharge mA),
    /// i.e. the nominal capacity, 0.5 C and about 0.25 C.
    pub fn defaults(self) -> (u16, u16, u16) {
        match self {
            EneloopType::StdAA => (2000, 1000, 500),
            EneloopType::StdAAA => (800, 400, 200),
            EneloopType::ProXXAA => (2500, 1250, 600),
            EneloopType::ProXXAAA => (900, 450, 200),
            EneloopType::LiteAA => (1000, 500, 250),
            EneloopType::LiteAAA => (600, 300, 150),
            EneloopType::PlusAA => (2200, 1100, 550),
            EneloopType::StdC => (3200, 1600, 800),
            EneloopType::StdD => (6000, 3000, 1500),
        }
    }
}

impl std::fmt::Display for EneloopType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
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
            eneloop_type: EneloopType::StdAA,
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
    /// The 16-byte device code from the 0x57 reply (the official app sends it as hex to
    /// SkyRC's update server).
    pub code: [u8; 16],
    /// Code bytes 8-9 as hex: OEM code, "0000" for SkyRC's own brand.
    pub oem: String,
    /// e.g. "1.82" (code bytes 11-12: major, minor as a decimal number).
    pub firmware_version: String,
    /// The official app reads only the low nibble of code byte 13 (e.g. "1"; the charger's
    /// menu shows 1.0).
    pub hardware_version: String,
    /// Code bytes 0-5: the ASCII model ID, "100214" for every MC5000. Not a serial number;
    /// the field name is kept for API stability.
    pub serial_number: String,
    pub slot_count: u8,
}

impl MC5000DeviceInfo {
    /// Parse device info from 0x57 command response
    /// Response: `0f 13 57 00` + 16 code bytes + checksum (21 bytes), e.g.
    /// `0f 13 57 00 31 30 30 32 31 34 04 00 00 00 00 01 52 01 00 80 57`. Code bytes:
    /// 0-5 model ID, 8-9 OEM, 11-12 firmware, 13 hardware (low nibble), 15 check byte.
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
                    code: [0; 16],
                    oem: "Unknown".to_string(),
                    firmware_version: "Unknown".to_string(),
                    hardware_version: "Unknown".to_string(),
                    serial_number: "Unknown".to_string(),
                    slot_count: 4,
                })
            }
            CMD_DEVICE_INFO => {
                if response.len() < 21 {
                    return Err(BluetoothError::CommunicationError(
                        format!("Device info response too short: {} bytes", response.len())
                    ));
                }
                // The 16 code bytes follow `0f 13 57 00`.
                let code: [u8; 16] = response[4..20].try_into().expect("16 bytes");
                Ok(MC5000DeviceInfo {
                    code,
                    oem: format!("{:02X}{:02X}", code[8], code[9]),
                    firmware_version: format!("{}.{:02}", code[11], code[12]),
                    hardware_version: (code[13] & 0x0F).to_string(),
                    serial_number: String::from_utf8_lossy(&code[0..6]).to_string(),
                    slot_count: 4,
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

/// One frame ("page") of a 0x98 voltage-history reply:
/// `0f <len> 98 <slot bit> <pages> <page> <interval s> <samples: u16 LE mV>.. <sum>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    /// Slot 1-4.
    pub slot_id: u8,
    /// Pages in the whole reply, 1-3.
    pub total_pages: u8,
    /// This page, from 1.
    pub page: u8,
    /// Seconds between samples.
    pub interval_s: u8,
    /// Up to 100 smoothed voltage samples, oldest first.
    pub samples_mv: Vec<u16>,
}

impl HistoryPage {
    /// Parses a checksum-validated 0x98 frame.
    pub fn parse(frame: &[u8]) -> Result<Self, BluetoothError> {
        let bad = |why: &str| BluetoothError::CommunicationError(format!("0x98 page: {why}"));
        if frame.len() < 8 || frame[0] != PACKET_START || frame[2] != CMD_VOLTAGE_HISTORY {
            return Err(bad("not a history frame"));
        }
        if usize::from(frame[1]) + 2 != frame.len() || !(frame.len() - 8).is_multiple_of(2) {
            return Err(bad("length does not match"));
        }
        let slot_bit = frame[3];
        if !slot_bit.is_power_of_two() || slot_bit > 0x08 {
            return Err(bad("slot mask is not one slot"));
        }
        let (total_pages, page) = (frame[4], frame[5]);
        if total_pages == 0 || page == 0 || page > total_pages {
            return Err(bad("page numbers out of range"));
        }
        Ok(Self {
            slot_id: slot_bit.trailing_zeros() as u8 + 1,
            total_pages,
            page,
            interval_s: frame[6],
            samples_mv: frame[7..frame.len() - 1]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| u16::from_le_bytes(b))
                .collect(),
        })
    }
}

/// A slot's whole voltage history, as the charger keeps it for the program in progress.
///
/// Sample `i` (from 0) covers roughly the seconds up to `(i + 1) * interval_s` after the
/// program started. The charger smooths every sample (0.6 previous + 0.4 new reading) and,
/// each time its 300-sample buffer fills, averages neighbouring pairs and doubles the
/// interval, so the history always spans the whole program in fewer than 300 samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoltageHistory {
    pub slot_id: u8,
    pub interval_s: u8,
    pub samples_mv: Vec<u16>,
}

impl VoltageHistory {
    /// Joins the pages of one reply; `None` until every page from 1 to the total is there.
    pub fn from_pages(pages: &[HistoryPage]) -> Option<Self> {
        let first = pages.iter().find(|p| p.page == 1)?;
        let mut samples_mv = Vec::new();
        for n in 1..=first.total_pages {
            let page = pages.iter().find(|p| {
                p.page == n && p.slot_id == first.slot_id && p.total_pages == first.total_pages
            })?;
            samples_mv.extend_from_slice(&page.samples_mv);
        }
        Some(Self {
            slot_id: first.slot_id,
            interval_s: first.interval_s,
            samples_mv,
        })
    }
}

/// The charger's system settings from a 0x96 reply, as raw values: the official app reads
/// them into its settings screen but their value ranges are not documented. 0x96 only
/// reads; this client never writes them (0x95).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MC5000SystemSettings {
    pub system_beep: u8,
    pub notification_beep: u8,
    pub end_beep: u8,
    pub lcd: u8,
    pub warning: u8,
    pub sleep: u8,
}

impl MC5000SystemSettings {
    /// Parses `0f 09 96 <x> <beep> <notify> <end> <lcd> <warning> <sleep> <sum>`.
    pub fn parse_from_response(response: &[u8]) -> Result<Self, BluetoothError> {
        if response.len() < 11 || response[0] != PACKET_START || response[2] != CMD_SYSTEM_SETTINGS {
            return Err(BluetoothError::CommunicationError(format!(
                "Not a system settings reply: {response:02X?}"
            )));
        }
        Ok(Self {
            system_beep: response[4],
            notification_beep: response[5],
            end_beep: response[6],
            lcd: response[7],
            warning: response[8],
            sleep: response[9],
        })
    }
}

#[derive(Debug, Clone)]
pub struct MC5000SlotStatus {
    pub slot_id: u8,
    pub state: MC5000SlotState,
    pub voltage_mv: u16,
    pub current_ma: u16,
    pub capacity_mah: u32,
    /// Bytes 12-15: elapsed time of the program (big-endian u32 in the official app).
    pub elapsed_seconds: u32,
    pub resistance_milliohm: u16,
    /// Bytes 8-9: battery temperature in °C according to the official app; 0 = no reading
    /// (0 in every capture so far).
    pub temperature_c: u16,
    /// Byte 18: what the slot is doing (see [`phase`]). `None` for frames shorter than
    /// the 23-byte layout.
    pub phase: Option<u8>,
    /// Byte 19: the configured program, in the per-chemistry numbering of the 0x94
    /// config. Kept after a stop, so it says what was set up, not that it is running.
    pub mode: Option<OperationMode>,
    /// Byte 21: the configured chemistry (same codes as the 0x94 config).
    pub chemistry: Option<BatteryChemistry>,
    /// Byte 20: the charger's error code, 0 = none (see [`charger_error_text`]).
    pub error: Option<u8>,
}

/// Status byte 18 values (official app; 0, 2, 3 and 6 also seen on a live charger).
pub mod phase {
    pub const IDLE: u8 = 0;
    /// "Wait…": the program is about to start.
    pub const WAITING: u8 = 1;
    pub const CHARGING: u8 = 2;
    pub const DISCHARGING: u8 = 3;
    /// Rest between the stages of a multi-stage program.
    pub const RESTING: u8 = 4;
    pub const DONE: u8 = 5;
    pub const FINISHED: u8 = 6;
}

/// The official app's English text for a status byte 20 error code; `None` for 0 (no
/// error) and for codes the app does not know. Codes 9, 10 and 13 were seen in captures.
pub fn charger_error_text(code: u8) -> Option<&'static str> {
    Some(match code {
        1 => "Input voltage too low",
        2 => "Input voltage too high",
        3 | 9 => "Connection break",
        4 => "Capacity limit",
        5 => "Time limit",
        6 => "Internal temperature too high",
        7 => "Calibration failed",
        8 => "High internal resistance",
        10 => "Battery type error",
        11 => "Overload protection",
        12 => "Reversed polarity",
        13 => "Fully charged",
        _ => return None,
    })
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

        // Bytes 8-9: battery temperature (°C, big-endian; official app)
        let temperature_c = u16::from_be_bytes([response[8], response[9]]);

        // Elapsed time: bytes 12-15 (big-endian u32, seconds; official app). Bytes 12-13
        // are 0 below 18.2 h, which is why 14-15 alone used to look right.
        let elapsed_seconds = if response.len() >= 16 {
            u32::from_be_bytes([response[12], response[13], response[14], response[15]])
        } else {
            0
        };

        // Resistance: bytes 16-17 (big-endian, milliohms)
        let resistance_milliohm = if response.len() >= 18 {
            u16::from_be_bytes([response[16], response[17]])
        } else {
            0
        };

        // Bytes 18, 19, 21 (23-byte layout): phase, configured mode, configured chemistry.
        // Byte 18 was once read as a delta-V value; live testing of every mode showed it is
        // the slot's phase (see `phase`).
        let extended = response.len() >= 23;
        let phase = extended.then(|| response[18]);
        let error = extended.then(|| response[20]);
        let chemistry = extended
            .then(|| BatteryChemistry::from_protocol_byte(response[21]))
            .flatten();
        let mode = chemistry.and_then(|c| OperationMode::from_byte_for_chemistry(response[19], c));

        let state = if voltage_mv == 0 {
            MC5000SlotState::Empty
        } else {
            match phase {
                Some(phase::IDLE) => MC5000SlotState::Idle,
                Some(phase::CHARGING) => MC5000SlotState::Charging,
                Some(phase::DISCHARGING) => MC5000SlotState::Discharging,
                Some(phase::DONE | phase::FINISHED) => MC5000SlotState::Completed,
                // Waiting to start and resting between stages: the program is running.
                Some(phase::WAITING | phase::RESTING) => MC5000SlotState::Paused,
                // Unknown phase values: fall back to what the current says.
                Some(_) if current_ma > 0 => MC5000SlotState::Charging,
                Some(_) if elapsed_seconds > 0 => MC5000SlotState::Paused,
                Some(_) => MC5000SlotState::Idle,
                // Short frame (other firmware): current-based guess, direction unknown.
                None if current_ma > 0 => MC5000SlotState::Charging,
                None if elapsed_seconds > 0 => MC5000SlotState::Completed,
                None => MC5000SlotState::Idle,
            }
        };

        Ok(MC5000SlotStatus {
            slot_id,
            state,
            voltage_mv,
            current_ma,
            capacity_mah,
            elapsed_seconds,
            resistance_milliohm,
            temperature_c,
            phase,
            mode,
            chemistry,
            error,
        })
    }

    /// Whether a program is running on the slot (including a rest between its stages).
    pub fn is_running(&self) -> bool {
        matches!(
            self.state,
            MC5000SlotState::Charging
                | MC5000SlotState::ChargingCC
                | MC5000SlotState::ChargingCV
                | MC5000SlotState::Discharging
                | MC5000SlotState::Paused
        )
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_suffix_takes_the_trailing_hex_unit_id() {
        assert_eq!(MC5000Protocol::name_suffix("#Charger-FF34").as_deref(), Some("FF34"));
        assert_eq!(MC5000Protocol::name_suffix("Charger-e56a").as_deref(), Some("E56A"));
        assert_eq!(MC5000Protocol::name_suffix("Charger"), None);
        assert_eq!(MC5000Protocol::name_suffix("Charger-XYZ1"), None);
    }

    #[test]
    fn detects_chargers_by_name_regardless_of_unit_id() {
        let addr = btleplug::api::BDAddr::default();
        for name in ["#Charger-FF34", "#Charger-E56A", "Charger-E56A", "SkyRC MC5000"] {
            assert!(MC5000Protocol::is_mc5000_device(name, &addr), "{name}");
        }
        for name in ["Charger", "Phone Charger", "43\" Crystal UHD"] {
            assert!(!MC5000Protocol::is_mc5000_device(name, &addr), "{name}");
        }
    }

    fn parse(frame: &[u8]) -> MC5000SlotStatus {
        MC5000SlotStatus::parse_from_response(frame).expect("valid status frame")
    }

    // Frames received from a real charger while testing every mode.
    const LIION_DISCHARGING: [u8; 23] = [
        0x0F, 0x15, 0x91, 0x08, 0x01, 0xF8, 0x0F, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0x03, 0x00, 0x1F,
        0x03, 0x02, 0x00, 0x00, 0xC6,
    ];
    const NIMH_CYCLE_CHARGING: [u8; 23] = [
        0x0F, 0x15, 0x91, 0x02, 0x00, 0x8C, 0x05, 0x59, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0xE7,
        0x02, 0x04, 0x00, 0x03, 0x6E,
    ];
    const NIMH_DISCHARGE_FINISHED: [u8; 23] = [
        0x0F, 0x15, 0x91, 0x02, 0x00, 0x00, 0x04, 0x8F, 0, 0, 0, 0, 0, 0, 0, 0x0A, 0x00, 0xE6,
        0x06, 0x03, 0x00, 0x03, 0x22,
    ];
    const NIMH_IDLE_AFTER_STOP: [u8; 23] = [
        0x0F, 0x15, 0x91, 0x02, 0x00, 0x00, 0x04, 0xDB, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00, 0x00,
        0x00, 0x03, 0x00, 0x03, 0x78,
    ];

    #[test]
    fn status_reports_discharging_from_the_phase_byte() {
        let s = parse(&LIION_DISCHARGING);
        assert_eq!((s.slot_id, s.current_ma, s.voltage_mv), (4, 504, 4094));
        assert_eq!(s.state, MC5000SlotState::Discharging);
        assert_eq!(s.mode, Some(OperationMode::Discharge));
        assert_eq!(s.chemistry, Some(BatteryChemistry::LiIon));
        assert!(s.is_running());
    }

    #[test]
    fn status_decodes_the_nickel_mode_numbering() {
        let s = parse(&NIMH_CYCLE_CHARGING);
        assert_eq!(s.state, MC5000SlotState::Charging);
        assert_eq!(s.mode, Some(OperationMode::Cycle));
        assert_eq!(s.chemistry, Some(BatteryChemistry::NiMH));
    }

    #[test]
    fn status_reports_a_program_the_charger_finished() {
        let s = parse(&NIMH_DISCHARGE_FINISHED);
        assert_eq!(s.state, MC5000SlotState::Completed);
        assert_eq!(s.mode, Some(OperationMode::Discharge));
        assert!(!s.is_running());
    }

    #[test]
    fn idle_slot_still_reports_its_configured_mode() {
        let s = parse(&NIMH_IDLE_AFTER_STOP);
        assert_eq!(s.state, MC5000SlotState::Idle);
        assert_eq!(s.mode, Some(OperationMode::Discharge));
        assert!(!s.is_running());
    }

    #[test]
    fn mode_bytes_round_trip_for_each_chemistry_family() {
        use OperationMode::*;
        let families: [(&[BatteryChemistry], &[OperationMode]); 3] = [
            (
                &[
                    BatteryChemistry::LiIon,
                    BatteryChemistry::LiIonHV,
                    BatteryChemistry::LiFePO4,
                    BatteryChemistry::LTO,
                    BatteryChemistry::NaIon,
                ],
                &[Charge, Storage, Discharge, Cycle],
            ),
            (
                &[BatteryChemistry::NiMH, BatteryChemistry::NiCd, BatteryChemistry::Eneloop],
                &[Charge, Refresh, BreakIn, Discharge, Cycle],
            ),
            (&[BatteryChemistry::NiZn, BatteryChemistry::RAM], &[Charge, Discharge, Cycle]),
        ];
        let all = [Charge, Storage, Discharge, Cycle, Refresh, BreakIn];
        for (chemistries, modes) in families {
            for &chemistry in chemistries {
                for (byte, &mode) in modes.iter().enumerate() {
                    assert_eq!(mode.to_byte_for_chemistry(chemistry), Some(byte as u8), "{chemistry:?} {mode:?}");
                    assert_eq!(OperationMode::from_byte_for_chemistry(byte as u8, chemistry), Some(mode));
                }
                for mode in all.iter().filter(|m| !modes.contains(m)) {
                    assert_eq!(mode.to_byte_for_chemistry(chemistry), None, "{chemistry:?} has no {mode:?}");
                }
                assert_eq!(OperationMode::from_byte_for_chemistry(modes.len() as u8, chemistry), None);
            }
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// NiZn/RAM program on slot 1 with the parameters of the official app's captured frames.
    fn nizn_ram_config(chemistry: BatteryChemistry, mode: OperationMode) -> ChargeConfig {
        let (target, cutoff) = match chemistry {
            BatteryChemistry::NiZn => (1900, 1100),
            _ => (1650, 900),
        };
        ChargeConfig {
            discharge_current_ma: 1000,
            target_voltage_mv: target,
            cutoff_voltage_mv: cutoff,
            charge_cutoff_current_ma: 100,
            discharge_cutoff_current_ma: 100,
            trickle_charge_ma: 0,
            keep_voltage_mv: 0,
            delta_peak_mv: 6,
            cutoff_timer_min: 0,
            max_time_min: 300,
            cycle_direction: if chemistry == BatteryChemistry::NiZn && mode == OperationMode::Cycle { 1 } else { 0 },
            charge_resting_min: 10,
            discharge_resting_min: 10,
            cycle_count: if chemistry == BatteryChemistry::NiZn && mode == OperationMode::Cycle { 2 } else { 1 },
            ..ChargeConfig::new(1, chemistry, mode, 3000, 1000)
        }
    }

    /// Frames the official app sent for NiZn and RAM: Discharge is mode 1, Cycle mode 2.
    #[test]
    fn nizn_and_ram_configs_match_the_official_app_byte_for_byte() {
        let captured = [
            (BatteryChemistry::NiZn, OperationMode::Discharge,
             "0f2a94010103e803e80bb8076c044c00640064000a000a0100060000003c0000012c00060ce4000000000034"),
            (BatteryChemistry::NiZn, OperationMode::Cycle,
             "0f2a94010203e803e80bb8076c044c00640064000a000a0201060000003c0000012c00060ce4000000000037"),
            (BatteryChemistry::RAM, OperationMode::Discharge,
             "0f2a94010103e803e80bb80672038400640064000a000a0100060000003c0000012c00070ce4000000000071"),
            (BatteryChemistry::RAM, OperationMode::Cycle,
             "0f2a94010203e803e80bb80672038400640064000a000a0100060000003c0000012c00070ce4000000000072"),
        ];
        for (chemistry, mode, frame) in captured {
            let built = MC5000Protocol::build_charge_config_command(&nizn_ram_config(chemistry, mode)).unwrap();
            assert_eq!(built, hex(frame), "{chemistry:?} {mode:?}");
        }
    }

    /// A device-info reply captured from an MC5000 with firmware 1.82.
    #[test]
    fn device_info_reply_decodes_model_and_versions() {
        let info = MC5000DeviceInfo::parse_from_response(&hex(
            "0f1357003130303231340400000000015201008057",
        ))
        .unwrap();
        assert_eq!(info.serial_number, "100214");
        assert_eq!(info.oem, "0000");
        assert_eq!(info.firmware_version, "1.82");
        assert_eq!(info.hardware_version, "1");
        // The last code byte makes the 16 code bytes sum to 0 mod 256.
        let frame = hex("0f1357003130303231340400000000015201008057");
        assert_eq!(frame[4..20].iter().fold(0u8, |a, &b| a.wrapping_add(b)), 0);
    }

    /// Builds a 23-byte status frame with a valid checksum.
    fn status_frame(phase: u8, error: u8, temperature: u16, elapsed: u32) -> Vec<u8> {
        let mut f = vec![0x0F, 0x15, 0x91, 0x01, 0x00, 0x00, 0x05, 0x14];
        f.extend(temperature.to_be_bytes());
        f.extend([0x00, 0x10]); // capacity 16 mAh
        f.extend(elapsed.to_be_bytes());
        f.extend([0x00, 0x30, phase, 0x00, error, 0x03]); // R, phase, mode, error, NiMH
        let sum = f[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        f.push(sum);
        f
    }

    #[test]
    fn status_decodes_error_temperature_and_32_bit_elapsed_time() {
        // Captured: slot 1 holding a NiMH cell after a Li-Ion program was sent to it.
        let s = parse(&hex("0f159101009505890000000000000000000000000a00bf"));
        assert_eq!(s.error, Some(10));
        assert_eq!(charger_error_text(10), Some("Battery type error"));
        assert_eq!(charger_error_text(0), None);

        let s = parse(&status_frame(phase::CHARGING, 0, 31, 70_000));
        assert_eq!((s.temperature_c, s.elapsed_seconds, s.error), (31, 70_000, Some(0)));
    }

    #[test]
    fn waiting_and_resting_count_as_running_and_done_as_completed() {
        for (p, state) in [
            (phase::WAITING, MC5000SlotState::Paused),
            (phase::RESTING, MC5000SlotState::Paused),
            (phase::DONE, MC5000SlotState::Completed),
            (phase::FINISHED, MC5000SlotState::Completed),
        ] {
            let s = parse(&status_frame(p, 0, 0, 60));
            assert_eq!(s.state, state, "phase {p}");
            assert_eq!(s.is_running(), state == MC5000SlotState::Paused);
        }
    }

    /// A reply read from an MC5000 with firmware 1.83.
    #[test]
    fn system_settings_reply_decodes_raw_values() {
        let s = MC5000SystemSettings::parse_from_response(&hex("0f09960000000001010098")).unwrap();
        assert_eq!((s.system_beep, s.notification_beep, s.end_beep, s.lcd, s.warning, s.sleep), (0, 0, 0, 1, 1, 0));
        assert!(MC5000SystemSettings::parse_from_response(&hex("0f0465010167")).is_err());
        assert_eq!(MC5000Protocol::build_system_settings_query(), hex("0f0396049a"));
    }

    #[test]
    fn eneloop_type_goes_into_byte_35_for_eneloop_only() {
        let mut config = ChargeConfig::new(1, BatteryChemistry::Eneloop, OperationMode::Charge, 1000, 500);
        config.eneloop_type = EneloopType::LiteAA;
        let frame = MC5000Protocol::build_charge_config_command(&config).unwrap();
        assert_eq!((frame.len(), frame[3 + 35]), (44, 4));
        config.chemistry = BatteryChemistry::NiMH;
        let frame = MC5000Protocol::build_charge_config_command(&config).unwrap();
        assert_eq!(frame[3 + 35], 0, "other chemistries send 0");
        // The official app's order (its eneloop_mode list).
        let names: Vec<_> = EneloopType::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["Std AA", "Std AAA", "Pro/XX AA", "Pro/XX AAA", "Lite AA", "Lite AAA", "Plus AA", "Std C", "Std D"]);
        assert_eq!(EneloopType::from_byte(8), Some(EneloopType::StdD));
        assert_eq!(EneloopType::from_byte(9), None);
    }

    #[test]
    fn a_mode_the_chemistry_lacks_is_refused() {
        for (chemistry, mode) in [
            (BatteryChemistry::RAM, OperationMode::Storage),
            (BatteryChemistry::NiZn, OperationMode::Refresh),
            (BatteryChemistry::NiMH, OperationMode::Storage),
            (BatteryChemistry::LiIon, OperationMode::BreakIn),
        ] {
            let config = ChargeConfig::new(1, chemistry, mode, 2000, 500);
            assert!(MC5000Protocol::build_charge_config_command(&config).is_err(), "{chemistry:?} {mode:?}");
        }
    }

    /// Status frames the charger sent while running the programs above.
    #[test]
    fn nizn_and_ram_status_frames_decode_their_mode() {
        let cases = [
            ("0f1591010191043f0000000000000002014703010006bb", BatteryChemistry::NiZn, OperationMode::Discharge, MC5000SlotState::Discharging),
            ("0f159101000005340000000000000000012e0302000605", BatteryChemistry::NiZn, OperationMode::Cycle, MC5000SlotState::Discharging),
            ("0f15910102b3038c0000000000000004013b0301000721", BatteryChemistry::RAM, OperationMode::Discharge, MC5000SlotState::Discharging),
            ("0f159101000005310000000000000001013e0202000713", BatteryChemistry::RAM, OperationMode::Cycle, MC5000SlotState::Charging),
        ];
        for (frame, chemistry, mode, state) in cases {
            let s = parse(&hex(frame));
            assert_eq!((s.chemistry, s.mode, s.state), (Some(chemistry), Some(mode), state), "{frame}");
        }
    }

    #[test]
    fn short_frames_fall_back_to_the_current() {
        let s = parse(&LIION_DISCHARGING[..12]);
        assert_eq!(s.state, MC5000SlotState::Charging);
        assert_eq!((s.phase, s.mode), (None, None));
    }

    #[test]
    fn handshake_carries_the_unit_suffix() {
        let packet = MC5000Protocol::build_device_info_command("E56A");
        assert_eq!(&packet[..6], &[0x0F, 0x13, 0x57, 0x00, 0xE5, 0x6A]);
        assert_eq!(packet.len(), 21, "length byte 0x13 = 19 bytes after it");
        let sum = packet[2..packet.len() - 1].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        assert_eq!(packet[packet.len() - 1], sum);
    }

    /// The request the official app sent, byte for byte.
    #[test]
    fn handshake_matches_the_official_app() {
        assert_eq!(
            MC5000Protocol::build_device_info_command("FF34"),
            hex("0f135700ff343738c1a4000000000000000000005e")
        );
    }
    #[test]
    fn history_request_matches_the_official_app() {
        assert_eq!(
            MC5000Protocol::build_voltage_history_command(0x08),
            vec![0x0F, 0x03, 0x98, 0x08, 0xA0]
        );
    }

    #[test]
    fn history_pages_from_a_live_charger_parse_and_join() {
        let pages: Vec<HistoryPage> = test_frames::history_slot4()
            .iter()
            .map(|f| HistoryPage::parse(f).unwrap())
            .collect();
        assert_eq!(pages[0].slot_id, 4);
        assert_eq!((pages[0].total_pages, pages[0].page, pages[0].interval_s), (3, 1, 4));
        assert_eq!(&pages[0].samples_mv[..4], &[4145, 4172, 4192, 4198]);
        assert_eq!(pages.iter().map(|p| p.samples_mv.len()).collect::<Vec<_>>(), [100, 100, 23]);

        let history = VoltageHistory::from_pages(&pages).unwrap();
        assert_eq!((history.slot_id, history.interval_s, history.samples_mv.len()), (4, 4, 223));
        // Out of order is fine; a missing page is not.
        let reversed: Vec<_> = pages.iter().rev().cloned().collect();
        assert_eq!(VoltageHistory::from_pages(&reversed).as_ref(), Some(&history));
        assert_eq!(VoltageHistory::from_pages(&pages[..2]), None);
    }

    #[test]
    fn history_page_rejects_malformed_frames() {
        let mut frame = test_frames::history_slot4()[2].clone();
        frame[3] = 0x0C; // two slots
        assert!(HistoryPage::parse(&frame).is_err());
        let mut frame = test_frames::history_slot4()[2].clone();
        frame[5] = 4; // page 4 of 3
        assert!(HistoryPage::parse(&frame).is_err());
        assert!(HistoryPage::parse(&test_frames::history_slot4()[2][..53]).is_err());
    }
}

/// Frames received from a real charger, shared by the tests of this crate.
#[cfg(test)]
pub(crate) mod test_frames {
    /// The 0x98 reply for slot 4 (Li-Ion, charging at 4.20 V, 892 s into the program):
    /// pages of 100, 100 and 23 samples at 4 s. Pages 1 and 2 are rebuilt from their sample
    /// values and checked against the checksums the charger sent; page 3 is verbatim.
    pub(crate) fn history_slot4() -> Vec<Vec<u8>> {
        let page = |n: u8, samples: &[u16], checksum: u8| {
            let mut f = vec![0x0F, (samples.len() * 2 + 6) as u8, 0x98, 0x08, 0x03, n, 0x04];
            f.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
            let sum = f[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            assert_eq!(sum, checksum, "page {n} differs from the captured frame");
            f.push(sum);
            f
        };
        let mut first = vec![4145, 4172, 4192];
        first.extend([4198; 97]);
        let third = vec![
            0x0f, 0x34, 0x98, 0x08, 0x03, 0x03, 0x04, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66,
            0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66,
            0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66,
            0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x66, 0x10, 0x44,
        ];
        assert_eq!(page(3, &[4198; 23], 0x44), third);
        vec![page(1, &first, 0x6b), page(2, &[4198; 100], 0xc1), third]
    }
}
