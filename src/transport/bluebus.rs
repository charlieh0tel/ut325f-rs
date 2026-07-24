use futures::StreamExt;
use std::time::Duration;
use zbus::fdo::{PropertiesChangedStream, PropertiesProxy};
use zbus::zvariant::OwnedObjectPath;

use super::{DATA_OUT_UUID, DiscoveredMeter, METER_NAME_PREFIX, Transport, finalize_discovered};
use crate::error::{Error, Result};

const BLUEZ_SERVICE: &str = "org.bluez";
const ADAPTER_IFACE: &str = "org.bluez.Adapter1";
const DEVICE_IFACE: &str = "org.bluez.Device1";
const GATT_CHARACTERISTIC_IFACE: &str = "org.bluez.GattCharacteristic1";
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport over Bluetooth LE using the BlueZ D-Bus API via `bluebus`.
///
/// The device must already be known to BlueZ (i.e. paired or previously
/// discovered). Teardown: `close` disconnects the device, `detach`
/// deliberately leaves it connected (and awake), and drop makes a
/// best-effort disconnect of a connection this transport initiated.
/// A connected meter stops advertising, so a leaked connection would
/// hide it from every later scan.
pub struct BluebusTransport {
    signals: PropertiesChangedStream,
    // The notification session lives on its own D-Bus connection. BlueZ
    // scopes sessions to the client connection and ends them when it
    // closes, so dropping the transport reliably stops notifications on
    // every path (timeout, cancellation, any thread) without racing
    // other transports that share the caller's connection.
    _notify_connection: zbus::Connection,
    // Unlike notify sessions, BlueZ device connections are adapter-level
    // state that no client teardown cleans up; this guard disconnects on
    // drop, and only if we initiated the connection.
    _device: DisconnectGuard,
}

impl BluebusTransport {
    /// Connects to the meter with the given Bluetooth address
    /// (e.g. "E8:26:CF:F1:23:61") and starts notifications.
    pub async fn open(address: &str) -> Result<Self> {
        let connection = ::bluebus::get_system_connection().await?;
        Self::open_on(&connection, address).await
    }

    /// Like [`open`](Self::open), but reuses an existing zbus connection
    /// (e.g. the one an application already holds for bluebus).
    pub async fn open_on(connection: &zbus::Connection, address: &str) -> Result<Self> {
        tokio::time::timeout(OPEN_TIMEOUT, Self::open_inner(connection, address))
            .await
            .map_err(|_| Error::ConnectTimeout(address.to_owned()))?
    }

    async fn open_inner(connection: &zbus::Connection, address: &str) -> Result<Self> {
        let object_manager = ::bluebus::ObjectManagerProxy::builder(connection)
            .build()
            .await?;

        let device_path = find_device(&object_manager, address)
            .await?
            .ok_or_else(|| Error::DeviceNotKnown(address.to_owned()))?;

        let device = ::bluebus::DeviceProxy::builder(connection)
            .path(device_path.clone())?
            .build()
            .await?;
        // The guard covers cancellation for the rest of open (e.g. the
        // open timeout firing) and then rides in the transport; on drop
        // it only disconnects a connection this call established.
        let mut device_guard = DisconnectGuard {
            device: device.clone(),
            initiated: false,
        };
        if !device.connected().await? {
            device.connect().await.map_err(|e| Error::ConnectFailed {
                address: address.to_owned(),
                source: Box::new(e),
            })?;
            device_guard.initiated = true;
        }
        wait_services_resolved(&device).await?;

        let characteristic_path = find_characteristic(&object_manager, &device_path, DATA_OUT_UUID)
            .await?
            .ok_or_else(|| Error::CharacteristicNotFound {
                uuid: DATA_OUT_UUID.to_owned(),
                address: address.to_owned(),
            })?;

        let notify_connection = ::bluebus::get_system_connection().await?;

        // Subscribe to property changes before enabling notifications so
        // no frames are missed.
        let properties = PropertiesProxy::builder(&notify_connection)
            .destination(BLUEZ_SERVICE)?
            .path(characteristic_path.clone())?
            .build()
            .await?;
        let signals = properties.receive_properties_changed().await?;

        let mut characteristic = ::bluebus::GattCharacteristic1Proxy::builder(&notify_connection)
            .destination(BLUEZ_SERVICE)?
            .path(characteristic_path)?
            .build()
            .await?;
        characteristic.start_notify().await?;

        Ok(Self {
            signals,
            _notify_connection: notify_connection,
            _device: device_guard,
        })
    }

    /// Scans for `timeout` and returns the UT325F meters known to
    /// BlueZ, strongest signal first. Devices with `rssi: None` come
    /// from BlueZ's cache (e.g. paired meters currently out of range).
    ///
    /// Scans on every powered adapter; a scan already started by
    /// another Bluetooth client is reused, and adapters that fail are
    /// skipped as long as at least one is usable.
    pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredMeter>> {
        let connection = ::bluebus::get_system_connection().await?;
        Self::discover_on(&connection, timeout).await
    }

    /// Like [`discover`](Self::discover), but reuses an existing zbus
    /// connection.
    pub async fn discover_on(
        connection: &zbus::Connection,
        timeout: Duration,
    ) -> Result<Vec<DiscoveredMeter>> {
        let object_manager = ::bluebus::ObjectManagerProxy::builder(connection)
            .build()
            .await?;
        let objects = object_manager.get_managed_objects().await?;

        // Discovery runs on its own D-Bus connection: BlueZ ends a
        // client's scans when its connection closes, so scans cannot
        // outlive this call even on cancellation or a failed
        // StopDiscovery below.
        let scan_connection = ::bluebus::get_system_connection().await?;

        // Start discovery on every powered adapter. BlueZ returns
        // InProgress if another client is already scanning; that scan
        // serves us just as well and is not ours to stop.
        let mut started = Vec::new();
        let mut usable = 0;
        let mut adapter_error = None;
        for (path, interfaces) in &objects {
            if !interfaces.contains_key(ADAPTER_IFACE) {
                continue;
            }
            let adapter = ::bluebus::AdapterProxy::builder(&scan_connection)
                .path(path.clone())?
                .build()
                .await?;
            match adapter.powered().await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    adapter_error = Some(Error::AdapterUnusable {
                        adapter: path.to_string(),
                        source: Box::new(e),
                    });
                    continue;
                }
            }
            match adapter.start_discovery().await {
                Ok(()) => {
                    started.push(adapter);
                    usable += 1;
                }
                Err(e) if e.to_string().contains("InProgress") => usable += 1,
                Err(e) => {
                    adapter_error = Some(Error::AdapterUnusable {
                        adapter: path.to_string(),
                        source: Box::new(e),
                    });
                }
            }
        }
        if usable == 0 {
            return Err(adapter_error.unwrap_or(Error::NoUsableAdapter));
        }

        tokio::time::sleep(timeout).await;

        let objects = object_manager.get_managed_objects().await?;
        for adapter in &started {
            let _ = adapter.stop_discovery().await;
        }
        drop(scan_connection);

        let mut meters = Vec::new();
        for interfaces in objects.values() {
            let Some(properties) = interfaces.get(DEVICE_IFACE) else {
                continue;
            };
            // Skip devices with missing or oddly-typed properties
            // rather than failing the whole scan.
            let Some(Ok(name)) = properties.get("Name").map(|v| v.downcast_ref::<&str>()) else {
                continue;
            };
            if !name.starts_with(METER_NAME_PREFIX) {
                continue;
            }
            let Some(Ok(address)) = properties.get("Address").map(|v| v.downcast_ref::<&str>())
            else {
                continue;
            };
            let rssi = properties
                .get("RSSI")
                .and_then(|v| v.downcast_ref::<i16>().ok());
            let connected = properties
                .get("Connected")
                .and_then(|v| v.downcast_ref::<bool>().ok())
                .unwrap_or(false);
            meters.push(DiscoveredMeter {
                address: address.to_owned(),
                name: name.to_owned(),
                rssi,
                connected,
            });
        }
        Ok(finalize_discovered(meters))
    }

    /// Discovers meters for `timeout` and connects to the only meter
    /// found; errors if there are none or more than one (opening an
    /// arbitrary meter could pick the wrong one).
    pub async fn open_only(timeout: Duration) -> Result<Self> {
        let connection = ::bluebus::get_system_connection().await?;
        Self::open_only_on(&connection, timeout).await
    }

    /// Like [`open_only`](Self::open_only), but reuses an existing zbus
    /// connection.
    pub async fn open_only_on(connection: &zbus::Connection, timeout: Duration) -> Result<Self> {
        let meter = super::exactly_one(Self::discover_on(connection, timeout).await?)?;
        Self::open_on(connection, &meter.address).await
    }

    /// Ends the notification session (by closing its dedicated D-Bus
    /// connection, which drops the session and match rules server-side)
    /// without touching the device connection, and disarms the drop
    /// guard. Returns the device proxy.
    async fn end_notifications(self) -> ::bluebus::DeviceProxy<'static> {
        let Self {
            signals,
            _notify_connection: notify_connection,
            _device: mut guard,
        } = self;
        let device = guard.device.clone();
        guard.initiated = false;
        drop(guard);
        drop(signals);
        notify_connection.graceful_shutdown().await;
        device
    }
}

impl Transport for BluebusTransport {
    async fn recv(&mut self) -> Result<Vec<u8>> {
        loop {
            let signal = self
                .signals
                .next()
                .await
                .ok_or(Error::Disconnected("notification stream ended"))?;
            let args = signal.args()?;
            if args.interface_name.as_str() != GATT_CHARACTERISTIC_IFACE {
                continue;
            }
            if let Some(value) = args.changed_properties.get("Value") {
                return Ok(value.try_clone()?.try_into()?);
            }
        }
    }

    async fn close(self) -> Result<()> {
        let device = self.end_notifications().await;
        if device.connected().await? {
            device.disconnect().await?;
        }
        Ok(())
    }

    async fn detach(self) -> Result<()> {
        self.end_notifications().await;
        Ok(())
    }
}

/// Holds the device proxy for the transport's lifetime and, on drop,
/// disconnects only a connection this transport initiated (drop
/// expresses no intent, so it must not release a connection another
/// client established). Best-effort only: the spawned disconnect does
/// not survive runtime shutdown, so graceful teardown must go through
/// `close` or `detach`.
struct DisconnectGuard {
    device: ::bluebus::DeviceProxy<'static>,
    initiated: bool,
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        if !self.initiated {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let device = self.device.clone();
        handle.spawn(async move {
            let _ = device.disconnect().await;
        });
    }
}

async fn find_device(
    object_manager: &::bluebus::ObjectManagerProxy<'_>,
    address: &str,
) -> Result<Option<OwnedObjectPath>> {
    let objects = object_manager.get_managed_objects().await?;
    for (path, interfaces) in &objects {
        let Some(properties) = interfaces.get(DEVICE_IFACE) else {
            continue;
        };
        let Some(Ok(device_address)) = properties.get("Address").map(|v| v.downcast_ref::<&str>())
        else {
            continue;
        };
        if device_address.eq_ignore_ascii_case(address) {
            return Ok(Some(path.clone()));
        }
    }
    Ok(None)
}

async fn find_characteristic(
    object_manager: &::bluebus::ObjectManagerProxy<'_>,
    device_path: &OwnedObjectPath,
    uuid: &str,
) -> Result<Option<OwnedObjectPath>> {
    let device_prefix = format!("{}/", device_path.as_str());
    let objects = object_manager.get_managed_objects().await?;
    for (path, interfaces) in &objects {
        if !path.as_str().starts_with(&device_prefix) {
            continue;
        }
        let Some(properties) = interfaces.get(GATT_CHARACTERISTIC_IFACE) else {
            continue;
        };
        let Some(Ok(characteristic_uuid)) =
            properties.get("UUID").map(|v| v.downcast_ref::<&str>())
        else {
            continue;
        };
        if characteristic_uuid.eq_ignore_ascii_case(uuid) {
            return Ok(Some(path.clone()));
        }
    }
    Ok(None)
}

/// Waits for BlueZ to finish GATT service discovery. Unbounded; the
/// caller's open timeout provides the bound.
async fn wait_services_resolved(device: &::bluebus::DeviceProxy<'_>) -> Result<()> {
    let mut resolved_changes = device.receive_services_resolved_changed().await;
    if device.services_resolved().await? {
        return Ok(());
    }
    while let Some(change) = resolved_changes.next().await {
        if change.get().await.unwrap_or(false) {
            return Ok(());
        }
    }
    Err(Error::Disconnected("service discovery signal stream ended"))
}
