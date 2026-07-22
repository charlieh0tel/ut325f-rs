use anyhow::{Context, Result, anyhow};
use btleplug::api::{BDAddr, Central, Manager as _, Peripheral as _, ValueNotification};
use btleplug::platform::{Manager, Peripheral};
use futures::{Stream, StreamExt};
use std::pin::Pin;
use uuid::Uuid;

use super::{DATA_OUT_UUID, Transport};

/// Transport over Bluetooth LE using the `btleplug` crate.
///
/// The device must already be known to the platform's Bluetooth stack
/// (i.e. paired or previously discovered). While the transport is alive
/// it holds the GATT notification session; the meter drops the BLE
/// connection when no client holds it.
pub struct BtleplugTransport {
    // Held to keep the connection and subscription alive.
    _peripheral: Peripheral,
    notifications: Pin<Box<dyn Stream<Item = ValueNotification> + Send>>,
    data_out_uuid: Uuid,
}

impl BtleplugTransport {
    /// Connects to the meter with the given Bluetooth address
    /// (e.g. "E8:26:CF:F1:23:61") and starts notifications.
    pub async fn open(address: &str) -> Result<Self> {
        let target: BDAddr = address
            .parse()
            .map_err(|e| anyhow!("Invalid Bluetooth address '{address}': {e}"))?;

        let manager = Manager::new()
            .await
            .context("Failed to initialize btleplug")?;
        let adapter = manager
            .adapters()
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No Bluetooth adapter found"))?;

        let mut peripheral = None;
        for candidate in adapter.peripherals().await? {
            if candidate.address() == target {
                peripheral = Some(candidate);
                break;
            }
        }
        let peripheral = peripheral
            .ok_or_else(|| anyhow!("Bluetooth device {address} is not known; pair it first"))?;

        if !peripheral.is_connected().await? {
            peripheral
                .connect()
                .await
                .with_context(|| format!("Failed to connect to {address}"))?;
        }
        peripheral.discover_services().await?;

        let data_out_uuid = Uuid::parse_str(DATA_OUT_UUID).expect("valid UUID");
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == data_out_uuid)
            .ok_or_else(|| {
                anyhow!("Characteristic {DATA_OUT_UUID} not found on {address}; is this a UT325F?")
            })?;

        // Take the notification stream before subscribing so no frames
        // are missed.
        let notifications = peripheral.notifications().await?;
        peripheral
            .subscribe(&characteristic)
            .await
            .context("Failed to enable notifications")?;

        Ok(Self {
            _peripheral: peripheral,
            notifications,
            data_out_uuid,
        })
    }
}

impl Transport for BtleplugTransport {
    async fn recv(&mut self) -> Result<Vec<u8>> {
        loop {
            let notification = self
                .notifications
                .next()
                .await
                .ok_or_else(|| anyhow!("BLE notification stream ended"))?;
            if notification.uuid == self.data_out_uuid {
                return Ok(notification.value);
            }
        }
    }
}
