use anyhow::{Result, anyhow};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tokio::time;

use crate::reading::Reading;

pub struct Meter {
    _sync_timeout: Duration,
    port: String,
    serial: Option<SerialStream>,
}

impl Meter {
    pub fn new(port: String) -> Self {
        Meter {
            _sync_timeout: Duration::from_secs(5),
            port,
            serial: None,
        }
    }

    pub async fn open(&mut self) -> Result<()> {
        let builder = tokio_serial::new(&self.port, 115200)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .timeout(Duration::from_secs(1));

        match builder.open_native_async() {
            Ok(port) => {
                self.serial = Some(port);
                self.clear_buffer().await?;
                Ok(())
            }
            Err(e) => Err(anyhow!("Failed to open serial port '{}': {}", self.port, e)),
        }
    }

    async fn clear_buffer(&mut self) -> Result<()> {
        for _ in 0..3 {
            match self.read().await {
                Ok(_) => (),
                Err(ref e) if e.to_string().contains("TimedOut") => {
                    // Ignore timeouts during clearing
                }
                Err(e) => eprintln!("Warning: Initial read error: {}", e),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    pub async fn read(&mut self) -> Result<Reading> {
        let mut serial = self
            .serial
            .as_mut()
            .ok_or_else(|| anyhow!("Serial port is not open"))?;
        let mut sync_buf = vec![0u8; Reading::N_SYNC_BYTES];
        let mut rest_buf = vec![0u8; Reading::N_BYTES - Reading::N_SYNC_BYTES];

        loop {
            read_with_timeout(&mut serial, &mut sync_buf,
                              self._sync_timeout).await?;
            if sync_buf == Reading::SYNC {
                break;
            }
        }
        read_with_timeout(&mut serial, &mut rest_buf,
                          self._sync_timeout).await?;

        let mut combined = sync_buf;
        combined.extend_from_slice(&rest_buf);
        let reading_array: [u8; Reading::N_BYTES] = combined.try_into().map_err(|v: Vec<u8>| {
            anyhow!(
                "Error converting Vec<u8> to [u8; {}]: {:?}",
                Reading::N_BYTES,
                v
            )
        })?;

        Reading::parse(&reading_array)
    }

    pub async fn close(&mut self) -> Result<()> {
        self.serial.take();
        Ok(())
    }
}


async fn read_with_timeout<R>(
    mut reader: R,
    buf: &mut [u8],
    timeout: Duration,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    match time::timeout(timeout, reader.read_exact(buf)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(anyhow!("Error reading data: {}", e)),
        Err(_) => Err(anyhow!("Timeout reading data")),
    }
}
