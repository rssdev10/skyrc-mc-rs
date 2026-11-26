//! How bytes reach the charger. [`Session`](crate::Session) runs on any [`Transport`].
//!
//! Everything above the transport (framing, pacing, polling, start/stop sequences, the GUI
//! and the CLI) is shared. A new link type only has to implement [`Transport`] and hand
//! its incoming bytes to [`Session::open`](crate::Session::open).
//!
//! - [`BleTransport`]: Bluetooth LE, the only link the MC5000 protocol is known for.
//! - [`SerialTransport`]: placeholder for a wired (USB serial) link, to be implemented once
//!   that protocol is known. Opening one fails; it never fakes a connection.

use crate::bluetooth::{Error, DiscoveredCharger, Protocol};
use crate::session::ConnectMode;
use btleplug::api::Peripheral as _;
use btleplug::platform::Peripheral;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::StreamExt;

/// Raw bytes as they arrive from the link. Chunks need not align with frames.
pub type IncomingBytes = BoxStream<'static, Vec<u8>>;

/// A connected link to a charger.
pub trait Transport: Send + Sync + 'static {
    /// Writes one packet. Pacing is the session's job, not the transport's.
    fn write<'a>(&'a self, packet: &'a [u8]) -> BoxFuture<'a, Result<(), Error>>;

    /// Whether the link is still up.
    fn is_connected(&self) -> BoxFuture<'_, bool>;

    /// Closes the link.
    fn disconnect(&self) -> BoxFuture<'_, Result<(), Error>>;
}

/// Bluetooth LE link, through btleplug.
pub struct BleTransport {
    protocol: Protocol,
    peripheral: Peripheral,
}

impl BleTransport {
    /// Connects (BLE link, service discovery, handshake and, for [`ConnectMode::Control`],
    /// the init sequence) and subscribes to notifications. One attempt, no timeout:
    /// [`Session::connect`](crate::Session::connect) adds both.
    pub async fn connect(
        device: &DiscoveredCharger,
        mode: ConnectMode,
    ) -> Result<(Self, IncomingBytes), Error> {
        let mut protocol = Protocol::new();
        let peripheral = device.peripheral.clone();
        match mode {
            ConnectMode::Control => protocol.connect(&peripheral).await?,
            ConnectMode::Monitor => protocol.connect_without_init(&peripheral).await?,
        }
        let notifications = peripheral.notifications().await?;
        let incoming = notifications.map(|notification| notification.value).boxed();
        Ok((
            Self {
                protocol,
                peripheral,
            },
            incoming,
        ))
    }
}

impl Transport for BleTransport {
    fn write<'a>(&'a self, packet: &'a [u8]) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(self.protocol.send_command(packet))
    }

    fn is_connected(&self) -> BoxFuture<'_, bool> {
        Box::pin(self.protocol.is_connected())
    }

    fn disconnect(&self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move { Ok(self.peripheral.disconnect().await?) })
    }
}

/// Wired (USB serial) link. Placeholder: the MC5000's wired protocol is not known yet.
///
/// To implement: open the port in [`SerialTransport::open`], spawn a reader that sends
/// received chunks into the returned [`IncomingBytes`], write packets in
/// [`Transport::write`], and list ports next to the BLE scan. If the wired protocol uses
/// different packets, add builders and parsers in `bluetooth.rs`'s style; the session's
/// framing assumes the `0F <len> <cmd> <data> <sum>` layout.
pub struct SerialTransport {
    port: String,
}

impl SerialTransport {
    /// Always fails until the wired protocol is implemented.
    pub async fn open(port: &str) -> Result<(Self, IncomingBytes), Error> {
        Err(Self {
            port: port.to_string(),
        }
        .not_supported())
    }

    fn not_supported(&self) -> Error {
        Error::CommunicationError(format!(
            "Serial connection on {} is not supported yet: the MC5000 wired protocol is unknown",
            self.port
        ))
    }
}

impl Transport for SerialTransport {
    fn write<'a>(&'a self, _packet: &'a [u8]) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move { Err(self.not_supported()) })
    }

    fn is_connected(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn disconnect(&self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serial_placeholder_refuses_to_open() {
        let error = SerialTransport::open("/dev/ttyUSB0")
            .await
            .err()
            .expect("must fail");
        assert!(error.to_string().contains("not supported yet"), "{error}");
        assert!(error.to_string().contains("/dev/ttyUSB0"), "{error}");
    }
}
