use btleplug::api::{
    BDAddr, Central, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::time::Duration;
use uuid::Uuid;

use super::{DATA_OUT_UUID, DiscoveredMeter, METER_NAME_PREFIX, Transport, finalize_discovered};
use crate::error::{Error, Result};

const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport over Bluetooth LE using the `btleplug` crate.
///
/// The device must already be known to the platform's Bluetooth stack
/// (i.e. paired or previously discovered). If the transport initiated
/// the BLE connection, dropping it disconnects the device; a connected
/// meter stops advertising, so a leaked connection would hide it from
/// every later scan.
pub struct BtleplugTransport {
    // Held to keep the connection and subscription alive; the guard
    // disconnects on drop, and only if we initiated the connection.
    _peripheral: DisconnectGuard,
    notifications: Pin<Box<dyn Stream<Item = ValueNotification> + Send>>,
    data_out_uuid: Uuid,
}

impl BtleplugTransport {
    /// Connects to the meter with the given Bluetooth address
    /// (e.g. "E8:26:CF:F1:23:61") and starts notifications.
    pub async fn open(address: &str) -> Result<Self> {
        tokio::time::timeout(OPEN_TIMEOUT, Self::open_inner(address))
            .await
            .map_err(|_| Error::ConnectTimeout(address.to_owned()))?
    }

    async fn open_inner(address: &str) -> Result<Self> {
        let target: BDAddr = address
            .parse()
            .map_err(|_| Error::InvalidAddress(address.to_owned()))?;

        // Search every adapter, tolerating per-adapter enumeration
        // failures as long as the target is found somewhere.
        let mut peripheral = None;
        let mut enumeration_error = None;
        'adapters: for adapter in &all_adapters().await? {
            let candidates = match adapter.peripherals().await {
                Ok(candidates) => candidates,
                Err(e) => {
                    enumeration_error = Some(e);
                    continue;
                }
            };
            for candidate in candidates {
                if candidate.address() == target {
                    peripheral = Some(candidate);
                    break 'adapters;
                }
            }
        }
        let peripheral = peripheral.ok_or_else(|| match enumeration_error {
            Some(source) => Error::DeviceSearchIncomplete {
                address: address.to_owned(),
                source,
            },
            None => Error::DeviceNotKnown(address.to_owned()),
        })?;

        // The guard covers cancellation for the rest of open (e.g. the
        // open timeout firing) and then rides in the transport; it only
        // disconnects a connection this call established.
        let mut peripheral = DisconnectGuard {
            peripheral,
            initiated: false,
        };
        if !peripheral.is_connected().await? {
            peripheral
                .connect()
                .await
                .map_err(|e| Error::ConnectFailed {
                    address: address.to_owned(),
                    source: Box::new(e),
                })?;
            peripheral.initiated = true;
        }
        peripheral.discover_services().await?;

        let data_out_uuid = Uuid::parse_str(DATA_OUT_UUID).expect("valid UUID");
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == data_out_uuid)
            .ok_or_else(|| Error::CharacteristicNotFound {
                uuid: DATA_OUT_UUID.to_owned(),
                address: address.to_owned(),
            })?;

        // Take the notification stream before subscribing so no frames
        // are missed.
        let notifications = peripheral.notifications().await?;
        peripheral.subscribe(&characteristic).await?;

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

        // The guard stops the scans we started, including on error and
        // cancellation paths.
        let mut guard = ScanGuard {
            adapters: Vec::new(),
        };
        let mut last_error = None;
        for adapter in &adapters {
            match adapter.start_scan(ScanFilter::default()).await {
                Ok(()) => guard.adapters.push(adapter.clone()),
                Err(e) => last_error = Some(e),
            }
        }
        if guard.adapters.is_empty() {
            return Err(Error::Btleplug(
                last_error.expect("all_adapters returns at least one adapter"),
            ));
        }

        tokio::time::sleep(timeout).await;

        // Tolerate per-adapter enumeration failures; error out only if
        // they may have hidden every meter.
        let mut meters = Vec::new();
        let mut enumeration_error = None;
        for adapter in &adapters {
            let peripherals = match adapter.peripherals().await {
                Ok(peripherals) => peripherals,
                Err(e) => {
                    enumeration_error = Some(e);
                    continue;
                }
            };
            for peripheral in peripherals {
                // A device whose properties cannot be read the first
                // time gets one retry before being skipped, so a
                // transient fault does not silently hide a meter.
                let mut properties = peripheral.properties().await;
                if properties.is_err() {
                    properties = peripheral.properties().await;
                }
                let Ok(Some(properties)) = properties else {
                    continue;
                };
                let Some(name) = properties.local_name else {
                    continue;
                };
                if !name.starts_with(METER_NAME_PREFIX) {
                    continue;
                }
                let connected = peripheral.is_connected().await.unwrap_or(false);
                meters.push(DiscoveredMeter {
                    address: properties.address.to_string(),
                    name,
                    rssi: properties.rssi,
                    connected,
                });
            }
        }

        // Stop the scans we started; the guard covers earlier exits.
        for adapter in guard.adapters.drain(..) {
            let _ = adapter.stop_scan().await;
        }

        if meters.is_empty()
            && let Some(e) = enumeration_error
        {
            return Err(Error::Btleplug(e));
        }
        Ok(finalize_discovered(meters))
    }
}

/// Holds the peripheral for the transport's lifetime and disconnects it
/// on drop if this transport initiated the connection.
struct DisconnectGuard {
    peripheral: Peripheral,
    initiated: bool,
}

impl std::ops::Deref for DisconnectGuard {
    type Target = Peripheral;

    fn deref(&self) -> &Peripheral {
        &self.peripheral
    }
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        if !self.initiated {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let peripheral = self.peripheral.clone();
        handle.spawn(async move {
            let _ = peripheral.disconnect().await;
        });
    }
}

async fn all_adapters() -> Result<Vec<Adapter>> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        return Err(Error::NoUsableAdapter);
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
                .ok_or(Error::Disconnected("notification stream ended"))?;
            if notification.uuid == self.data_out_uuid {
                return Ok(notification.value);
            }
        }
    }
}

/// Stops the scans a discovery started, including on error and
/// cancellation paths.
struct ScanGuard {
    adapters: Vec<Adapter>,
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        for adapter in self.adapters.drain(..) {
            handle.spawn(async move {
                let _ = adapter.stop_scan().await;
            });
        }
    }
}
