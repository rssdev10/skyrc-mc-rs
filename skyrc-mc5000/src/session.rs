//! The one way to talk to a connected MC5000. Both the GUI and the CLI use [`Session`].
//!
//! A session owns a connection (any [`Transport`]; today Bluetooth LE) and two
//! background tasks:
//!
//! - a **reader** that reassembles notification bytes into checksum-validated
//!   [`Frame`]s, records the latest status per slot and broadcasts [`SessionEvent`]s;
//! - an optional **poller** ([`Session::start_polling`]) that requests slot status one
//!   slot at a time and re-requests slots that did not answer.
//!
//! Every write goes through one async lock that also enforces a minimum gap between
//! writes. The charger never pushes status by itself and silently drops requests that
//! arrive too close together (measured: ~30% answered back-to-back, ~95% at 100 ms+).
//! Multi-step command sequences hold the lock for their whole duration, so polling
//! never interleaves with a start/stop sequence.

use crate::bluetooth::{
    BluetoothError, ChargeConfig, DiscoveredBluetoothDevice, HistoryPage, MC5000DeviceInfo,
    MC5000Protocol, MC5000SlotStatus, MC5000SystemSettings, StartStopAction, VoltageHistory,
    CMD_VOLTAGE_HISTORY,
};
use crate::transport::{BleTransport, IncomingBytes, Transport};
use btleplug::api::Peripheral as _;
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Minimum gap between two status requests.
pub const REQUEST_GAP: Duration = Duration::from_millis(120);
/// Gap between the packets of a command sequence (FE query, config, start/stop).
pub const COMMAND_GAP: Duration = Duration::from_millis(200);
/// Longest a single write may take. A write that never completes would otherwise hold the
/// write lock forever and block polling and stop commands (seen on a live charger: on
/// macOS a write-without-response can wait forever for CoreBluetooth's "ready to send"
/// callback). A healthy write is only handed to the system, so it completes at once.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Consecutive write timeouts after which the link is declared lost.
pub const MAX_WRITE_TIMEOUTS: u32 = 2;
/// While polling, a link that delivers nothing for this long is declared lost.
pub const LINK_STALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Time allowed for one connection attempt: BLE link, service discovery, handshake, init.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long [`Session::connect`] scans for a charger whose handle no longer works.
pub const RESCAN_TIMEOUT_SECS: u64 = 5;
/// Connection attempts before giving up. The link is torn down between attempts.
pub const CONNECT_ATTEMPTS: u32 = 2;

/// [`Session::read_history`] asks again when no page arrives for this long. The charger
/// sends pages about 100 ms apart.
pub const HISTORY_RETRY: Duration = Duration::from_millis(800);

const PACKET_START: u8 = 0x0F;
const CMD_CHANNEL_STATUS: u8 = 0x91;
/// Longest frame accepted from the charger. Longer length bytes are treated as noise,
/// except for 0x98 history pages, which carry up to 100 samples.
const MAX_FRAME_LEN: usize = 64;
/// Longest 0x98 history page: 100 samples (200 bytes) + 8.
const MAX_HISTORY_FRAME_LEN: usize = 208;

/// How a session connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectMode {
    /// Handshake plus the version and settings queries (0x74, 0x65). Leaves running
    /// programs untouched.
    Control,
    /// Handshake only. Safe while a program is running; status queries only.
    Monitor,
}

/// One checksum-validated packet received from the charger: `0F <len> <cmd> <data..> <sum>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame(Vec<u8>);

impl Frame {
    /// The raw bytes, including start marker, length and checksum.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// The command byte.
    pub fn command(&self) -> u8 {
        self.0[2]
    }

    /// The decoded slot status, if this is a 0x91 status frame for a known slot.
    pub fn status(&self) -> Option<MC5000SlotStatus> {
        if self.command() != CMD_CHANNEL_STATUS {
            return None;
        }
        MC5000SlotStatus::parse_from_response(&self.0)
            .ok()
            .filter(|s| (1..=4).contains(&s.slot_id))
    }
}

/// Reassembles notification payloads into frames.
///
/// BLE may split one frame across several notifications (for example with a 20-byte
/// payload limit), or pack several frames into one. Frames are delimited by their
/// length byte and accepted only when the checksum matches.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Feeds one notification payload and returns every complete frame it finished.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        // A notification that starts a new frame while an earlier one is still incomplete
        // means the earlier one lost its tail: drop it rather than corrupt the new one.
        if !self.buf.is_empty() && chunk.first() == Some(&PACKET_START) {
            let mut joined = self.buf.clone();
            joined.extend_from_slice(chunk);
            if !Self::starts_with_frame(&joined) {
                self.buf.clear();
            }
        }
        self.buf.extend_from_slice(chunk);

        let mut frames = Vec::new();
        loop {
            match self.buf.iter().position(|&b| b == PACKET_START) {
                Some(start) => {
                    self.buf.drain(..start);
                }
                None => {
                    self.buf.clear();
                    break;
                }
            }
            let total = match Self::frame_len(&self.buf) {
                FrameLen::Known(total) => total,
                FrameLen::NeedMore => break,
                FrameLen::Implausible => {
                    self.buf.drain(..1); // implausible length byte: resync
                    continue;
                }
            };
            if self.buf.len() < total {
                break; // wait for the rest of the frame
            }
            if Self::checksum_ok(&self.buf[..total]) {
                frames.push(Frame(self.buf.drain(..total).collect()));
            } else {
                self.buf.drain(..1); // not a real frame start: resync
            }
        }
        frames
    }

    /// Total frame length from the length byte, if plausible. A length only a history
    /// page can have is accepted once the command byte shows it is one.
    fn frame_len(buf: &[u8]) -> FrameLen {
        let Some(&len) = buf.get(1) else {
            return FrameLen::NeedMore;
        };
        let total = usize::from(len) + 2;
        if (4..=MAX_FRAME_LEN).contains(&total) {
            return FrameLen::Known(total);
        }
        if total > MAX_HISTORY_FRAME_LEN {
            return FrameLen::Implausible;
        }
        match buf.get(2) {
            None => FrameLen::NeedMore,
            Some(&CMD_VOLTAGE_HISTORY) => FrameLen::Known(total),
            Some(_) => FrameLen::Implausible,
        }
    }

    fn starts_with_frame(buf: &[u8]) -> bool {
        buf.first() == Some(&PACKET_START)
            && matches!(Self::frame_len(buf),
                FrameLen::Known(total) if buf.len() >= total && Self::checksum_ok(&buf[..total]))
    }

    /// Checksum: sum of command and data bytes, modulo 256.
    fn checksum_ok(frame: &[u8]) -> bool {
        let (body, sum) = frame[2..].split_at(frame.len() - 3);
        body.iter().fold(0u8, |acc, &b| acc.wrapping_add(b)) == sum[0]
    }
}

/// What a frame's length byte says.
enum FrameLen {
    Known(usize),
    /// Not enough bytes yet to judge the length.
    NeedMore,
    Implausible,
}

/// Something that happened on the connection.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// A frame arrived from the charger.
    Frame(Frame),
    /// The notification stream ended: the charger is gone.
    Disconnected,
}

/// A connected MC5000. Cheap to clone; all clones share one connection.
///
/// Dropping the last clone stops the background tasks and requests a disconnect.
/// Prefer an explicit [`Session::disconnect`], which waits for it.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

struct Inner {
    transport: Arc<dyn Transport>,
    name: String,
    id: String,
    generation: u64,
    /// Held for every write; stores when the last write finished.
    io: tokio::sync::Mutex<Option<Instant>>,
    events: broadcast::Sender<SessionEvent>,
    latest: Arc<Mutex<LatestStatus>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    poller: Mutex<Option<JoinHandle<()>>>,
    /// Set by an explicit [`Session::disconnect`]; dropping then needs no second one.
    disconnected: AtomicBool,
    /// Set once the link is gone (stream ended, or declared lost); writes then fail fast.
    lost: Arc<AtomicBool>,
    consecutive_timeouts: AtomicU32,
    /// When the reader last received anything.
    last_rx: Arc<Mutex<Instant>>,
    stall_timeout_ms: AtomicU64,
}

impl Drop for Inner {
    fn drop(&mut self) {
        abort(&self.reader);
        abort(&self.poller);
        if self.disconnected.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let transport = self.transport.clone();
            runtime.spawn(async move {
                let _ = transport.disconnect().await;
            });
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("name", &self.inner.name)
            .field("id", &self.inner.id)
            .field("generation", &self.inner.generation)
            .finish()
    }
}

/// Recovers the data of a poisoned lock instead of propagating one task's panic.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn abort(handle: &Mutex<Option<JoinHandle<()>>>) {
    if let Some(handle) = lock(handle).take() {
        handle.abort();
    }
}

fn slot_mask(slot: u8) -> Result<u8, BluetoothError> {
    if (1..=4).contains(&slot) {
        Ok(1 << (slot - 1))
    } else {
        Err(BluetoothError::CommunicationError(format!(
            "Invalid slot {slot}: must be 1-4"
        )))
    }
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Whether to print every received frame (`MC5000_DEBUG` set).
fn debug_frames() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MC5000_DEBUG").is_some())
}

/// Latest status per slot (index 0 = slot 1) and when it arrived.
type LatestStatus = [Option<(MC5000SlotStatus, Instant)>; 4];

impl Session {
    /// Scans for `timeout_secs` and returns the MC5000 chargers found.
    pub async fn scan(timeout_secs: u64) -> Result<Vec<DiscoveredBluetoothDevice>, BluetoothError> {
        let devices = MC5000Protocol::scan_devices(timeout_secs).await?;
        Ok(devices.into_iter().filter(|d| d.is_mc5000).collect())
    }

    /// Connects to `device`. If its handle no longer works, finds the charger again by its
    /// ID and connects to the fresh handle: macOS forgets a peripheral once it has been
    /// disconnected, so a handle from an earlier scan fails with "Peripheral no longer
    /// available" after a Disconnect or a lost link.
    pub async fn connect(
        device: &DiscoveredBluetoothDevice,
        mode: ConnectMode,
    ) -> Result<Self, BluetoothError> {
        let first_error = match Self::connect_with_retry(device, mode).await {
            Ok(session) => return Ok(session),
            Err(e) => e,
        };
        log::info!("Looking for {} again after: {first_error}", device.name);
        match MC5000Protocol::scan_for_device(&device.id, RESCAN_TIMEOUT_SECS).await {
            Ok(Some(found)) => Self::connect_with_retry(&found, mode).await,
            Ok(None) => Err(BluetoothError::ConnectionFailed(format!(
                "{first_error}; the charger was not found again"
            ))),
            Err(e) => Err(BluetoothError::ConnectionFailed(format!("{first_error}; {e}"))),
        }
    }

    /// One handle, retried once. A failed attempt is always followed by an explicit
    /// disconnect, so no half-open link is left behind (on Windows a stale link makes every
    /// later attempt fail).
    async fn connect_with_retry(
        device: &DiscoveredBluetoothDevice,
        mode: ConnectMode,
    ) -> Result<Self, BluetoothError> {
        let mut last_error = None;
        for attempt in 1..=CONNECT_ATTEMPTS {
            let attempt_result =
                tokio::time::timeout(CONNECT_TIMEOUT, BleTransport::connect(device, mode)).await;
            match attempt_result {
                Ok(Ok((transport, incoming))) => {
                    return Ok(Self::open(transport, incoming, &device.name, &device.id));
                }
                Ok(Err(e)) => last_error = Some(e),
                Err(_) => {
                    last_error = Some(BluetoothError::ConnectionFailed(format!(
                        "Connection timeout ({} s)",
                        CONNECT_TIMEOUT.as_secs()
                    )))
                }
            }
            log::warn!(
                "Connect attempt {attempt}/{CONNECT_ATTEMPTS} to {} failed: {:?}",
                device.name,
                last_error
            );
            let _ = device.peripheral.disconnect().await;
            if attempt < CONNECT_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(last_error.unwrap_or_else(|| BluetoothError::ConnectionFailed("Not attempted".into())))
    }

    /// Starts a session on an already connected transport. `incoming` carries the raw
    /// bytes the charger sends; `name` and `id` identify it. This is how any link type,
    /// not only Bluetooth, gets framing, pacing, polling and command sequences.
    /// Must be called inside a tokio runtime.
    pub fn open(
        transport: impl Transport,
        mut incoming: IncomingBytes,
        name: &str,
        id: &str,
    ) -> Self {
        let (events, _) = broadcast::channel(256);
        let latest = Arc::new(Mutex::new([None, None, None, None]));

        let lost = Arc::new(AtomicBool::new(false));
        let last_rx = Arc::new(Mutex::new(Instant::now()));
        let reader = {
            let events = events.clone();
            let latest = latest.clone();
            let lost = lost.clone();
            let last_rx = last_rx.clone();
            tokio::spawn(async move {
                let mut decoder = FrameDecoder::default();
                while let Some(chunk) = incoming.next().await {
                    *lock(&last_rx) = Instant::now();
                    for frame in decoder.push(&chunk) {
                        if debug_frames() {
                            println!("[DEBUG] RX frame: {:02X?}", frame.bytes());
                        }
                        if let Some(status) = frame.status() {
                            let index = usize::from(status.slot_id - 1);
                            lock(&latest)[index] = Some((status, Instant::now()));
                        }
                        // No receivers is fine: nobody is listening right now.
                        let _ = events.send(SessionEvent::Frame(frame));
                    }
                }
                if !lost.swap(true, Ordering::SeqCst) {
                    let _ = events.send(SessionEvent::Disconnected);
                }
            })
        };

        Self {
            inner: Arc::new(Inner {
                transport: Arc::new(transport),
                name: name.to_string(),
                id: id.to_string(),
                generation: GENERATION.fetch_add(1, Ordering::SeqCst),
                io: tokio::sync::Mutex::new(None),
                events,
                latest,
                reader: Mutex::new(Some(reader)),
                poller: Mutex::new(None),
                disconnected: AtomicBool::new(false),
                lost,
                consecutive_timeouts: AtomicU32::new(0),
                last_rx,
                stall_timeout_ms: AtomicU64::new(LINK_STALL_TIMEOUT.as_millis() as u64),
            }),
        }
    }

    /// Advertised name of the charger.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// ID of the charger (the BLE peripheral ID for Bluetooth).
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// Unique per connection; changes on every reconnect.
    pub fn generation(&self) -> u64 {
        self.inner.generation
    }

    /// Subscribes to everything received from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.inner.events.subscribe()
    }

    /// The most recent status of `slot` (1-4) and when it arrived.
    pub fn latest_status(&self, slot: u8) -> Option<(MC5000SlotStatus, Instant)> {
        let index = usize::from(slot.checked_sub(1)?);
        lock(&self.inner.latest).get(index).cloned().flatten()
    }

    /// Mask of slots whose latest status shows a running program.
    pub fn active_mask(&self) -> u8 {
        (1..=4u8)
            .filter(|&slot| {
                self.latest_status(slot)
                    .is_some_and(|(s, _)| s.is_running())
            })
            .fold(0, |mask, slot| mask | 1 << (slot - 1))
    }

    /// Whether the BLE link is still up.
    pub async fn is_connected(&self) -> bool {
        self.inner.transport.is_connected().await
    }

    /// Sends one packet, keeping at least [`REQUEST_GAP`] after the previous write.
    pub async fn send(&self, packet: &[u8]) -> Result<(), BluetoothError> {
        self.send_sequence(&[packet.to_vec()], REQUEST_GAP).await
    }

    /// Sends packets in order, `gap` apart, with no other write in between.
    pub async fn send_sequence(
        &self,
        packets: &[Vec<u8>],
        gap: Duration,
    ) -> Result<(), BluetoothError> {
        let mut last_write = self.inner.io.lock().await;
        for packet in packets {
            if self.is_lost() {
                return Err(BluetoothError::ConnectionFailed(
                    "Link to the charger was lost".into(),
                ));
            }
            if let Some(wait) = last_write.map(|t| gap.saturating_sub(t.elapsed())) {
                tokio::time::sleep(wait).await;
            }
            let result =
                match tokio::time::timeout(WRITE_TIMEOUT, self.inner.transport.write(packet)).await
                {
                    Ok(result) => result,
                    Err(_) => {
                        log::warn!("Write of {packet:02X?} timed out");
                        let timeouts =
                            self.inner.consecutive_timeouts.fetch_add(1, Ordering::SeqCst) + 1;
                        if timeouts >= MAX_WRITE_TIMEOUTS {
                            self.mark_lost(&format!("{timeouts} writes in a row timed out"));
                        }
                        Err(BluetoothError::CommunicationError(format!(
                            "Write timed out after {} ms",
                            WRITE_TIMEOUT.as_millis()
                        )))
                    }
                };
            *last_write = Some(Instant::now());
            if result.is_ok() {
                self.inner.consecutive_timeouts.store(0, Ordering::SeqCst);
            }
            result?;
        }
        Ok(())
    }

    /// Requests the status of `slot` (1-4). The answer arrives as an event.
    pub async fn request_status(&self, slot: u8) -> Result<(), BluetoothError> {
        let mask = slot_mask(slot)?;
        self.send(&MC5000Protocol::build_channel_status_command(mask))
            .await
    }

    /// Requests the status of `slot` (1-4) and waits for a fresh answer, re-requesting
    /// every 500 ms until `timeout`.
    pub async fn read_status(
        &self,
        slot: u8,
        timeout: Duration,
    ) -> Result<MC5000SlotStatus, BluetoothError> {
        let mut events = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            self.request_status(slot).await?;
            let retry_at = deadline.min(tokio::time::Instant::now() + Duration::from_millis(500));
            loop {
                match tokio::time::timeout_at(retry_at, events.recv()).await {
                    Ok(Ok(SessionEvent::Frame(frame))) => {
                        if let Some(status) = frame.status().filter(|s| s.slot_id == slot) {
                            return Ok(status);
                        }
                    }
                    Ok(Ok(SessionEvent::Disconnected))
                    | Ok(Err(broadcast::error::RecvError::Closed)) => {
                        return Err(BluetoothError::ConnectionFailed(
                            "Charger disconnected".into(),
                        ));
                    }
                    Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                    Err(_) => break,
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BluetoothError::CommunicationError(format!(
                    "No status from slot {slot} within {} ms",
                    timeout.as_millis()
                )));
            }
        }
    }

    /// Sends `packet` and waits for the first frame whose command is `reply`, sending again
    /// every 500 ms until `timeout`. Subscribes before sending, so a fast reply is not lost.
    pub async fn request(
        &self,
        packet: &[u8],
        reply: u8,
        timeout: Duration,
    ) -> Result<Frame, BluetoothError> {
        let mut events = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            self.send(packet).await?;
            let retry_at = deadline.min(tokio::time::Instant::now() + Duration::from_millis(500));
            loop {
                match tokio::time::timeout_at(retry_at, events.recv()).await {
                    Ok(Ok(SessionEvent::Frame(frame))) if frame.command() == reply => {
                        return Ok(frame)
                    }
                    Ok(Ok(SessionEvent::Frame(_))) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                    Ok(Ok(SessionEvent::Disconnected))
                    | Ok(Err(broadcast::error::RecvError::Closed)) => {
                        return Err(BluetoothError::ConnectionFailed("Charger disconnected".into()));
                    }
                    Err(_) => break,
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BluetoothError::CommunicationError(format!(
                    "No 0x{reply:02X} reply within {} ms",
                    timeout.as_millis()
                )));
            }
        }
    }

    /// Asks the charger for its device info (0x57): model ID, firmware and hardware
    /// versions. Read only. The charger answers whatever address the request carries.
    pub async fn device_info(&self, timeout: Duration) -> Result<MC5000DeviceInfo, BluetoothError> {
        let suffix = MC5000Protocol::name_suffix(self.name()).unwrap_or_default();
        let packet = MC5000Protocol::build_device_info_command(&suffix);
        let frame = self.request(&packet, 0x57, timeout).await?;
        MC5000DeviceInfo::parse_from_response(frame.bytes())
    }

    /// Reads the charger's system settings (0x96). Read only.
    pub async fn system_settings(
        &self,
        timeout: Duration,
    ) -> Result<MC5000SystemSettings, BluetoothError> {
        let packet = MC5000Protocol::build_system_settings_query();
        let frame = self.request(&packet, 0x96, timeout).await?;
        MC5000SystemSettings::parse_from_response(frame.bytes())
    }

    /// Reads the voltage history of `slot` (1-4) for the program in progress (0x98). Read
    /// only. The charger sends up to three pages about 100 ms apart; the request is sent
    /// again when nothing arrives for [`HISTORY_RETRY`], which restarts the reply. A slot
    /// with no history never answers, so that ends in a timeout error.
    pub async fn read_history(
        &self,
        slot: u8,
        timeout: Duration,
    ) -> Result<VoltageHistory, BluetoothError> {
        let packet = MC5000Protocol::build_voltage_history_command(slot_mask(slot)?);
        let mut events = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut pages: Vec<HistoryPage> = Vec::new();
        loop {
            self.send(&packet).await?;
            loop {
                let retry_at = deadline.min(tokio::time::Instant::now() + HISTORY_RETRY);
                match tokio::time::timeout_at(retry_at, events.recv()).await {
                    Ok(Ok(SessionEvent::Frame(frame))) if frame.command() == CMD_VOLTAGE_HISTORY => {
                        let Ok(page) = HistoryPage::parse(frame.bytes()) else { continue };
                        if page.slot_id != slot {
                            continue;
                        }
                        if page.page == 1 {
                            pages.clear(); // a new reply (after a resend) starts over
                        }
                        pages.push(page);
                        if let Some(history) = VoltageHistory::from_pages(&pages) {
                            return Ok(history);
                        }
                    }
                    Ok(Ok(SessionEvent::Frame(_))) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
                    Ok(Ok(SessionEvent::Disconnected))
                    | Ok(Err(broadcast::error::RecvError::Closed)) => {
                        return Err(BluetoothError::ConnectionFailed("Charger disconnected".into()));
                    }
                    Err(_) => break,
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BluetoothError::CommunicationError(format!(
                    "No voltage history from slot {slot} within {} ms",
                    timeout.as_millis()
                )));
            }
        }
    }

    /// Reads all four slots, one after another. Slots that never answer are `None`.
    pub async fn read_all_status(
        &self,
        timeout_per_slot: Duration,
    ) -> [Option<MC5000SlotStatus>; 4] {
        let mut result = [None, None, None, None];
        for (index, entry) in result.iter_mut().enumerate() {
            *entry = self
                .read_status(index as u8 + 1, timeout_per_slot)
                .await
                .ok();
        }
        result
    }

    /// Starts background polling: every `interval`, request each slot once, then re-request
    /// any slot that did not answer. Replaces a poller that is already running.
    pub fn start_polling(&self, interval: Duration) {
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let handle = tokio::spawn(async move {
            loop {
                let round_start = Instant::now();
                for slot in 1..=4u8 {
                    let Some(session) = upgrade(&weak) else {
                        return;
                    };
                    let _ = session.request_status(slot).await;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                for slot in 1..=4u8 {
                    let Some(session) = upgrade(&weak) else {
                        return;
                    };
                    let answered = session
                        .latest_status(slot)
                        .is_some_and(|(_, at)| at >= round_start);
                    if !answered {
                        let _ = session.request_status(slot).await;
                    }
                }
                {
                    let Some(session) = upgrade(&weak) else {
                        return;
                    };
                    if session.since_last_rx() > session.stall_timeout() {
                        session.mark_lost("no reply from the charger while polling");
                        return;
                    }
                }
                tokio::time::sleep(interval.saturating_sub(round_start.elapsed())).await;
            }
        });
        if let Some(previous) = lock(&self.inner.poller).replace(handle) {
            previous.abort();
        }
    }

    /// Stops background polling.
    pub fn stop_polling(&self) {
        abort(&self.inner.poller);
    }

    /// Configures and starts slots.
    ///
    /// For each config: `FE <slot>` (stops whatever runs there), then the 0x94 config, which
    /// starts the program when the charger acknowledges it. Then one 0x93 frame with every
    /// configured slot plus `keep_running`; the official app uses 0x93 only to read a slot's
    /// program and the charger ignores it, so other slots keep running either way.
    ///
    /// Nothing is sent when a config cannot be encoded (a mode its chemistry does not have).
    pub async fn start_slots(
        &self,
        configs: &[ChargeConfig],
        keep_running: u8,
    ) -> Result<(), BluetoothError> {
        let packets = Self::start_packets(configs, keep_running)?;
        self.send_sequence(&packets, COMMAND_GAP).await
    }

    /// Stops the slots in `stop_mask`, one slot at a time: `FE <slot>`, then `93 00`.
    ///
    /// `FE <slot>` is the stop (official app; captures). A mask with several bits stops
    /// only the lowest slot (live), hence one FE per slot. The `93 00` that follows is
    /// harmless. Other slots keep running.
    pub async fn stop_slots(&self, stop_mask: u8) -> Result<(), BluetoothError> {
        self.send_sequence(&Self::stop_packets(stop_mask), COMMAND_GAP)
            .await
    }

    fn stop_packets(stop_mask: u8) -> Vec<Vec<u8>> {
        (0..4)
            .map(|i| 1u8 << i)
            .filter(|bit| stop_mask & bit != 0)
            .flat_map(|bit| {
                [
                    MC5000Protocol::build_fe_query_command_for(bit),
                    MC5000Protocol::build_start_stop_command(StartStopAction::StopAll),
                ]
            })
            .collect()
    }

    fn start_packets(
        configs: &[ChargeConfig],
        keep_running: u8,
    ) -> Result<Vec<Vec<u8>>, BluetoothError> {
        if configs.is_empty() {
            return Ok(Vec::new());
        }
        let mut packets = Vec::with_capacity(configs.len() * 2 + 1);
        let mut start_mask = keep_running;
        for config in configs {
            packets.push(MC5000Protocol::build_fe_query_command_for(
                config.channel_bitmask,
            ));
            packets.push(MC5000Protocol::build_charge_config_command(config)?);
            start_mask |= config.channel_bitmask;
        }
        packets.push(MC5000Protocol::build_start_stop_command(
            StartStopAction::ChannelMask(start_mask),
        ));
        Ok(packets)
    }

    /// Whether the link is gone. A lost session stays lost: connect a new one.
    pub fn is_lost(&self) -> bool {
        self.inner.lost.load(Ordering::SeqCst)
    }

    /// Changes how long a polled link may stay silent before it is declared lost.
    pub fn set_stall_timeout(&self, timeout: Duration) {
        self.inner
            .stall_timeout_ms
            .store(timeout.as_millis() as u64, Ordering::SeqCst);
    }

    fn stall_timeout(&self) -> Duration {
        Duration::from_millis(self.inner.stall_timeout_ms.load(Ordering::SeqCst))
    }

    fn since_last_rx(&self) -> Duration {
        lock(&self.inner.last_rx).elapsed()
    }

    /// Declares the link lost: reports [`SessionEvent::Disconnected`] once, stops the
    /// background tasks and closes the link, so the owner can connect a fresh session.
    /// A frozen link (writes hang, nothing arrives) never ends on its own; this is how a
    /// session gets out of it.
    fn mark_lost(&self, reason: &str) {
        if self.inner.lost.swap(true, Ordering::SeqCst) {
            return;
        }
        log::warn!("Link to {} lost: {reason}", self.inner.name);
        let _ = self.inner.events.send(SessionEvent::Disconnected);
        abort(&self.inner.reader);
        let transport = self.inner.transport.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(WRITE_TIMEOUT, transport.disconnect()).await;
            });
        }
        // Last: this may be the poller itself, which must not abort itself before the above.
        abort(&self.inner.poller);
    }

    /// Stops polling and closes the BLE link.
    pub async fn disconnect(&self) -> Result<(), BluetoothError> {
        if self.inner.disconnected.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.inner.lost.store(true, Ordering::SeqCst);
        self.stop_polling();
        abort(&self.inner.reader);
        self.inner.transport.disconnect().await?;
        Ok(())
    }
}

fn upgrade(weak: &Weak<Inner>) -> Option<Session> {
    weak.upgrade().map(|inner| Session { inner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bluetooth::{BatteryChemistry, OperationMode};

    /// Status frame for slot 4 at 4.128 V, as received from a real charger.
    const SLOT4_STATUS: [u8; 23] = [
        0x0F, 0x15, 0x91, 0x08, 0x00, 0x00, 0x10, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC9,
    ];
    /// Reply to the 0x74 version query, as received from a real charger.
    const VERSION_REPLY: [u8; 6] = [0x0F, 0x04, 0x74, 0x01, 0x01, 0x76];

    #[test]
    fn shared_types_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Session>();
        assert_send_sync::<SessionEvent>();
        assert_send_sync::<Frame>();
        assert_send_sync::<FrameDecoder>();
    }

    #[test]
    fn decodes_a_whole_status_frame() {
        let frames = FrameDecoder::default().push(&SLOT4_STATUS);
        assert_eq!(frames.len(), 1);
        let status = frames[0].status().expect("status frame");
        assert_eq!(status.slot_id, 4);
        assert_eq!(status.voltage_mv, 4128);
    }

    #[test]
    fn reassembles_a_frame_split_at_the_20_byte_ble_limit() {
        let mut decoder = FrameDecoder::default();
        assert!(decoder.push(&SLOT4_STATUS[..20]).is_empty());
        let frames = decoder.push(&SLOT4_STATUS[20..]);
        assert_eq!(frames, vec![Frame(SLOT4_STATUS.to_vec())]);
    }

    #[test]
    fn splits_several_frames_in_one_notification() {
        let mut chunk = VERSION_REPLY.to_vec();
        chunk.extend_from_slice(&SLOT4_STATUS);
        let frames = FrameDecoder::default().push(&chunk);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].command(), 0x74);
        assert!(frames[1].status().is_some());
    }

    #[test]
    fn accepts_status_frames_of_other_lengths() {
        // A firmware that sends a longer 0x91 frame must still decode.
        let mut longer = SLOT4_STATUS[..22].to_vec();
        longer.extend_from_slice(&[0x00, 0x00]);
        longer[1] = 0x17;
        let sum = longer[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        longer.push(sum);
        let frames = FrameDecoder::default().push(&longer);
        assert_eq!(frames[0].status().map(|s| s.voltage_mv), Some(4128));
    }

    #[test]
    fn drops_frames_with_a_bad_checksum_and_resyncs() {
        let mut bad = SLOT4_STATUS;
        bad[22] ^= 0xFF;
        let mut chunk = vec![0xAA, 0x0F]; // garbage, including a false start marker
        chunk.extend_from_slice(&bad);
        chunk.extend_from_slice(&VERSION_REPLY);
        let frames = FrameDecoder::default().push(&chunk);
        assert_eq!(frames, vec![Frame(VERSION_REPLY.to_vec())]);
    }

    #[test]
    fn recovers_when_a_frame_loses_its_tail() {
        let mut decoder = FrameDecoder::default();
        assert!(decoder.push(&SLOT4_STATUS[..20]).is_empty());
        // The tail never comes; the next notification is a new frame.
        let frames = decoder.push(&SLOT4_STATUS);
        assert_eq!(frames, vec![Frame(SLOT4_STATUS.to_vec())]);
    }

    #[test]
    fn non_status_frames_have_no_status() {
        let frames = FrameDecoder::default().push(&VERSION_REPLY);
        assert!(frames[0].status().is_none());
    }

    fn config(slot: u8) -> ChargeConfig {
        ChargeConfig::new(
            slot,
            BatteryChemistry::LiIon,
            OperationMode::Charge,
            3000,
            1000,
        )
    }

    #[test]
    fn start_sequence_configures_each_slot_then_starts_the_combined_mask() {
        let packets = Session::start_packets(&[config(1), config(3)], 0b1000).unwrap();
        let commands: Vec<u8> = packets.iter().map(|p| p[2]).collect();
        assert_eq!(commands, vec![0xFE, 0x94, 0xFE, 0x94, 0x93]);
        assert_eq!(packets[0][3], 0x01);
        assert_eq!(packets[2][3], 0x04);
        assert_eq!(packets[4][3], 0x0D, "slots 1, 3 and the kept slot 4");
    }

    #[test]
    fn stop_sequence_selects_the_slot_then_stops() {
        let packets = Session::stop_packets(0x02);
        assert_eq!(
            packets,
            vec![
                vec![0x0F, 0x03, 0xFE, 0x02, 0x00],
                vec![0x0F, 0x03, 0x93, 0x00, 0x93]
            ]
        );
    }

    #[test]
    fn stopping_several_slots_selects_and_stops_each_one() {
        let packets = Session::stop_packets(0x0A);
        let pairs: Vec<(u8, u8)> = packets.chunks(2).map(|p| (p[0][3], p[1][3])).collect();
        assert_eq!(pairs, vec![(0x02, 0x00), (0x08, 0x00)]);
        assert_eq!(Session::stop_packets(0x0F).len(), 8);
    }

    #[test]
    fn start_sequence_is_empty_without_configs() {
        assert!(Session::start_packets(&[], 0x0F).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_mode_the_chemistry_lacks_sends_nothing() {
        let (session, writes) = fake_session(0);
        let bad = ChargeConfig::new(2, BatteryChemistry::NiMH, OperationMode::Storage, 400, 2000);
        assert!(session.start_slots(&[config(1), bad], 0).await.is_err());
        assert!(commands(&writes).is_empty(), "no frame may reach the charger");
    }

    /// In-memory charger. Records every write; answers each 0x91 request with a status
    /// frame (split at the 20-byte BLE limit), except the first `drop_requests` requests.
    struct FakeCharger {
        writes: Writes,
        replies: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
        drop_requests: std::sync::atomic::AtomicUsize,
        /// This many writes, from the first, never complete.
        hang_writes: std::sync::atomic::AtomicUsize,
    }

    fn status_frame(mask: u8) -> Vec<u8> {
        let mut frame = SLOT4_STATUS.to_vec();
        frame[3] = mask;
        let sum = frame[2..22].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        frame[22] = sum;
        frame
    }

    impl Transport for FakeCharger {
        fn write<'a>(
            &'a self,
            packet: &'a [u8],
        ) -> futures::future::BoxFuture<'a, Result<(), BluetoothError>> {
            Box::pin(async move {
                let hang = self
                    .hang_writes
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                if hang {
                    futures::future::pending::<()>().await;
                }
                lock(&self.writes).push((Instant::now(), packet.to_vec()));
                if let (0x57 | 0x96, Some(replies)) = (packet[2], &self.replies) {
                    let reply = if packet[2] == 0x57 { &INFO_REPLY[..] } else { &SETTINGS_REPLY[..] };
                    let _ = replies.send(reply.to_vec());
                }
                if packet[2] == 0x98 && packet[3] == 0x08 {
                    let dropped = self
                        .drop_requests
                        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                        .is_ok();
                    if let (false, Some(replies)) = (dropped, &self.replies) {
                        // Real pages, split at the 20-byte BLE limit, a status frame between.
                        for page in crate::bluetooth::test_frames::history_slot4() {
                            for chunk in page.chunks(20) {
                                let _ = replies.send(chunk.to_vec());
                            }
                            let _ = replies.send(status_frame(0x01));
                        }
                    }
                }
                if packet[2] == 0x91 {
                    let dropped = self
                        .drop_requests
                        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                        .is_ok();
                    if let (false, Some(replies)) = (dropped, &self.replies) {
                        let frame = status_frame(packet[3]);
                        let _ = replies.send(frame[..20].to_vec());
                        let _ = replies.send(frame[20..].to_vec());
                    }
                }
                Ok(())
            })
        }

        fn is_connected(&self) -> futures::future::BoxFuture<'_, bool> {
            Box::pin(async { true })
        }

        fn disconnect(&self) -> futures::future::BoxFuture<'_, Result<(), BluetoothError>> {
            Box::pin(async { Ok(()) })
        }
    }

    type Writes = Arc<Mutex<Vec<(Instant, Vec<u8>)>>>;

    fn fake_session(drop_requests: usize) -> (Session, Writes) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let incoming = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed();
        let writes = Writes::default();
        let charger = FakeCharger {
            writes: writes.clone(),
            replies: Some(tx),
            drop_requests: drop_requests.into(),
            hang_writes: 0.into(),
        };
        (
            Session::open(charger, incoming, "#Charger-TEST", "test"),
            writes,
        )
    }

    fn commands(writes: &Writes) -> Vec<u8> {
        lock(writes).iter().map(|(_, packet)| packet[2]).collect()
    }

    /// Replies read from an MC5000 with firmware 1.83.
    const INFO_REPLY: [u8; 21] = [
        0x0F, 0x13, 0x57, 0x00, 0x31, 0x30, 0x30, 0x32, 0x31, 0x34, 0x04, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x53, 0x01, 0x00, 0x7F, 0x57,
    ];
    const SETTINGS_REPLY: [u8; 11] = [0x0F, 0x09, 0x96, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x98];

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn device_info_and_settings_are_read_from_their_replies() {
        let (session, writes) = fake_session(0);
        session.start_polling(Duration::from_millis(10)); // status frames arrive in between
        let info = session.device_info(Duration::from_secs(2)).await.unwrap();
        assert_eq!((info.serial_number.as_str(), info.firmware_version.as_str()), ("100214", "1.83"));
        let settings = session.system_settings(Duration::from_secs(2)).await.unwrap();
        assert_eq!(settings.lcd, 1);
        session.stop_polling();
        let sent = commands(&writes);
        assert!(sent.contains(&0x57) && sent.contains(&0x96));
        assert!(!sent.iter().any(|c| matches!(c, 0x94 | 0xFE | 0x93 | 0x74 | 0x65)), "read only: {sent:02X?}");
    }

    #[test]
    fn decodes_history_pages_longer_than_other_frames() {
        let pages = crate::bluetooth::test_frames::history_slot4();
        let mut decoder = FrameDecoder::default();
        let mut frames = Vec::new();
        for chunk in pages[0].chunks(20) {
            frames.extend(decoder.push(chunk)); // 208 bytes in 20-byte notifications
        }
        frames.extend(decoder.push(&pages[2])); // whole, in one notification
        assert_eq!(frames, vec![Frame(pages[0].clone()), Frame(pages[2].clone())]);
    }

    #[test]
    fn a_long_length_byte_is_noise_for_other_commands() {
        // `0F CE 91`: a length only a history page may have, on a status command.
        let mut chunk = vec![0x0F, 0xCE, 0x91, 0x00];
        chunk.extend_from_slice(&VERSION_REPLY);
        assert_eq!(FrameDecoder::default().push(&chunk), vec![Frame(VERSION_REPLY.to_vec())]);
        // The command byte may arrive in the next notification.
        let mut decoder = FrameDecoder::default();
        assert!(decoder.push(&[0x0F, 0xCE]).is_empty());
        assert_eq!(decoder.push(&chunk[2..]), vec![Frame(VERSION_REPLY.to_vec())]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn history_is_read_across_pages_while_polling() {
        let (session, writes) = fake_session(0);
        session.start_polling(Duration::from_millis(10));
        let history = session.read_history(4, Duration::from_secs(2)).await.unwrap();
        session.stop_polling();
        assert_eq!((history.slot_id, history.interval_s, history.samples_mv.len()), (4, 4, 223));
        assert_eq!(history.samples_mv[0], 4145);
        let sent = commands(&writes);
        assert!(!sent.iter().any(|c| matches!(c, 0x94 | 0xFE | 0x93)), "read only: {sent:02X?}");
    }

    #[tokio::test]
    async fn history_is_asked_again_when_the_first_request_is_lost() {
        let (session, writes) = fake_session(1);
        let history = session.read_history(4, Duration::from_secs(3)).await.unwrap();
        assert_eq!(history.samples_mv.len(), 223);
        assert_eq!(commands(&writes), vec![0x98, 0x98]);
    }

    #[tokio::test]
    async fn a_slot_without_history_times_out() {
        let (session, _) = fake_session(0);
        assert!(session.read_history(1, Duration::from_millis(500)).await.is_err());
        assert!(session.read_history(5, Duration::from_millis(500)).await.is_err());
    }

    #[tokio::test]
    async fn a_request_without_reply_times_out() {
        let (session, _) = fake_session(0);
        let err = session.request(&[0x0F, 0x03, 0x99, 0x00, 0x99], 0x99, Duration::from_millis(600)).await;
        assert!(err.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn read_status_decodes_an_answer_split_across_chunks() {
        let (session, _) = fake_session(0);
        let status = session
            .read_status(4, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!((status.slot_id, status.voltage_mv), (4, 4128));
        assert_eq!(
            session.latest_status(4).map(|(s, _)| s.voltage_mv),
            Some(4128)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn read_status_re_requests_when_the_charger_drops_a_request() {
        let (session, writes) = fake_session(1);
        let status = session
            .read_status(2, Duration::from_secs(3))
            .await
            .unwrap();
        assert_eq!(status.slot_id, 2);
        assert_eq!(
            commands(&writes),
            vec![0x91, 0x91],
            "one dropped request, one retry"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_requests_are_paced() {
        let (session, writes) = fake_session(0);
        let tasks: Vec<_> = (1..=4u8)
            .map(|slot| {
                let session = session.clone();
                tokio::spawn(async move { session.request_status(slot).await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let times: Vec<Instant> = lock(&writes).iter().map(|(at, _)| *at).collect();
        assert_eq!(times.len(), 4);
        for pair in times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap >= REQUEST_GAP - Duration::from_millis(2), "gap {gap:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn polling_never_interleaves_with_a_command_sequence() {
        let (session, writes) = fake_session(0);
        session.start_polling(Duration::from_millis(10)); // as aggressive as possible
        tokio::time::sleep(Duration::from_millis(300)).await;
        session
            .start_slots(&[config(1), config(3)], 0)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        session.stop_polling();

        let sent = commands(&writes);
        let sequence: Vec<usize> = (0..sent.len()).filter(|&i| sent[i] != 0x91).collect();
        assert_eq!(
            sequence.iter().map(|&i| sent[i]).collect::<Vec<_>>(),
            vec![0xFE, 0x94, 0xFE, 0x94, 0x93]
        );
        assert!(
            sequence.windows(2).all(|w| w[1] == w[0] + 1),
            "interleaved: {sent:02X?}"
        );
        assert!(
            sent[..sequence[0]].contains(&0x91),
            "polling ran before the sequence"
        );
        assert!(
            sent[sequence[4] + 1..].contains(&0x91),
            "polling resumed after it"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn poller_re_requests_slots_that_did_not_answer() {
        let (session, writes) = fake_session(1); // the very first request goes unanswered
        session.start_polling(Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        session.stop_polling();
        let sent = lock(&writes).iter().map(|(_, p)| p[3]).collect::<Vec<_>>();
        assert_eq!(
            sent,
            vec![0x01, 0x02, 0x04, 0x08, 0x01],
            "slot 1 is asked again"
        );
        assert!((1..=4).all(|slot| session.latest_status(slot).is_some()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_hung_write_times_out_and_does_not_block_later_writes() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let incoming = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed();
        let writes = Writes::default();
        let charger = FakeCharger {
            writes: writes.clone(),
            replies: Some(tx),
            drop_requests: 0.into(),
            hang_writes: 1.into(),
        };
        let session = Session::open(charger, incoming, "#Charger-TEST", "test");

        let started = Instant::now();
        assert!(
            session.request_status(1).await.is_err(),
            "hung write must fail"
        );
        assert!(started.elapsed() < WRITE_TIMEOUT + Duration::from_millis(500));
        // The lock was released: stopping still works.
        session.stop_slots(0x0F).await.unwrap();
        assert_eq!(lock(&writes).len(), 8);
    }

    /// A session on a fake charger that answers nothing, plus its writes.
    fn silent_session(hang_writes: usize) -> (Session, Writes) {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        // Keep the sender alive inside the stream so the link does not end by itself.
        let incoming = futures::stream::unfold((rx, _tx), |(mut rx, tx)| async move {
            rx.recv().await.map(|chunk| (chunk, (rx, tx)))
        })
        .boxed();
        let writes = Writes::default();
        let charger = FakeCharger {
            writes: writes.clone(),
            replies: None,
            drop_requests: 0.into(),
            hang_writes: hang_writes.into(),
        };
        (Session::open(charger, incoming, "#Charger-TEST", "test"), writes)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn repeated_write_timeouts_declare_the_link_lost() {
        let (session, _) = silent_session(usize::MAX);
        let mut events = session.subscribe();
        assert!(session.request_status(1).await.is_err());
        assert!(!session.is_lost(), "one timeout is not enough");
        assert!(session.request_status(2).await.is_err());
        assert!(session.is_lost());
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv()).await;
        assert!(matches!(event, Ok(Ok(SessionEvent::Disconnected))));

        // Later writes fail at once instead of waiting for another timeout.
        let started = Instant::now();
        assert!(session.stop_slots(0x0F).await.is_err());
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_silent_link_is_declared_lost_while_polling() {
        let (session, writes) = silent_session(0);
        session.set_stall_timeout(Duration::from_millis(400));
        let mut events = session.subscribe();
        session.start_polling(Duration::from_millis(50));
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv()).await;
        assert!(matches!(event, Ok(Ok(SessionEvent::Disconnected))));
        assert!(session.is_lost());
        // Polling stopped with it.
        let sent = lock(&writes).len();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(lock(&writes).len(), sent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_lost_link_is_reported_once() {
        let (session, _) = silent_session(usize::MAX);
        let mut events = session.subscribe();
        for slot in 1..=4 {
            let _ = session.request_status(slot).await;
        }
        assert!(matches!(events.recv().await, Ok(SessionEvent::Disconnected)));
        let more = tokio::time::timeout(Duration::from_millis(300), events.recv()).await;
        assert!(more.is_err(), "no second Disconnected event");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn session_reports_the_end_of_the_link() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let incoming = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed();
        let charger = FakeCharger {
            writes: Writes::default(),
            replies: None,
            drop_requests: 0.into(),
            hang_writes: 0.into(),
        };
        let session = Session::open(charger, incoming, "#Charger-TEST", "test");
        let mut events = session.subscribe();
        drop(tx);
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv()).await;
        assert!(matches!(event, Ok(Ok(SessionEvent::Disconnected))));
    }

    #[test]
    fn slot_mask_rejects_out_of_range_slots() {
        assert_eq!(slot_mask(1).ok(), Some(0x01));
        assert_eq!(slot_mask(4).ok(), Some(0x08));
        assert!(slot_mask(0).is_err());
        assert!(slot_mask(5).is_err());
    }
}
