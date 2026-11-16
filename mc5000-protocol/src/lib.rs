//! MC5000 Multi-Slot Battery Charger Protocol Library
//!
//! This library provides a protocol implementation for communicating with
//! MC5000 battery chargers over Bluetooth Low Energy.
//!
//! ## Features
//! - Bluetooth communication via BLE
//! - Support for 10 battery chemistries: Li-Ion, Li-Ion HV, LiFePO4, NiMH, NiCd, eneloop,
//!   NiZn, RAM, LTO, Na-Ion
//! - 6 operation modes: Charge, Storage, Discharge, Cycle, Refresh, BreakIn
//! - [`Session`]: the shared connection used by every front end (paced polling,
//!   frame reassembly, command sequences)
//! - [`Transport`]: the link a session runs on. Bluetooth LE today; a wired (serial)
//!   placeholder is ready for when that protocol is known.

pub mod bluetooth;
pub mod session;
pub mod transport;

pub use bluetooth::{
    BluetoothError, DiscoveredBluetoothDevice, MC5000Protocol,
    BatteryChemistry, ChargeConfig, OperationMode,
    MC5000SlotStatus, MC5000SlotState, StartStopAction,
};
pub use session::{ConnectMode, Frame, FrameDecoder, Session, SessionEvent};
pub use transport::{BleTransport, IncomingBytes, SerialTransport, Transport};
