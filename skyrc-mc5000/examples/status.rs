//! Reads every slot and the voltage history of slot 1. Read only: safe while charging.
//!
//! `cargo run -p skyrc-mc5000 --example status`

use skyrc_mc5000::{ConnectMode, Session};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let charger = Session::scan(5)
        .await?
        .into_iter()
        .next()
        .ok_or("no MC5000 found")?;
    // Monitor: handshake only, safe while programs are running.
    let session = Session::connect(&charger, ConnectMode::Monitor).await?;

    for status in session
        .read_all_status(Duration::from_secs(2))
        .await
        .into_iter()
        .flatten()
    {
        println!(
            "slot {}: {:.3} V, {} mA",
            status.slot_id,
            status.voltage(),
            status.current_ma
        );
    }

    if let Ok(history) = session.read_history(1, Duration::from_secs(4)).await {
        println!(
            "{} samples, {} s apart",
            history.samples_mv.len(),
            history.interval_s
        );
    }

    session.disconnect().await?;
    Ok(())
}
