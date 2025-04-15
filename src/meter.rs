use anyhow::{Result, anyhow};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

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
        for _ in 0..10 {
            // Increased dummy reads
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
        let serial = self
            .serial
            .as_mut()
            .ok_or_else(|| anyhow!("Serial port is not open"))?;
        let mut sync_buf = vec![0u8; Reading::N_SYNC_BYTES];
        let mut rest_buf = vec![0u8; Reading::N_BYTES - Reading::N_SYNC_BYTES];

        loop {
            tokio::select! {
                result = serial.read_exact(&mut sync_buf) => {
                    result.map_err(|e| anyhow!("Error reading sync header: {}", e))?;
                }
                _ = tokio::time::sleep(self._sync_timeout) => {
                    return Err(anyhow!("Timeout reading sync header"));
                }
            }

            if sync_buf == Reading::SYNC {
                break;
            }
        }

        tokio::select! {
            result = serial.read_exact(&mut rest_buf) => {
                result.map_err(|e| anyhow!("Error reading data: {}", e))?;
            }
            _ = tokio::time::sleep(self._sync_timeout) => {
                return Err(anyhow!("Timeout reading data"));
            }
        }

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
