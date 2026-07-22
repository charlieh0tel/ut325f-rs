use anyhow::{Result, anyhow};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use super::Transport;

/// Transport over the meter's USB serial interface.
pub struct SerialTransport {
    serial: SerialStream,
}

impl SerialTransport {
    pub async fn open(port: &str) -> Result<Self> {
        let builder = tokio_serial::new(port, 115200)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .timeout(Duration::from_secs(1));

        let serial = builder
            .open_native_async()
            .map_err(|e| anyhow!("Failed to open serial port '{}': {}", port, e))?;
        Ok(Self { serial })
    }
}

impl Transport for SerialTransport {
    async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 256];
        let n = self.serial.read(&mut buf).await?;
        if n == 0 {
            return Err(anyhow!("Serial port closed"));
        }
        buf.truncate(n);
        Ok(buf)
    }
}
