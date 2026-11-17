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

# Charger model, firmware/hardware versions, settings and slots (read only)
charger-cli info

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
| `mc5000-protocol` | `mc5000-protocol/src/` | `session.rs`: `Session`, the one connection every front end uses (connect with retry, paced polling, frame reassembly, start/stop sequences). `transport.rs`: the `Transport` trait a session runs on, with the Bluetooth LE implementation and a placeholder for a future wired (serial) link that fails until that protocol is known; a new link implements `Transport` and calls `Session::open`. `bluetooth.rs`: BLE transport (scan, connect, write, notifications), packet builders, checksum, response parsers, `BatteryChemistry`, `OperationMode`, `ChargeConfig`. The only crate that knows the wire format. |
| `charger-controller` | `charger-controller/src/` | iced GUI. `app.rs` is the state machine (`ChargerApp`, `AppMessage`, `update`, `subscription`); `ui/` holds views; `slot.rs`, `data.rs`, `export.rs` hold slot state, sample logging and CSV export; `settings.rs`, `profiles.rs`, `slot_persist.rs` persist user data via `confy`; `i18n.rs` plus `locales/*.json` provide translations. `control.rs` is the text control channel used by headless mode (see below). |
| `charger-controller-cli` | `charger-controller-cli/src/main.rs` | `charger-cli` binary (clap). One async function per subcommand, all through `Session`. |

Rules that follow from this split:

- Wire-format knowledge (command bytes, offsets, scaling, checksums, chemistry and mode bytes) lives
  only in `mc5000-protocol`. The GUI and CLI use its builders and parsers and never hand-assemble
  packets.
- All charger I/O goes through `mc5000_protocol::Session`: status (`read_status`,
  `start_polling`, `subscribe`), starting slots (`start_slots`) and stopping them (`stop_slots`).
  Do not open a second connection path or write to the peripheral directly.
- Reuse the existing enums and helpers (`BatteryChemistry`, `OperationMode`, `ChargeConfig`
  builders) instead of duplicating logic.
- Keep the public API of `mc5000-protocol` stable unless a breaking change is intended.

### Concurrency model

- **`Session`** (protocol crate, tokio) owns the BLE link and two background tasks: a reader that
  reassembles notifications into checksum-validated frames and broadcasts them, and an optional
  poller. Every write passes one async lock that keeps writes at least 120 ms apart (the charger
  drops requests sent closer together and never pushes status on its own); multi-step command
  sequences hold the lock throughout, so polling never interleaves with a start/stop sequence.
  `Session` is `Clone`; clones share the connection. Every write times out after 1 s; two
  timeouts in a row, or 10 s without a reply while polling, mark the link lost (reported once
  as `SessionEvent::Disconnected`), after which writes fail at once. Front ends then connect a
  new session: the GUI reconnects automatically, as does `charger-cli monitor`.
- **GUI**: iced runs `update()` and `view()` on the UI thread and never waits for the charger.
  Connect and every command run as `Task::perform` futures and report back as
  `AppMessage::Connected` / `AppMessage::CommandFinished`. Decoded statuses arrive through
  `ChargerApp::subscription()` as `AppMessage::StatusReceived`. The 1 s `Tick` only refreshes
  time-based UI; it sends nothing to the charger.
- **CLI**: one tokio runtime; each command connects a `Session`, uses it and disconnects. `monitor`
  runs the session's poller; Ctrl-C is signalled through an `AtomicBool`.

### Documentation map

- `docs/PROTOCOL.md` – authoritative BLE protocol reference. Update it together with any change to
  packet encoding or decoding.
- `docs/MODES.md` – default settings per chemistry and mode, recorded from the charger's own menu.
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
- Hardware checks run against a real MC5000: the CLI (`charger-cli status`, `charger-cli monitor`,
  `charger-cli debug`) or the GUI in headless mode (below).

### Headless mode: driving the GUI from scripts and agents

`charger-controller -H` runs the real GUI app with an invisible window and reads commands from
stdin; `--control` does the same with the window visible. Each command produces the same
`AppMessage`s a user's clicks do (for example `start` opens the Configure dialog, fills it in and
presses Confirm), so the GUI's own state machine, tasks and BLE session are what get tested.
Feedback is one JSON object per line on stdout: `state` snapshots, `event`s (`connected`,
`connect_failed`, `connection_lost`, `command_finished`, `scanned`, `quit_waiting`,
`stop_not_delivered`), `ack`s and `error`s. A failed `wait-until` makes the process exit with
status 1. After `stop`, `wait-until slot <n> idle` returns only once the charger reports the slot
idle, and `quit` waits for commands still on their way to the charger.

```bash
charger-controller -H status          # connect, print one state snapshot, quit
charger-controller -H monitor -i 2    # state snapshot every 2 s
printf 'wait-until scanned 20\nconnect\nwait-until connected 70\nwait-until readings\nstate\nquit\n' \
  | charger-controller -H
```

The subcommands (`scan`, `status`, `monitor`, `start`, `stop`, `auto`) are ready-made scripts in
the same command language. Type `help` in a session for the full list
(`charger-controller/src/control.rs`).

### Diagnostics

```bash
MC5000_VERBOSE=1 cargo run -p charger-controller   # verbose logging
MC5000_DEBUG=1   cargo run -p charger-controller   # raw packet dumps (implies verbose)
```

The GUI keeps `settings.toml`, `profiles.toml` and `slot_configs.toml` in:

- macOS: `~/Library/Application Support/com.mc5000.charger/` (the app's bundle identifier)
- Linux: `~/.config/mc5000/`
- Windows: `%APPDATA%\mc5000\config\`

`charger-controller/src/config_store.rs` owns these paths. Older macOS builds used
`~/Library/Application Support/rs.mc5000/`; files there are moved on first launch.

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
