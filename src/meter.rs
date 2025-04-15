use anyhow::{anyhow, Result};
use serialport::SerialPort;
use std::time::{Duration, Instant};

use crate::reading::Reading;

/// Communication with a Uni-T UT325F over a serial port.
pub struct Meter {
    _sync_timeout: Duration,
    port: String,
    serial: Option<Box<dyn SerialPort>>,
}

impl Meter {
    pub fn new(port: String) -> Self {
        Meter {
            _sync_timeout: Duration::from_secs(5),
            port,
            serial: None,
        }
    }

    pub fn open(&mut self) -> Result<()> {
        let builder = serialport::new(&self.port, 115200)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(Duration::from_secs(1));

        match builder.open() {
            Ok(port) => {
                self.serial = Some(port);
                // We get garbage sometimes before we're fully synchronized.
                for _ in 0..5 {
                    match self.read() {
                        Ok(_) => (),  // Discard
                        Err(_) => (), // Never mind, it will get sorted.
                    }
                }
                Ok(())
            }
            Err(e) => Err(anyhow!("Failed to open serial port '{}': {}", self.port, e)),
        }
    }

    pub fn read(&mut self) -> Result<Reading> {
        let serial = self
            .serial
            .as_mut()
            .ok_or_else(|| anyhow!("Serial port is not open"))?;
        let start = Instant::now();

        while start.elapsed() < self._sync_timeout {
            let mut sync_buf = vec![0u8; Reading::N_SYNC_BYTES];
            match serial.read_exact(&mut sync_buf) {
                Ok(_) => {
                    if sync_buf == Reading::SYNC {
                        let mut rest_buf = vec![0u8; Reading::N_BYTES - Reading::N_SYNC_BYTES];
                        serial.read_exact(&mut rest_buf)?;
                        let mut combined = sync_buf;
                        combined.extend_from_slice(&rest_buf);
                        let reading_array: [u8; Reading::N_BYTES] =
                            combined.try_into().map_err(|v: Vec<u8>| {
                                anyhow!(
                                    "Error converting Vec<u8> to [u8; {}]: {:?}",
                                    Reading::N_BYTES,
                                    v
                                )
                            })?;
                        return Reading::parse(&reading_array);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // Continue reading if it's just a timeout
                }
                Err(e) => return Err(anyhow!("Error reading from serial port: {}", e)),
            }
        }
        Err(anyhow!("Failed to sync within timeout."))
    }

    pub fn close(&mut self) {
        if let Some(serial) = self.serial.take() {
            // The serial port will be closed when the Box is dropped
            drop(serial);
        }
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.close();
    }
}
