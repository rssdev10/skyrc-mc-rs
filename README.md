# MC5000 Charger Controller

A cross-platform Rust workspace for controlling the SkyRC MC5000 4-slot intelligent battery charger via Bluetooth Low Energy.

This project results from reverse‑engineering the SkyRC Android application's BLE protocol, with assistance from AI.


## Project Structure

This workspace consists of three crates:

### 1. `mc5000-protocol` - Protocol Library
Core library implementing the MC5000 Bluetooth communication protocol.

```bash
# Use as a dependency
[dependencies]
mc5000-protocol = { path = "../mc5000-protocol" }
```

**Features:**
- Bluetooth device scanning and connection
- Command encoding/decoding
- Status monitoring
- Charge configuration

### 2. `charger-controller` - GUI Application
Full-featured graphical interface for controlling the charger.

```bash
# Run the GUI
cargo run -p charger-controller
# or
./target/debug/charger-controller
```

**Features:**
- Real-time slot monitoring with graphs
- Visual configuration dialog
- Auto-charge mode with battery detection
- Data export to CSV

### 3. `charger-controller-cli` - Command Line Interface
Console application for scripting and headless operation.

```bash
# Run the CLI
cargo run -p charger-controller-cli -- --help
# or
./target/debug/charger-cli --help
```

**Commands:**
- `scan` - Scan for available devices
- `status [slot]` - Get status of slots
- `charge <slot> [options]` - Start charging
- `discharge <slot> [options]` - Start discharging
- `stop [slot]` - Stop charging/discharging (all slots if no slot given)
- `cycle <slot> [options]` - Charge/discharge cycles for capacity testing
- `refresh <slot> [options]` - Deep cycle restoration (NiMH/NiCd)
- `break-in <slot> [options]` - Conditioning cycles for new batteries
- `auto [slot]` - Auto-detect chemistry and start charging
- `monitor` - Continuous slot monitoring
- `debug` - Debug BLE communication and view raw notifications

## Battery Support

| Chemistry | Target Voltage | Cutoff Voltage |
|-----------|----------------|----------------|
| Li-Ion    | 4.20V          | 3.20V          |
| Li-Ion HV | 4.35V          | 3.40V          |
| LiFePO4   | 3.65V          | 2.90V          |
| NiMH      | 1.65V          | 0.90V          |
| NiCd      | 1.65V          | 0.90V          |
| eneloop   | 1.65V          | 0.90V          |
| NiZn      | 1.90V          | 1.10V          |
| RAM       | 1.65V          | 0.90V          |
| LTO       | 2.85V          | 1.80V          |
| Na-Ion    | 4.00V          | 2.00V          |

## Operation Modes

- **Charge** (0x00) - Standard charging
- **Storage** (0x01) - Charge/discharge to storage voltage
- **Discharge** (0x02) - Discharge battery
- **Cycle** (0x03) - Charge-discharge cycle for capacity testing
- **Refresh** (0x04) - Deep cycle for NiMH/NiCd
- **BreakIn** (0x05) - New battery conditioning

## Quick Start

### Building

```bash
# Build all crates
cargo build

# Build specific crate
cargo build -p mc5000-protocol
cargo build -p charger-controller
cargo build -p charger-controller-cli
```

### Running the GUI

```bash
cargo run -p charger-controller
```

### CLI Examples

```bash
# Scan for devices
charger-cli scan

# Get all slot status
charger-cli status

# Get status of slot 2
charger-cli status 2

# Charge slot 3 with Li-Ion at 2000mA
charger-cli charge 3 --chemistry liion --current 2000

# Charge slot 1 with NiMH at 1000mA
charger-cli charge 1 --chemistry nimh --current 1000

# Discharge slot 2 with Li-Ion at 500mA
charger-cli discharge 2 --chemistry liion --current 500

# Run cycle mode on slot 4 (Li-Ion)
charger-cli cycle 4 --chemistry liion --current 2000

# Run refresh mode on slot 3 (NiMH)
charger-cli refresh 3 --chemistry nimh --current 400 --discharge-current 250

# Break-in new NiMH batteries in slot 1
charger-cli break-in 1 --chemistry nimh --current 300

# Auto-detect battery chemistry and start charging all slots
charger-cli auto

# Auto-detect and charge slot 2
charger-cli auto 2

# Stop slot 2
charger-cli stop 2

# Stop all slots
charger-cli stop

# Continuous monitoring
charger-cli monitor
```

## Auto-Charge Feature

The auto-charge feature automatically:
1. Detects battery chemistry from voltage
2. Starts charging at 100mA initial current
3. Measures internal resistance
4. Adjusts current based on: `min(capacity × 0.5C, 300mV/R, 3000mA)`

```bash
# CLI
charger-cli auto

# Or in GUI, click the ⚡ Auto button
```

## Documentation

- **[docs/PROTOCOL.md](docs/PROTOCOL.md)** - BLE protocol specification

## Development

### Dependencies

- Rust 1.70+
- btleplug (Bluetooth)
- iced 0.12 (GUI)
- clap 4.5 (CLI)
- tokio (async runtime)

### Testing

```bash
# Run tests
cargo test

# Run with verbose output
MC5000_VERBOSE=1 cargo run -p charger-controller
```

## License

MIT
