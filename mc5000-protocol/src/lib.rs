//! MC5000 Multi-Slot Battery Charger Protocol Library
//!
//! This library provides a protocol implementation for communicating with
//! MC5000 battery chargers over Bluetooth (and potentially USB in the future).
//!
//! ## Features
//! - Bluetooth communication via BLE
//! - Support for 8 battery chemistries: Li-Ion, Li-Ion HV, LiFePO4, NiMH, NiCd, NiZn, LTO, RAM
//! - 6 operation modes: Charge, Storage, Discharge, Cycle, Refresh, BreakIn
//! - Real-time status monitoring
//! - Configuration of charge parameters

pub mod bluetooth;
pub mod device;

pub use bluetooth::{
    BluetoothError, DiscoveredBluetoothDevice, MC5000Protocol,
    BatteryChemistry, ChargeConfig, OperationMode,
    MC5000SlotStatus, MC5000SlotState, StartStopAction,
};

pub use device::{Device, DeviceManager, DeviceError, DeviceType, DeviceStatus};