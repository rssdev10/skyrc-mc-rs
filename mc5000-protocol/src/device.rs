use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use crate::bluetooth::{MC5000Protocol, BluetoothError, DiscoveredBluetoothDevice};

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub port: String,
    pub device_type: DeviceType,
    pub status: DeviceStatus,
    pub bluetooth_protocol: Option<MC5000Protocol>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceType {
    MultiSlotCharger,
    BluetoothMC5000,
    // Can be extended for other device types
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceStatus {
    Connected,
    Disconnected,
    Error(String),
}

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Communication error: {0}")]
    CommunicationError(String),
    #[error("Device not found")]
    DeviceNotFound,
    #[error("Invalid response")]
    InvalidResponse,
    #[error("Bluetooth error: {0}")]
    BluetoothError(#[from] BluetoothError),
}

pub struct DeviceManager {
    available_devices: Vec<String>,
    #[allow(dead_code)]
    device_configs: HashMap<String, DeviceConfig>,
    bluetooth_devices: Vec<DiscoveredBluetoothDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub name: String,
    pub port: String,
    pub baud_rate: u32,
    pub protocol_version: String,
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceManager {
    pub fn new() -> Self {
        Self {
            available_devices: Self::scan_devices(),
            device_configs: HashMap::new(),
            bluetooth_devices: Vec::new(),
        }
    }

    pub fn get_available_devices(&self) -> &[String] {
        &self.available_devices
    }

    pub async fn scan_bluetooth_devices(&mut self) -> Result<(), DeviceError> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        if verbose {
            println!("[DEVICE VERBOSE] Starting Bluetooth scan (5 seconds)...");
        }
        
        match MC5000Protocol::scan_devices(5).await {
            Ok(devices) => {
                if verbose {
                    println!("[DEVICE VERBOSE] Found {} Bluetooth devices", devices.len());
                }
                
                self.bluetooth_devices = devices;
                // Add Bluetooth devices to available devices list
                let mut mc5000_devices = Vec::new();
                
                for bt_device in &self.bluetooth_devices {
                    if bt_device.is_mc5000 {
                        let device_name = format!("MC5000 BT: {} (ID:{})", bt_device.name, bt_device.id);
                        if verbose {
                            println!("[DEVICE VERBOSE]   - {} (RSSI: {:?})", bt_device.name, bt_device.rssi);
                        }
                        if !self.available_devices.contains(&device_name) {
                            mc5000_devices.push(device_name);
                        }
                    }
                }
                
                // Sort MC5000 devices first, then other devices
                for device in mc5000_devices {
                    if !self.available_devices.contains(&device) {
                        self.available_devices.push(device);
                    }
                }
                
                Ok(())
            }
            Err(e) => {
                if verbose {
                    println!("[DEVICE VERBOSE] Bluetooth scan failed: {}", e);
                }
                log::error!("Failed to scan Bluetooth devices: {}", e);
                Err(DeviceError::from(e))
            }
        }
    }

    /// Quick targeted scan for a previously connected device.
    /// Polls BLE for up to `timeout_secs` seconds, returning early if the
    /// target peripheral (identified by its `peripheral_id`) is found.
    /// When found, the device is added to `bluetooth_devices` and the
    /// formatted display name (`"MC5000 BT: <name> (ID:<id>)"`) is returned.
    /// Returns `Ok(None)` when the device was not seen — the caller should
    /// fall back to a full `scan_bluetooth_devices` scan.
    pub async fn quick_scan_for_device(&mut self, peripheral_id: &str, timeout_secs: u64) -> Result<Option<String>, DeviceError> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        if verbose {
            println!("[DEVICE VERBOSE] Quick scan for peripheral ID: {}", peripheral_id);
        }

        match MC5000Protocol::scan_for_device(peripheral_id, timeout_secs).await {
            Ok(Some(bt_device)) => {
                if verbose {
                    println!("[DEVICE VERBOSE] Quick scan found: {} ({})", bt_device.name, bt_device.id);
                }
                let display_name = format!("MC5000 BT: {} (ID:{})", bt_device.name, bt_device.id);
                if !self.available_devices.contains(&display_name) {
                    self.available_devices.push(display_name.clone());
                }
                // Replace or insert in bluetooth_devices so connect() can find the peripheral
                if let Some(existing) = self.bluetooth_devices.iter_mut().find(|d| d.id == bt_device.id) {
                    *existing = bt_device;
                } else {
                    self.bluetooth_devices.push(bt_device);
                }
                Ok(Some(display_name))
            }
            Ok(None) => {
                if verbose {
                    println!("[DEVICE VERBOSE] Quick scan: device not found within timeout");
                }
                Ok(None)
            }
            Err(e) => {
                if verbose {
                    println!("[DEVICE VERBOSE] Quick scan failed: {}", e);
                }
                Err(DeviceError::from(e))
            }
        }
    }

    pub async fn connect(&mut self, device_name: String) -> Result<Device, DeviceError> {
        // Check if this is a Bluetooth device
        if device_name.starts_with("MC5000 BT:") {
            return self.connect_bluetooth_device(&device_name).await;
        }

        // Original serial device connection logic
        if !self.available_devices.contains(&device_name) {
            return Err(DeviceError::DeviceNotFound);
        }

        // Simulate connection attempt
        match self.establish_connection(&device_name) {
            Ok(port) => {
                let device = Device {
                    name: device_name.clone(),
                    port,
                    device_type: DeviceType::MultiSlotCharger,
                    status: DeviceStatus::Connected,
                    bluetooth_protocol: None,
                };
                
                log::info!("Connected to device: {}", device_name);
                Ok(device)
            }
            Err(e) => {
                log::error!("Failed to connect to device {}: {}", device_name, e);
                Err(e)
            }
        }
    }

    async fn connect_bluetooth_device(&mut self, device_name: &str) -> Result<Device, DeviceError> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        // Extract peripheral ID from the formatted name
        // Format is: "MC5000 BT: <name> (ID:<peripheral_id>)"
        // We need to extract the peripheral ID after "ID:"
        let peripheral_id = device_name
            .find("ID:")
            .map(|start| {
                let id_start = start + 3;
                device_name[id_start..]
                    .find(')')
                    .map(|end| &device_name[id_start..id_start + end])
                    .unwrap_or("")
            })
            .unwrap_or("");
        
        // Extract the original device name (everything between "MC5000 BT: " and " (ID:")
        let selected_name = device_name
            .strip_prefix("MC5000 BT: ")
            .and_then(|s| s.find(" (ID:").map(|pos| &s[..pos]))
            .unwrap_or("MC5000");
        
        if verbose {
            println!("[DEVICE VERBOSE] Connecting to Bluetooth device: {} (ID:{})", selected_name, peripheral_id);
        }
        
        // Find the Bluetooth device by peripheral ID or name
        let bt_device = self.bluetooth_devices
            .iter()
            .find(|d| d.id == peripheral_id || d.name == selected_name)
            .ok_or(DeviceError::DeviceNotFound)?;

        if verbose {
            println!("[DEVICE VERBOSE] Found device: {}", bt_device.name);
            println!("[DEVICE VERBOSE] Creating protocol and connecting...");
        }

        // Create protocol and connect
        let mut protocol = MC5000Protocol::new();
        // Connect without init sequence to avoid disrupting running operations.
        // Init sequence will be sent on-demand before control commands.
        protocol.connect_without_init(&bt_device.peripheral).await?;
        
        if verbose {
            println!("[DEVICE VERBOSE] Verifying connection...");
        }
        
        // Verify connection
        if !protocol.is_connected().await {
            if verbose {
                println!("[DEVICE VERBOSE] Connection verification failed");
            }
            return Err(DeviceError::ConnectionFailed("Failed to establish Bluetooth connection".to_string()));
        }

        // Skip device info verification since MC5000 uses notifications
        // and we already know it's a MC5000 from the scan
        if verbose {
            println!("[DEVICE VERBOSE] ✓ Connection established successfully");
            println!("[DEVICE VERBOSE] Skipping device info query (MC5000 uses notifications)");
        }

        // Use the original selected name, not the peripheral's current name
        let device = Device {
            name: format!("MC5000 BT: {} (ID:{})", selected_name, peripheral_id),
            port: peripheral_id.to_string(),
            device_type: DeviceType::BluetoothMC5000,
            status: DeviceStatus::Connected,
            bluetooth_protocol: Some(protocol),
        };
        
        log::info!("Connected to Bluetooth MC5000: {} (ID:{})", selected_name, peripheral_id);
        Ok(device)
    }

    pub fn get_bluetooth_devices(&self) -> &[DiscoveredBluetoothDevice] {
        &self.bluetooth_devices
    }

    pub async fn refresh_all_devices(&mut self) -> Result<usize, DeviceError> {
        let verbose = std::env::var("MC5000_VERBOSE").is_ok();
        
        if verbose {
            println!("[DEVICE VERBOSE] Refreshing all devices...");
        }
        
        // Refresh serial devices
        self.available_devices = Self::scan_devices();
        
        // Refresh Bluetooth devices
        self.scan_bluetooth_devices().await?;
        
        if verbose {
            println!("[DEVICE VERBOSE] Total available devices: {}", self.available_devices.len());
        }
        
        Ok(self.available_devices.len())
    }

    fn scan_devices() -> Vec<String> {
        // Note: Serial port scanning is disabled, focusing on Bluetooth connections
        // To add serial port support, add the serialport crate and uncomment scanning code
        
        
        
        // Serial port scanning disabled - Bluetooth is the primary interface
        // Add mock devices for testing if needed
        // devices.push("Mock Charger #1".to_string());
        
        Vec::new()
    }

    #[allow(dead_code)]
    fn identify_device(port_name: &str) -> Result<String, DeviceError> {
        // In a real implementation, open the port and send identification command
        // For now, return a mock name
        Ok(format!("Multi-Slot Charger on {}", port_name))
    }

    fn establish_connection(&self, device_name: &str) -> Result<String, DeviceError> {
        // In a real implementation:
        // 1. Open serial port with correct settings
        // 2. Send handshake command
        // 3. Verify response
        // 4. Initialize device communication
        
        // Simulate connection delay and potential failure
        std::thread::sleep(std::time::Duration::from_millis(500));
        
        if device_name.contains("Mock") {
            Ok("/dev/ttyUSB0".to_string()) // Mock port
        } else {
            Err(DeviceError::ConnectionFailed("Device not responding".to_string()))
        }
    }
}

impl Device {
    pub fn send_command(&self, command: &[u8]) -> Result<Vec<u8>, DeviceError> {
        // In a real implementation, send command via serial port
        // and wait for response
        
        log::debug!("Sending command to {}: {:?}", self.name, command);
        
        // Mock response
        Ok(vec![0x00, 0x01, 0x02])
    }

    pub async fn send_command_async(&self, command: &[u8]) -> Result<Vec<u8>, DeviceError> {
        if let Some(ref protocol) = self.bluetooth_protocol {
            // Bluetooth communication
            protocol.send_command(command).await?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let response = protocol.read_response().await?;
            Ok(response)
        } else {
            // Fallback to synchronous method for serial devices
            self.send_command(command)
        }
    }

    pub async fn get_slot_status_async(&self, slot_id: u8) -> Result<crate::bluetooth::MC5000SlotStatus, DeviceError> {
        if let Some(ref protocol) = self.bluetooth_protocol {
            let status = protocol.get_slot_status(slot_id).await?;
            Ok(status)
        } else {
            Err(DeviceError::CommunicationError("Bluetooth protocol not available".to_string()))
        }
    }

    pub async fn start_charge_async(&self, slot_id: u8, voltage: f32, current: f32) -> Result<(), DeviceError> {
        if let Some(ref protocol) = self.bluetooth_protocol {
            protocol.start_charge(slot_id, voltage, current).await?;
            Ok(())
        } else {
            Err(DeviceError::CommunicationError("Bluetooth protocol not available".to_string()))
        }
    }

    pub async fn stop_slot_async(&self, slot_id: u8) -> Result<(), DeviceError> {
        if let Some(ref protocol) = self.bluetooth_protocol {
            protocol.stop_slot(slot_id).await?;
            Ok(())
        } else {
            Err(DeviceError::CommunicationError("Bluetooth protocol not available".to_string()))
        }
    }

    pub async fn is_connected_async(&self) -> bool {
        if let Some(ref protocol) = self.bluetooth_protocol {
            protocol.is_connected().await
        } else {
            true // Assume serial devices are connected if they exist
        }
    }

    pub fn read_slot_data(&self, slot_id: u8) -> Result<SlotData, DeviceError> {
        // In a real implementation, send specific command to read slot data
        
        Ok(SlotData {
            slot_id,
            voltage: 3.7 + (slot_id as f32 * 0.1),
            current: 1.0 + (slot_id as f32 * 0.2),
            temperature: 25.0,
            capacity_charged: 1500,
            time_elapsed: 3600,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SlotData {
    pub slot_id: u8,
    pub voltage: f32,
    pub current: f32,
    pub temperature: f32,
    pub capacity_charged: u32, // mAh
    pub time_elapsed: u32, // seconds
}