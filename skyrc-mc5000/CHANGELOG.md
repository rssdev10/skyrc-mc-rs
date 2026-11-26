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
