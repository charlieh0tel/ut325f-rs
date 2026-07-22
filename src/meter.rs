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

    /// Returns the next reading, skipping corrupted frames. Errors only
    /// on transport failure or when no valid frame arrives within the
    /// read timeout.
    pub async fn read(&mut self) -> Result<Reading> {
        tokio::time::timeout(self.read_timeout, self.read_frame())
            .await
            .map_err(|_| anyhow!("Timeout reading data"))?
    }

    async fn read_frame(&mut self) -> Result<Reading> {
        loop {
            // The decoder yields only checksum-valid frames; parse can
            // still reject one (e.g. an unknown hold type) — skip it.
            if let Some(frame) = self.decoder.next_frame() {
                if let Ok(reading) = Reading::parse(&frame) {
                    return Ok(reading);
                }
                continue;
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

    /// Discovers meters over Bluetooth LE for `timeout` and opens the
    /// only one found; errors if there are none or more than one.
    pub async fn open_ble_only(timeout: std::time::Duration) -> Result<Self> {
        Ok(Self::new(
            crate::transport::BleTransport::open_only(timeout).await?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::tests::fix_checksum;
    use std::collections::VecDeque;

    struct ChunkTransport {
        chunks: VecDeque<Vec<u8>>,
    }

    impl Transport for ChunkTransport {
        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.chunks
                .pop_front()
                .ok_or_else(|| anyhow!("Transport closed"))
        }
    }

    fn meter_with(chunks: Vec<Vec<u8>>) -> Meter<ChunkTransport> {
        Meter::new(ChunkTransport {
            chunks: chunks.into(),
        })
    }

    fn valid_frame() -> [u8; Reading::N_BYTES] {
        let mut frame = [0u8; Reading::N_BYTES];
        frame[..Reading::N_SYNC_BYTES].copy_from_slice(&Reading::SYNC);
        fix_checksum(&mut frame);
        frame
    }

    #[tokio::test]
    async fn test_read_across_chunks() -> Result<()> {
        let frame = valid_frame();
        let mut meter = meter_with(vec![frame[..30].to_vec(), frame[30..].to_vec()]);
        let reading = meter.read().await?;
        assert_eq!(reading.hold_type, crate::reading::HoldType::Current);
        Ok(())
    }

    #[tokio::test]
    async fn test_read_skips_corrupt_frame() -> Result<()> {
        let mut corrupted = valid_frame();
        corrupted[10] ^= 0x01;
        let mut meter = meter_with(vec![corrupted.to_vec(), valid_frame().to_vec()]);
        assert!(meter.read().await.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_read_skips_unparseable_frame() -> Result<()> {
        // Checksum-valid but with an unknown hold type; read must skip
        // to the following good frame rather than fail.
        let mut bad_hold = valid_frame();
        bad_hold[Reading::N_BYTES - 3] = 0xff;
        fix_checksum(&mut bad_hold);
        let mut meter = meter_with(vec![bad_hold.to_vec(), valid_frame().to_vec()]);
        assert!(meter.read().await.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_read_transport_error() {
        let mut meter = meter_with(vec![]);
        assert!(meter.read().await.is_err());
    }
}
