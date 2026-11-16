# MC5000 Charger Controller — GUI Design

## Framework

- **iced 0.14** (Rust-native, Elm-architecture GUI)
- Function-based application API: `iced::application(boot, update, view)`
- Async runtime: Tokio (via iced's `tokio` feature)
- Canvas-based graph rendering (no plotters dependency)

## Architecture

```
ChargerApp (state machine)
├── new()          → initial state + startup Task
├── update()       → message handling, returns Task
├── view()         → UI layout (delegates to ui/ modules)
├── subscription() → UI tick + session status stream + control channel
└── theme()        → Theme::Dark
```

### State

- `ChargerApp` holds connection state, the scanned chargers, the shared `mc5000_protocol::Session`
  while connected, 4 slot states, data logger, config dialog state
- `ConnectionStatus`: Disconnected | Connecting | Connected | Error
- `SlotState`: Idle | Charging | Discharging | Completed | Paused | Error

### Messages

`AppMessage` enum (~50 variants) handles:
- Bluetooth lifecycle (scan, connect, disconnect): `BluetoothScanComplete`, `QuickScanResult`,
  `Connected`, `ConnectionLost`
- Slot operations (start/stop tasks, config changes); their outcome arrives as `CommandFinished`
- UI interactions (slot selection, config dialog, export)
- Background events: `Tick` (UI timer only), `StatusReceived` (decoded slot status)
- `Control` (one line from the headless control channel)

**Link recovery.** When the session reports the link lost (`ConnectionLost`), the app shows
`Reconnecting (attempt N)`, retries up to 5 times with growing delays (`ReconnectResult`), and
resumes polling. A Stop pressed while offline, or one that failed, is queued and sent first on
the next connection; starts are not retried. Stop All stays enabled while reconnecting.

`update()` never waits for the charger: connect and every start/stop sequence run as
`Task::perform` futures on the shared `Session` (`start_slots`, `stop_slots`). A failed command sets
the affected slots to `Error`.

## Layout

```
┌──────────────────────────────────────────────────────────┐
│ Device Panel (controls + status)              [⚙ Settings]│
├──────────────────────────────────────────────────────────┤
│ Charging Slots           [Auto] [SmartCharge] [Stop All] │
│ ┌─Slot 1─┐ ┌─Slot 2─┐ ┌─Slot 3─┐ ┌─Slot 4─┐           │
│ │ V/mA/W │ │ V/mA/W │ │ V/mA/W │ │ V/mA/W │           │
│ │Progress│ │Progress│ │Progress│ │Progress│           │
│ └────────┘ └────────┘ └────────┘ └────────┘           │
├──────────────────────────────────────────────────────────┤
│ Graph Panel (fills remaining space)                      │
└──────────────────────────────────────────────────────────┘
```

- Slot panels with battery inserted (voltage > 0) are rendered with a lighter background.
- The ⚙ Settings button opens a full-page settings view (theme, BT persistence, about).
- Graph voltage scale auto-adjusts to the selected slot's data range only.
- Auto and SmartCharge buttons are disabled for 5 seconds after clicking (cooldown).
- **Auto** detects each idle cell's chemistry from its resting voltage and starts charging right
  away at a fixed 500 mA (the Configure dialog's other defaults for that chemistry). **SmartCharge** detects the chemistry, runs a
  short low-current probe until the charger reports internal resistance, then charges at a
  current chosen from it (at most ~0.5 C of the smallest common cell: NiMH 400 mA, Li-Ion
  1000 mA, or 2000 mA below 25 mΩ for 21700-class cells). NiMH cells reading above 500 mΩ are
  charged gently at 200 mA and flagged as worn; cells above 1000 mΩ (NiMH) or 300 mΩ (Li-Ion)
  are stopped as broken or not rechargeable.
- Each slot can show a **notice** under its header: why Auto/Smart skipped or limited the cell
  (ambiguous voltage, worn cell, refused cell, no resistance reading) or why a command failed.
  It stays until something new starts on the slot or the cell is removed, so a status update
  cannot hide it. Headless snapshots carry it as `notice`.
  Detection only commits where voltage is unambiguous (≥ 3.7 V Li-Ion, < 1.5 V NiMH) and never
  picks Li-Ion HV; other cells are skipped. Rules and thresholds: `src/auto_charge.rs`.
- The Configure dialog refuses Confirm, with the reason shown above its buttons, when the
  slot's measured voltage cannot belong to the chosen chemistry (for example NiMH settings on
  a 4.1 V cell). Completion is reported by the charger, never inferred by the GUI.
- Configure & Start and Stop buttons are disabled until the device confirms status via notification.
- After Configure & Start completes, the started slot is auto-selected in the graph.

### Config Dialog (modal overlay)

Appears when configuring a slot. Fields:
- Chemistry (pick_list)
- Mode (pick_list: Charge, Discharge, Storage, Cycle)
- Capacity, charge/discharge current, voltages, timers

## Source Files

| File | Purpose |
|------|---------|
| `src/app.rs` | State machine, message handling, subscriptions |
| `src/control.rs` | Headless control channel: commands → `AppMessage`s, JSON feedback |
| `src/auto_charge.rs` | Chemistry detection, Smart current rule, voltage/chemistry guard |
| `src/ui/mod.rs` | Top-level view composition, header bar |
| `src/ui/components/device_panel.rs` | Bluetooth device list + connect button + settings gear |
| `src/ui/components/slot_panel.rs` | Per-slot status display + controls (highlights battery presence) |
| `src/ui/components/graph_panel.rs` | Canvas-based voltage/current graph |
| `src/ui/components/data_panel.rs` | Data statistics + export buttons |
| `src/ui/components/config_dialog.rs` | Battery configuration dialog |
| `src/ui/components/settings_dialog.rs` | Settings page (theme, BT persistence, about/repo info) |
| `src/ui/message.rs` | Re-exports AppMessage |
| `src/config_dialog.rs` | ChargeMode enum, ChargeConfig defaults |
| `src/settings.rs` | Persistent settings (confy-backed, platform config dir) |
| `src/slot.rs` | Slot, SlotState, TaskConfig, BatteryChemistry |
| `src/data.rs` | DataLogger for measurement history |
| `src/export.rs` | CSV export logic |

## Subscriptions

1. **Tick** (1 s interval): refreshes time-based UI (elapsed times, cooldowns). Sends nothing.
2. **Session status** (`Subscription::run_with`, keyed by the connection's generation): forwards
   decoded slot statuses from the session's broadcast as `StatusReceived`, and `ConnectionLost`
   when the link ends. Polling itself runs inside the session (one slot at a time, ≥120 ms apart,
   all four slots every second; slots that did not answer are re-requested).
3. **Control channel** (headless `-H` or `--control` only): reads commands from stdin or a
   subcommand script and yields `Control` messages; `wait`, `wait-until` and `watch` run inside the
   stream against the snapshots `update()` publishes.

## Headless mode

`charger-controller -H` runs this same app with an invisible window; commands are translated into
the messages the UI emits (see README, "Headless mode"). Use it to test GUI behaviour end to end,
including against a real charger.

## Testing

Unit tests in `src/app.rs` (`#[cfg(test)] mod tests`) cover:
- Initial state validation
- Message handling (toggle, select, scan, config dialog)
- State conversion helpers
- No-panic guards for disconnected operations
- Handling of decoded statuses (`StatusReceived`)

`src/control.rs` tests cover command parsing and the `wait-until` conditions.
