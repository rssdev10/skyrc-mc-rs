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

- Rust stable, edition 2021 (CI uses the latest stable toolchain)
- btleplug 0.13 (Bluetooth LE)
- iced 0.14 (GUI, function-based application API, `tokio` feature)
- clap 4 (CLI)
- tokio (async runtime)
- Linux builds need the system packages listed in `.github/workflows/tests.yml`
  (`libdbus-1-dev`, `libudev-dev`, X11/Wayland and font headers)

### Architecture

| Crate | Path | Responsibility |
|-------|------|----------------|
| `mc5000-protocol` | `mc5000-protocol/src/` | BLE transport (`bluetooth.rs`: scan, connect, write, notifications), packet builders, checksum, response parsers, `BatteryChemistry`, `OperationMode`, `ChargeConfig`; device discovery and connection management (`device.rs`). The only crate that knows the wire format. |
| `charger-controller` | `charger-controller/src/` | iced GUI. `app.rs` is the state machine (`ChargerApp`, `AppMessage`, `update`, `subscription`); `ui/` holds views; `slot.rs`, `data.rs`, `export.rs` hold slot state, sample logging and CSV export; `settings.rs`, `profiles.rs`, `slot_persist.rs` persist user data via `confy`; `i18n.rs` plus `locales/*.json` provide translations. `main.rs` also exposes a few headless subcommands. |
| `charger-controller-cli` | `charger-controller-cli/src/main.rs` | `charger-cli` binary (clap). One async function per subcommand. |

Rules that follow from this split:

- Wire-format knowledge (command bytes, offsets, scaling, checksums, chemistry and mode bytes) lives
  only in `mc5000-protocol`. The GUI and CLI use its builders and parsers and never hand-assemble
  packets.
- Reuse the existing enums and helpers (`BatteryChemistry`, `OperationMode`, `ChargeConfig`
  builders) instead of duplicating logic.
- Keep the public API of `mc5000-protocol` stable unless a breaking change is intended.

### Concurrency model

- **Protocol crate**: async (tokio). `MC5000Protocol` is `Clone` and wraps a btleplug
  `Peripheral`; clones share the same BLE connection. Notifications arrive on a stream that is
  consumed by a separate task from the one sending commands.
- **GUI**: iced runs `update()` and `view()` on the UI thread. BLE notifications enter through
  `ChargerApp::subscription()` as `AppMessage::NotificationReceived`. Long-running or awaiting work
  should be returned as `Task::perform(...)`. `ChargerApp` also owns a private tokio `Runtime`
  used with `block_on` for short command sequences; every `block_on` freezes the UI for its
  duration.
- **CLI**: a tokio runtime with a spawned notification task that writes into
  `Arc<Mutex<HashMap<u8, MC5000SlotStatus>>>`, read by the main task; Ctrl-C is signalled through
  an `AtomicBool`.

### Documentation map

- `docs/PROTOCOL.md` – authoritative BLE protocol reference. Update it together with any change to
  packet encoding or decoding.
- `modes.md` – operation modes and their parameters per chemistry.
- `docs/GUI_DESIGN.md` – GUI architecture, messages and layout. Update it whenever the GUI changes.
- `packaging/README.md` – release packaging layout.
- When README or docs show CLI usage, the argument names must match `charger-controller-cli`
  exactly.

### Testing and linting

```bash
cargo build --workspace
cargo test --workspace                                        # unit tests
cargo clippy --workspace --all-targets -- -D warnings         # lint (must be warning-free)
```

- Unit tests live next to the code in each crate (`#[cfg(test)] mod tests`). GUI state-machine
  tests are in `charger-controller/src/app.rs`.
- Tests must not need a Bluetooth adapter or a charger: CI runs on a headless Linux runner. Build
  packets and feed recorded or synthetic byte arrays to the parsers instead.
- Hardware checks are manual: run the CLI (`charger-cli status`, `charger-cli monitor`,
  `charger-cli debug`) or the GUI against a real MC5000.

### Diagnostics

```bash
MC5000_VERBOSE=1 cargo run -p charger-controller   # verbose logging
MC5000_DEBUG=1   cargo run -p charger-controller   # raw packet dumps (implies verbose)
```

### Continuous integration

- `.github/workflows/tests.yml` – build, test and clippy on every pull request and on pushes to `main`.
- `.github/workflows/release.yml` – release packaging (see below).

### Release Automation

- GitHub Actions workflow: `.github/workflows/release.yml`
- Manual pre-release build: run the `build and release` workflow with `workflow_dispatch`
- Release build: publishing a GitHub release triggers the same workflow and uploads assets to the release

Generated assets are separated into:

- `mc5000-protocol` crate package
- `charger-cli` portable archives for Linux, Windows, and macOS
- `charger-controller` portable archives for Linux and Windows
- `MC5000Charger.app` zipped bundle and `MC5000Charger.dmg` for macOS

Packaging metadata and auxiliary files live in `packaging/`, including the macOS bundling script at `packaging/macos/create_app_bundle.sh`.

## License

MIT
