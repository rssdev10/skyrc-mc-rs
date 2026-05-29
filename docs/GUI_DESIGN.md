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
├── subscription() → tick timer + BLE notification stream
└── theme()        → Theme::Dark
```

### State

- `ChargerApp` holds connection state, 4 slot states, data logger, config dialog state
- `ConnectionStatus`: Disconnected | Connecting | Connected | Error
- `SlotState`: Idle | Charging | Discharging | Completed | Paused | Error

### Messages

`AppMessage` enum (~50 variants) handles:
- Bluetooth lifecycle (scan, connect, disconnect, notifications)
- Slot operations (start/stop tasks, config changes)
- UI interactions (slot selection, config dialog, export)
- Background events (tick, notification data)

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

### Config Dialog (modal overlay)

Appears when configuring a slot. Fields:
- Chemistry (pick_list)
- Mode (pick_list: Charge, Discharge, Storage, Cycle)
- Capacity, charge/discharge current, voltages, timers

## Source Files

| File | Purpose |
|------|---------|
| `src/app.rs` | State machine, message handling, subscriptions |
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

1. **Tick** (1s interval): Polls slot status when connected
2. **BLE Notifications** (`Subscription::run_with`): Streams notification data from the charger peripheral

## Testing

Unit tests in `src/app.rs` (`#[cfg(test)] mod tests`) cover:
- Initial state validation
- Message handling (toggle, select, scan, config dialog)
- State conversion helpers
- No-panic guards for disconnected operations
