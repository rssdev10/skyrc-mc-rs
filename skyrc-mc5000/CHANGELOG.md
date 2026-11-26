# Changelog

All notable changes to the `skyrc-mc5000` crate. The companion apps have their own release
notes on GitHub.

## Unreleased

## 0.1.0

First release under this name. Earlier versions were attached to the app releases as
`mc5000-protocol` (up to 0.3.1) and were not published on crates.io.

- `Session`: paced connection with polling, frame reassembly, start/stop sequences and
  lost-link detection, on any `Transport` (Bluetooth LE built in).
- Frame builders and parsers for slot status (0x91), programs (0x94), stop (0xFE), device
  info (0x57), system settings (0x96) and voltage history (0x98).
- All ten chemistries and their mode families; eneloop cell types.
- `charging`: chemistry detection and Auto/Smart current rules.

Compared with `mc5000-protocol` 0.3.1:

- Renamed: `MC5000SlotStatus`, `MC5000SlotState`, `MC5000DeviceInfo`,
  `MC5000SystemSettings`, `MC5000Protocol` are now `SlotStatus`, `SlotState`, `DeviceInfo`,
  `SystemSettings`, `Protocol`; `BluetoothError` is `Error`; `DiscoveredBluetoothDevice` is
  `DiscoveredCharger`; `DeviceInfo::serial_number` is `model_id`.
- Bluetooth LE is the default feature `ble`; without it the crate needs no Bluetooth stack.
- The crate no longer prints; it logs through `log` (sent and received frames at trace).
- It requires only tokio's `rt`, `sync` and `time` features.
