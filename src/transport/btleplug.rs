use anyhow::{Context, Result, anyhow};
use btleplug::api::{
    BDAddr, Central, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::time::Duration;
use uuid::Uuid;

use super::{DATA_OUT_UUID, DiscoveredMeter, METER_NAME_PREFIX, Transport, finalize_discovered};

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

        let mut peripheral = None;
        'adapters: for adapter in &all_adapters().await? {
            for candidate in adapter.peripherals().await? {
                if candidate.address() == target {
                    peripheral = Some(candidate);
                    break 'adapters;
                }
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

    /// Discovers meters for `timeout` and connects to the only meter
    /// found; errors if there are none or more than one (opening an
    /// arbitrary meter could pick the wrong one).
    pub async fn open_only(timeout: Duration) -> Result<Self> {
        let meter = super::exactly_one(Self::discover(timeout).await?)?;
        Self::open(&meter.address).await
    }

    /// Scans for `timeout` and returns the UT325F meters known to the
    /// Bluetooth stack, strongest signal first. Devices with
    /// `rssi: None` come from the stack's cache (e.g. paired meters
    /// currently out of range).
    ///
    /// Scans on every adapter; adapters that cannot scan (e.g. powered
    /// off) are skipped as long as at least one can.
    pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredMeter>> {
        let adapters = all_adapters().await?;

        let mut scanning = Vec::new();
        let mut last_error = None;
        for adapter in &adapters {
            match adapter.start_scan(ScanFilter::default()).await {
                Ok(()) => scanning.push(adapter),
                Err(e) => last_error = Some(e),
            }
        }
        if scanning.is_empty() {
            return Err(last_error.expect("all_adapters returns at least one adapter"))
                .context("Failed to start discovery on any adapter");
        }

        tokio::time::sleep(timeout).await;

        let mut meters = Vec::new();
        let mut result = Ok(());
        'adapters: for adapter in &adapters {
            for peripheral in match adapter.peripherals().await {
                Ok(peripherals) => peripherals,
                Err(e) => {
                    result = Err(e);
                    break 'adapters;
                }
            } {
                let Ok(Some(properties)) = peripheral.properties().await else {
                    continue;
                };
                let Some(name) = properties.local_name else {
                    continue;
                };
                if !name.starts_with(METER_NAME_PREFIX) {
                    continue;
                }
                meters.push(DiscoveredMeter {
                    address: properties.address.to_string(),
                    name,
                    rssi: properties.rssi,
                });
            }
        }
        for adapter in &scanning {
            let _ = adapter.stop_scan().await;
        }
        result?;
        Ok(finalize_discovered(meters))
    }
}

async fn all_adapters() -> Result<Vec<Adapter>> {
    let manager = Manager::new()
        .await
        .context("Failed to initialize btleplug")?;
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        return Err(anyhow!("No Bluetooth adapter found"));
    }
    Ok(adapters)
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
