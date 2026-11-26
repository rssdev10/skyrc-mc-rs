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
│ Data panel │ Graph of the selected slot (fills the rest) │
│ (stats,    │                                              │
│  export)   │ [∞ Infinite] [Clear graph]  Recorded: 14m52s │
└──────────────────────────────────────────────────────────┘
```

- Slot panels with battery inserted (voltage > 0) are rendered with a lighter background.
- The ⚙ Settings button opens a full-page settings view (theme, BT persistence, about).
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
  Detection only commits where voltage is unambiguous (≥ 2.7 V Li-Ion, < 1.5 V NiMH) and never
  picks Li-Ion HV; other cells are skipped. Rules and thresholds: `src/auto_charge.rs`.
- When the charger reports an error code for a slot (status byte 20, e.g. "battery type error",
  "fully charged"), the slot's notice becomes "Charger reports: …". A new code replaces the
  notice; when the charger clears the code, that notice goes. Snapshots carry the raw code as
  `charger_error`.
- The slot header shows the battery temperature (°C) only when the charger reports one
  (snapshot field `temperature_c`). Elapsed time reads `mm:ss`, or `h:mm:ss` from one hour on.
  The charger's "waiting to start" and "resting between stages" phases show as Paused.
- For eneloop, the Configure dialog shows an **eneloop type** picker (Std AA … Std D). Picking a
  type fills in its capacity and charge/discharge currents (the official app's defaults) and
  the type is sent to the charger; it is kept across mode changes and saved with profiles. The
  control channel takes it as `eneloop=<type>` (e.g. `lite-aa`).
- The Configure dialog refuses Confirm, with the reason shown above its buttons, when the
  slot's measured voltage cannot belong to the chosen chemistry (for example NiMH settings on
  a 4.1 V cell). Completion is reported by the charger, never inferred by the GUI.
- Configure & Start and Stop buttons are disabled until the device confirms status via notification.
- Start and Stop never change a slot's state by themselves: the slot keeps its state, with its
  buttons disabled (for at most 10 s), until the charger's status reports it running or idle.
  A program the charger ends at once (e.g. battery type error) shows as idle at once.
  When Smart switches a slot from its probe to the chosen current, the charger stops and
  restarts the program; the idle report in between is not shown (for at most 10 s, and never
  after a Stop). A stop that cannot be delivered is queued and sent first on
  the next connection.
- Closing the window (or `quit` on the control channel) waits up to 20 s for commands still on
  their way to the charger, so a stop just pressed is delivered before the app disconnects. If
  a stop still could not be delivered, the app logs an error and emits a `stop_not_delivered`
  event.
- After Configure & Start completes, the started slot is auto-selected in the graph.

### Graph

Shows the selected slot's voltage (thin line, left axis, V) and current (bold line, right axis,
mA). Source: `src/ui/components/graph_panel.rs`; it works like the EL15 controller's chart.

- **The x axis is time.** Points are placed by timestamp, never by sample index.
- **Roll / Infinite** (toolbar toggle, saved in the settings): Roll shows the last N seconds
  (window field + Set, at least 10 s, default 600 s), labelled relative to now (`-2m05s` … `0`);
  a partly filled window draws on the right. Infinite shows everything recorded, labelled with
  wall-clock times, and holds still when no data arrives. Toggling never adds or removes data.
- **Clear graph** (red, like the other destructive buttons) hides what is recorded so far from the graph in both modes. It deletes
  nothing: CSV exports still contain it. The data panel's Clear Record Data deletes the data
  and resets this.
- The unit is written on the line of each scale's top value, in its trace's color: **V** left
  of the voltage value, **mA** right of the current value.
- **Scaling:** both axes fit the visible samples (5 % padding; at least 0.1 V and 100 mA tall,
  so a flat trace is not stretched). The current axis starts at 0.
- **Decimation:** at most two points (minimum and maximum) per pixel column, so the whole span
  is always drawn and a one-sample spike survives. A pause of more than 5 s in live data breaks
  the line; a lone reading between gaps is drawn as a dot.
- **Charger history:** when the app's own data of a program has a hole longer than 15 s (it
  connected after the start, or the link was down), it reads the charger's voltage history
  (0x98, read only) once per connection and program, and fills the hole with it. Those samples
  are drawn **dashed** and listed in the legend as "from the charger's memory"; they have no
  current (the charger sends none), so the current line has a gap there. They are coarser than
  live data (the charger keeps under 300 samples per program) and stay at least 10 s away from
  live samples. A finished program is filled only if the app saw it run, since its start time
  is otherwise unknown. Headless snapshots report the samples added as `history_samples`, and a
  `{"event": "history"}` line is emitted.
- The data model behind it (`src/data.rs`): one time-ordered sample list per slot. Each sample
  records its source (`live` or `charger_history`); the all-samples CSV has a `source` column,
  and an unknown current is exported as an empty cell.

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
| `src/ui/components/graph_panel.rs` | Time-axis voltage/current graph: Roll/Infinite, Clear, decimation |
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
