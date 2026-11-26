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
                               keys: chemistry eneloop mode capacity current discharge
                                     target cutoff cycles
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
                               slot <n> <state> | slot <n> running | all idle |
                               settled (every start/stop confirmed by the charger)
  watch <seconds> [count]      print the state every <seconds>
  quit                         exit
  help                         this text
Chemistries: liion liionhv lifepo4 nimh nicd eneloop nizn ram lto naion
eneloop types: std-aa std-aaa proxx-aa proxx-aaa lite-aa lite-aaa plus-aa std-c std-d
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
    /// Start/stop command sequences on their way to the charger.
    pub commands_in_flight: u32,
    /// Control commands the app has processed. Waits compare it with the number of commands
    /// sent, so they never look at a state from before the previous command.
    #[serde(skip)]
    pub control_seq: u64,
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
    pub elapsed_s: u32,
    /// A start or stop was sent and the charger has not reported the change yet (for at
    /// most 10 s).
    pub awaiting_charger: bool,
    /// Battery temperature the charger reports (°C); 0 = no reading.
    pub temperature_c: u16,
    /// The charger's error code for the slot (status byte 20), when not 0.
    pub charger_error: Option<u8>,
    pub task: Option<String>,
    pub error: Option<String>,
    /// Message shown on the slot (why Auto/Smart skipped or limited the cell, a failure).
    pub notice: Option<String>,
    /// Samples the last read of the charger's voltage history added to the graph.
    pub history_samples: usize,
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

/// An eneloop type by name ("lite-aa", "Pro/XX AA", ...) or by its index (0-8).
fn parse_eneloop(value: &str) -> Option<mc5000_protocol::EneloopType> {
    let key = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    let wanted = key(value);
    mc5000_protocol::EneloopType::ALL
        .into_iter()
        .find(|t| key(t.name()) == wanted || (*t as u8).to_string() == wanted)
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
        "eneloop" => AppMessage::ConfigEneloopTypeChanged(
            parse_eneloop(&value).ok_or_else(|| format!("unknown eneloop type '{value}'"))?,
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
    /// Control commands handed to the app so far.
    sent: u64,
}

impl StreamState {
    /// Hands `line` to the app.
    fn send(mut self, line: String) -> Option<(AppMessage, StreamState)> {
        self.sent += 1;
        Some((AppMessage::Control(line), self))
    }

    /// Waits until the app has processed every command sent so far, so a following wait
    /// sees their effect (e.g. `auto` then `wait-until settled`).
    async fn catch_up(&mut self) {
        let sent = self.sent;
        let _ = tokio::time::timeout(
            Duration::from_secs(30),
            self.snapshots.wait_for(|s| s.control_seq >= sent),
        )
        .await;
    }
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
        sent: 0,
    };
    futures::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        loop {
            let Some(line) = state.input.next_line().await else {
                state.done = true;
                return if state.quit_at_end { state.send("quit".into()) } else { None };
            };
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let command = words.next();
            if matches!(command, Some("wait" | "wait-until" | "watch")) {
                state.catch_up().await;
            }
            match command {
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
                            return state.send("quit".into());
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
                    return state.send(line);
                }
                _ => return state.send(line),
            }
        }
    })
}

fn slot<'a>(s: &'a Snapshot, n: &str) -> Option<&'a SlotSnapshot> {
    n.parse::<usize>().ok().and_then(|n| s.slots.get(n.wrapping_sub(1)))
}

/// A program runs on the slot (charging, discharging or resting between stages), as the
/// charger reports it.
fn is_running(slot: &SlotSnapshot) -> bool {
    matches!(slot.state.as_str(), "charging" | "discharging" | "paused")
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
            // Every command has reached the charger and every slot shows its outcome.
            ["settled"] => {
                s.commands_in_flight == 0 && s.slots.iter().all(|slot| !slot.awaiting_charger)
            }
            ["all", "idle"] => s.slots.iter().all(|slot| slot.seen && !is_running(slot)),
            ["slot", n, "running"] => slot(s, n).is_some_and(is_running),
            ["slot", n, state] => slot(s, n).is_some_and(|slot| slot.state.eq_ignore_ascii_case(state)),
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
    #[test]
    fn eneloop_types_parse_by_name_or_index() {
        use mc5000_protocol::EneloopType;
        assert_eq!(super::parse_eneloop("lite-aa"), Some(EneloopType::LiteAA));
        assert_eq!(super::parse_eneloop("Pro/XX AAA"), Some(EneloopType::ProXXAAA));
        assert_eq!(super::parse_eneloop("8"), Some(EneloopType::StdD));
        assert_eq!(super::parse_eneloop("pro"), None);
    }

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

    #[tokio::test]
    async fn running_covers_every_active_state() {
        for state in ["charging", "discharging", "paused"] {
            let (_tx, mut rx) = watch::channel(snapshot(false, "connected", state));
            assert!(wait_until(&mut rx, &["slot", "4", "running", "0.05"]).await, "{state}");
            assert!(!wait_until(&mut rx, &["all", "idle", "0.05"]).await, "{state}");
        }
        let (_tx, mut rx) = watch::channel(snapshot(false, "connected", "completed"));
        assert!(!wait_until(&mut rx, &["slot", "4", "running", "0.05"]).await);
        assert!(wait_until(&mut rx, &["all", "idle", "0.05"]).await);
    }

    #[tokio::test]
    async fn all_idle_needs_every_slot_reported() {
        let mut s = snapshot(false, "connected", "idle");
        s.slots[2].seen = false;
        let (_tx, mut rx) = watch::channel(s);
        assert!(!wait_until(&mut rx, &["all", "idle", "0.05"]).await);
    }

    /// `auto` then `wait-until settled`: the wait must not pass on the snapshot from before
    /// the app processed `auto` (nothing in flight yet), only once the app has caught up.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn waits_do_not_run_ahead_of_the_app() {
        use futures::StreamExt;
        let (tx, rx) = watch::channel(snapshot(false, "connected", "idle"));
        let subscription = ControlSubscription {
            options: ControlOptions {
                source: Source::Script(Arc::new(vec!["auto".into(), "wait-until settled 5".into(), "state".into()])),
                quit_at_end: false,
            },
            snapshots: rx,
        };
        let mut stream = Box::pin(control_stream(&subscription));
        assert!(matches!(stream.next().await, Some(AppMessage::Control(line)) if line == "auto"));

        // The next message comes only after the wait. Let it start, then let the app
        // "process" auto: one command in flight, so not settled yet.
        let next = tokio::spawn(async move { stream.next().await.map(|_| ()) });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!next.is_finished(), "settled must not pass before the app processed auto");
        let mut busy = snapshot(false, "connected", "idle");
        busy.control_seq = 1;
        busy.commands_in_flight = 1;
        tx.send_replace(busy.clone());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!next.is_finished(), "a command is still in flight");

        busy.commands_in_flight = 0;
        tx.send_replace(busy);
        tokio::time::timeout(Duration::from_secs(2), next).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn settled_waits_for_commands_and_charger_confirmations() {
        let mut busy = snapshot(false, "connected", "idle");
        busy.commands_in_flight = 1;
        let (tx, mut rx) = watch::channel(busy.clone());
        assert!(!wait_until(&mut rx, &["settled", "0.05"]).await);

        busy.commands_in_flight = 0;
        busy.slots[1].awaiting_charger = true; // sent, not yet reported by the charger
        tx.send_replace(busy.clone());
        assert!(!wait_until(&mut rx, &["settled", "0.05"]).await);

        busy.slots[1].awaiting_charger = false;
        tx.send_replace(busy);
        assert!(wait_until(&mut rx, &["settled", "0.05"]).await);
    }
}
