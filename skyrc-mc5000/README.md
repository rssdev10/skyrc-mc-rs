# skyrc-mc5000

Unofficial Rust client for the **SkyRC MC5000** four-slot battery charger over Bluetooth LE.
Not affiliated with or endorsed by SkyRC.

- Protocol frames: builders and parsers for slot status, programs (all ten chemistries and
  their modes), stop, device info, system settings and the charger's voltage history.
- `Session`: the one connection an application needs. It paces writes (the charger drops
  requests sent too close together), reassembles frames split across notifications, polls
  the slots, keeps start and stop sequences from interleaving, and detects a lost link.
- `Transport`: the session runs on any link; Bluetooth LE (through `btleplug`) is built in,
  and an in-memory transport is enough for tests.
- `charging`: the chemistry detection and current rules used by the companion apps.

Verified against a real MC5000 with firmware 1.83. The full protocol description, with the
evidence for each field, is in
[docs/PROTOCOL.md](https://github.com/rssdev10/skyrc-mc-rs/blob/main/docs/PROTOCOL.md).

## Example

Read every slot and the voltage history of slot 1 (read only):

```rust,no_run
use std::time::Duration;
use skyrc_mc5000::{ConnectMode, Session};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let charger = Session::scan(5).await?.into_iter().next().ok_or("no MC5000 found")?;
    // Monitor: handshake only, safe while programs are running.
    let session = Session::connect(&charger, ConnectMode::Monitor).await?;

    for status in session.read_all_status(Duration::from_secs(2)).await.into_iter().flatten() {
        println!("slot {}: {:.3} V, {} mA", status.slot_id, status.voltage(), status.current_ma);
    }

    if let Ok(history) = session.read_history(1, Duration::from_secs(4)).await {
        println!("{} samples, {} s apart", history.samples_mv.len(), history.interval_s);
    }

    session.disconnect().await?;
    Ok(())
}
```

`Session::start_slots` and `Session::stop_slots` program and stop slots.

## Safety

This crate can push several amps into lithium and nickel cells. A wrong chemistry, current or
cut-off can overcharge or over-discharge a battery. The charger's own protections are the last
line of defence, not a substitute for correct settings. Only firmware 1.83 has been tested;
other firmware may differ. Do not leave charging cells unattended.

## Companion apps

The [repository](https://github.com/rssdev10/skyrc-mc-rs) also holds a GUI
(`charger-controller`) and a CLI (`charger-cli`) built on this crate, released as binaries.

## License

MIT
