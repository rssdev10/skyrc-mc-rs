//! Text control channel and JSON feedback for driving the real GUI without a mouse.
//!
//! Used for headless runs (`charger-controller -H`, invisible window) and for agents that
//! drive a visible window (`--control`). Every command becomes the same [`AppMessage`]s a
//! user's clicks produce, so the app's own `update()`, tasks, subscriptions and the shared
//! BLE session are what get exercised.
//!
//! Input: one command per line, from stdin or from a preset script. Output: one JSON
//! object per line on stdout (`state`, `event`, `ack`, `error`). See [`HELP`].

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

use crate::app::AppMessage;
use crate::config_dialog::ChargeMode;
use crate::slot::BatteryChemistry;

pub const HELP: &str = "\
Commands (one per line):
  scan                         rescan for chargers (Refresh button)
  devices                      list found chargers
  select <n|text>              pick a charger by list number (1-based) or name/ID text
  connect | disconnect         Connect / Disconnect buttons
  start <slot> [key=value...]  open Configure for <slot>, apply settings, press Confirm
                               keys: chemistry mode capacity current discharge target
                                     cutoff cycles
  configure <slot>             open the Configure dialog only
  set <key> <value>            change a field in the open dialog
  confirm | cancel             press Confirm / Cancel in the dialog
  stop [slot]                  Stop button of <slot>, or Stop All
  auto | smart                 Auto / Smart Charge buttons
  select-slot <slot>           select a slot for the graph
  state                        print the current state
  wait <seconds>               pause the script
  wait-until <condition> [s]   pause until a condition holds (default timeout 30 s):
                               scanned | connected | disconnected | readings |
                               slot <n> <state>
  watch <seconds> [count]      print the state every <seconds>
  quit                         exit
  help                         this text
Chemistries: liion liionhv lifepo4 nimh nicd eneloop nizn ram lto naion
Modes: charge storage discharge cycle refresh breakin";

/// Set when a scripted wait times out or a command fails; the process then exits with 1.
pub static FAILED: AtomicBool = AtomicBool::new(false);

/// Where control commands come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// Lines from standard input.
    Stdin,
    /// A fixed list of commands (headless subcommands).
    Script(Arc<Vec<String>>),
}

/// Control channel settings, chosen on the command line.
#[derive(Debug, Clone)]
pub struct ControlOptions {
    pub source: Source,
    /// Exit the app when the input ends.
    pub quit_at_end: bool,
}

/// What an agent sees of the app.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Snapshot {
    pub scanning: bool,
    pub connection: String,
    pub error: Option<String>,
    pub devices: Vec<String>,
    pub selected_device: Option<String>,
    pub connected_to: Option<String>,
    pub dialog_slot: Option<usize>,
    pub slots: Vec<SlotSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SlotSnapshot {
    pub slot: usize,
    pub state: String,
    /// True once the charger has reported this slot since connecting.
    pub seen: bool,
    /// Configured program as the charger reports it, e.g. "NiMH Cycle". Kept after a stop;
    /// `state` says whether it is running.
    pub mode: Option<String>,
    pub voltage: f32,
    pub current_ma: f32,
    pub capacity_mah: u32,
    pub resistance_mohm: u16,
    pub elapsed_s: u16,
    pub task: Option<String>,
    pub error: Option<String>,
    /// Message shown on the slot (why Auto/Smart skipped or limited the cell, a failure).
    pub notice: Option<String>,
}

/// Prints one JSON line: `{"type": kind, ...fields}`.
pub fn emit(kind: &str, fields: serde_json::Value) {
    let mut object = serde_json::Map::new();
    object.insert("type".into(), kind.into());
    if let serde_json::Value::Object(map) = fields {
        object.extend(map);
    }
    // One write per line: several tasks print to stdout.
    println!("{}", serde_json::Value::Object(object));
}

pub fn emit_state(snapshot: &Snapshot) {
    emit("state", serde_json::json!({ "state": snapshot }));
}

pub fn emit_error(message: &str) {
    FAILED.store(true, Ordering::SeqCst);
    emit("error", serde_json::json!({ "message": message }));
}

pub fn parse_chemistry(text: &str) -> Option<BatteryChemistry> {
    let key: String = text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    BatteryChemistry::all()
        .into_iter()
        .find(|c| format!("{c:?}").to_ascii_lowercase() == key)
}

pub fn parse_mode(text: &str) -> Option<ChargeMode> {
    let key: String = text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    [
        ChargeMode::Charge,
        ChargeMode::Storage,
        ChargeMode::Discharge,
        ChargeMode::Cycle,
        ChargeMode::Refresh,
        ChargeMode::BreakIn,
    ]
    .into_iter()
    .find(|m| format!("{m:?}").to_ascii_lowercase() == key)
}

/// The dialog message for `set <key> <value>`.
pub fn dialog_field(key: &str, value: &str) -> Result<AppMessage, String> {
    let value = value.to_string();
    Ok(match key {
        "chemistry" => AppMessage::ConfigChemistryChanged(
            parse_chemistry(&value).ok_or_else(|| format!("unknown chemistry '{value}'"))?,
        ),
        "mode" => AppMessage::ConfigModeChanged(
            parse_mode(&value).ok_or_else(|| format!("unknown mode '{value}'"))?,
        ),
        "capacity" => AppMessage::ConfigCapacityChanged(value),
        "current" => AppMessage::ConfigChargeCurrentChanged(value),
        "discharge" => AppMessage::ConfigDischargeCurrentChanged(value),
        "target" => AppMessage::ConfigTargetVoltageChanged(value),
        "cutoff" => AppMessage::ConfigCutoffVoltageChanged(value),
        "cycles" => AppMessage::ConfigCycleCountChanged(value),
        other => return Err(format!("unknown setting '{other}'")),
    })
}

/// Parses a slot number 1-4 into a 0-based index.
pub fn parse_slot(text: Option<&str>) -> Result<usize, String> {
    let text = text.ok_or("missing slot number")?;
    match text.parse::<usize>() {
        Ok(slot @ 1..=4) => Ok(slot - 1),
        _ => Err(format!("invalid slot '{text}': must be 1-4")),
    }
}

/// Subscription identity of the control channel (there is only ever one).
pub struct ControlSubscription {
    pub options: ControlOptions,
    pub snapshots: watch::Receiver<Snapshot>,
}

impl Hash for ControlSubscription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "mc5000_control".hash(state);
    }
}

enum Input {
    Stdin,
    Script(VecDeque<String>),
}

impl Input {
    async fn next_line(&mut self) -> Option<String> {
        match self {
            Input::Script(lines) => lines.pop_front(),
            Input::Stdin => tokio::task::spawn_blocking(|| {
                let mut buf = String::new();
                match std::io::stdin().read_line(&mut buf) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some(buf),
                }
            })
            .await
            .ok()
            .flatten(),
        }
    }
}

struct StreamState {
    input: Input,
    snapshots: watch::Receiver<Snapshot>,
    quit_at_end: bool,
    done: bool,
}

/// Reads commands and yields them as [`AppMessage::Control`]. Timing commands (`wait`,
/// `wait-until`, `watch`) run here, against the app's published snapshots.
pub fn control_stream(
    subscription: &ControlSubscription,
) -> impl futures::stream::Stream<Item = AppMessage> {
    let input = match &subscription.options.source {
        Source::Stdin => Input::Stdin,
        Source::Script(lines) => Input::Script(lines.iter().cloned().collect()),
    };
    let state = StreamState {
        input,
        snapshots: subscription.snapshots.clone(),
        quit_at_end: subscription.options.quit_at_end,
        done: false,
    };
    futures::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        loop {
            let Some(line) = state.input.next_line().await else {
                state.done = true;
                return state
                    .quit_at_end
                    .then(|| (AppMessage::Control("quit".into()), state));
            };
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            match words.next() {
                Some("wait") => {
                    let seconds = words
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(1.0);
                    tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.0))).await;
                    emit("ack", serde_json::json!({ "command": line }));
                }
                Some("wait-until") => {
                    let condition: Vec<&str> = words.collect();
                    if !wait_until(&mut state.snapshots, &condition).await {
                        emit_error(&format!("timed out: {line}"));
                        if state.quit_at_end {
                            state.done = true;
                            return Some((AppMessage::Control("quit".into()), state));
                        }
                    } else {
                        emit("ack", serde_json::json!({ "command": line }));
                    }
                }
                Some("watch") => {
                    let seconds = words
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(2.0);
                    let count = words.next().and_then(|s| s.parse::<u64>().ok());
                    let mut printed = 0;
                    while count.is_none_or(|c| printed < c) {
                        emit_state(&state.snapshots.borrow());
                        printed += 1;
                        tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.1))).await;
                    }
                }
                Some("quit" | "exit") => {
                    state.done = true;
                    return Some((AppMessage::Control(line), state));
                }
                _ => return Some((AppMessage::Control(line), state)),
            }
        }
    })
}

/// Waits until `condition` holds in the published snapshots, or its timeout passes.
async fn wait_until(snapshots: &mut watch::Receiver<Snapshot>, condition: &[&str]) -> bool {
    // An optional trailing number is the timeout in seconds.
    let (condition, timeout) = match condition.split_last() {
        Some((last, rest)) if last.parse::<f64>().is_ok() && !rest.is_empty() => {
            (rest, last.parse::<f64>().unwrap_or(30.0))
        }
        _ => (condition, 30.0),
    };
    let check = |s: &Snapshot| -> bool {
        match condition {
            ["scanned"] => !s.scanning,
            ["connected"] => s.connection == "connected",
            ["disconnected"] => s.connection != "connected" && s.connection != "connecting",
            ["readings"] => s.slots.iter().all(|slot| slot.seen),
            ["slot", n, state] => n
                .parse::<usize>()
                .ok()
                .and_then(|n| s.slots.get(n.wrapping_sub(1)))
                .is_some_and(|slot| slot.state.eq_ignore_ascii_case(state)),
            _ => false,
        }
    };
    let wait = snapshots.wait_for(check);
    matches!(
        tokio::time::timeout(Duration::from_secs_f64(timeout.max(0.0)), wait).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chemistry_and_mode_names_loosely() {
        assert_eq!(parse_chemistry("Li-Ion"), Some(BatteryChemistry::LiIon));
        assert_eq!(parse_chemistry("liion-hv"), Some(BatteryChemistry::LiIonHV));
        assert_eq!(parse_chemistry("NiMH"), Some(BatteryChemistry::NiMH));
        assert_eq!(parse_chemistry("unobtanium"), None);
        assert_eq!(parse_mode("break-in"), Some(ChargeMode::BreakIn));
        assert_eq!(parse_mode("discharge"), Some(ChargeMode::Discharge));
        assert_eq!(parse_mode("fast"), None);
    }

    #[test]
    fn parse_slot_accepts_one_to_four() {
        assert_eq!(parse_slot(Some("1")), Ok(0));
        assert_eq!(parse_slot(Some("4")), Ok(3));
        assert!(parse_slot(Some("0")).is_err());
        assert!(parse_slot(Some("5")).is_err());
        assert!(parse_slot(None).is_err());
    }

    #[test]
    fn dialog_fields_map_to_the_dialog_messages() {
        assert!(matches!(
            dialog_field("current", "1000"),
            Ok(AppMessage::ConfigChargeCurrentChanged(v)) if v == "1000"
        ));
        assert!(matches!(
            dialog_field("chemistry", "nimh"),
            Ok(AppMessage::ConfigChemistryChanged(BatteryChemistry::NiMH))
        ));
        assert!(dialog_field("colour", "red").is_err());
    }

    fn snapshot(scanning: bool, connection: &str, slot4_state: &str) -> Snapshot {
        Snapshot {
            scanning,
            connection: connection.into(),
            slots: (1..=4)
                .map(|slot| SlotSnapshot {
                    slot,
                    seen: true,
                    state: if slot == 4 {
                        slot4_state.into()
                    } else {
                        "idle".into()
                    },
                    ..SlotSnapshot::default()
                })
                .collect(),
            ..Snapshot::default()
        }
    }

    #[tokio::test]
    async fn wait_until_sees_a_later_state_change() {
        let (tx, mut rx) = watch::channel(snapshot(true, "disconnected", "idle"));
        let waiter = tokio::spawn(async move { wait_until(&mut rx, &["connected", "5"]).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send_replace(snapshot(false, "connected", "idle"));
        assert!(waiter.await.unwrap());
    }

    #[tokio::test]
    async fn wait_until_times_out() {
        let (_tx, mut rx) = watch::channel(snapshot(true, "disconnected", "idle"));
        assert!(!wait_until(&mut rx, &["connected", "0.05"]).await);
    }

    #[tokio::test]
    async fn wait_until_checks_a_slot_state() {
        let (_tx, mut rx) = watch::channel(snapshot(false, "connected", "charging"));
        assert!(wait_until(&mut rx, &["slot", "4", "Charging", "1"]).await);
        assert!(!wait_until(&mut rx, &["slot", "3", "charging", "0.05"]).await);
    }
}
