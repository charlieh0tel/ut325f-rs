use anyhow::{Result, anyhow};
use std::time::Duration;

use crate::decoder::FrameDecoder;
use crate::reading::Reading;
use crate::transport::Transport;

const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A UT325F meter on some transport.
///
/// The meter streams readings unsolicited (roughly 3 per second); `read`
/// returns the next one.
pub struct Meter<T: Transport> {
    transport: T,
    decoder: FrameDecoder,
    read_timeout: Duration,
}

impl<T: Transport> Meter<T> {
    pub fn new(transport: T) -> Self {
        Meter {
            transport,
            decoder: FrameDecoder::new(),
            read_timeout: DEFAULT_READ_TIMEOUT,
        }
    }

    pub async fn read(&mut self) -> Result<Reading> {
        tokio::time::timeout(self.read_timeout, self.read_frame())
            .await
            .map_err(|_| anyhow!("Timeout reading data"))?
    }

    async fn read_frame(&mut self) -> Result<Reading> {
        loop {
            if let Some(frame) = self.decoder.next_frame() {
                return Reading::parse(&frame);
            }
            let chunk = self.transport.recv().await?;
            self.decoder.push(&chunk);
        }
    }
}

#[cfg(feature = "serial")]
impl Meter<crate::transport::SerialTransport> {
    /// Opens the meter on a USB serial port (e.g. "/dev/ttyUSB0").
    pub async fn open_serial(port: &str) -> Result<Self> {
        Ok(Self::new(
            crate::transport::SerialTransport::open(port).await?,
        ))
    }
}

#[cfg(any(feature = "bluebus", feature = "btleplug"))]
impl Meter<crate::transport::BleTransport> {
    /// Opens the meter over Bluetooth LE by its Bluetooth address
    /// (e.g. "E8:26:CF:F1:23:61"), using the enabled BLE backend. The
    /// device must already be known to the Bluetooth stack (paired or
    /// discovered).
    pub async fn open_ble(address: &str) -> Result<Self> {
        Ok(Self::new(
            crate::transport::BleTransport::open(address).await?,
        ))
    }
}
