use futures::StreamExt;
use std::time::Duration;
use zbus::fdo::{PropertiesChangedStream, PropertiesProxy};
use zbus::zvariant::OwnedObjectPath;

use super::{DiscoveredMeter, METER_NAME_PREFIX, Transport, finalize_discovered};
use crate::error::{Error, Result};

/// UUID of the meter's BLE UART bridge "Data Out" characteristic. The
/// meter streams its readings here as GATT notifications, one frame per
/// notification.
pub const DATA_OUT_UUID: &str = "0000ff02-0000-1000-8000-00805f9b34fb";

const BLUEZ_SERVICE: &str = "org.bluez";
const ADAPTER_IFACE: &str = "org.bluez.Adapter1";
const DEVICE_IFACE: &str = "org.bluez.Device1";
const GATT_CHARACTERISTIC_IFACE: &str = "org.bluez.GattCharacteristic1";
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport over Bluetooth LE using the BlueZ D-Bus API via `bluebus`.
///
/// The device must already be known to BlueZ (i.e. paired or previously
/// discovered). While the transport is alive it holds the GATT
/// notification session; the meter drops the BLE connection when no
/// client holds it.
pub struct BluebusTransport {
    signals: PropertiesChangedStream,
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
        if !device.connected().await? {
            device.connect().await.map_err(|e| Error::ConnectFailed {
                address: address.to_owned(),
                source: Box::new(e),
            })?;
        }
        wait_services_resolved(&device).await?;

        let characteristic_path = find_characteristic(&object_manager, &device_path, DATA_OUT_UUID)
            .await?
            .ok_or_else(|| Error::CharacteristicNotFound {
                uuid: DATA_OUT_UUID.to_owned(),
                address: address.to_owned(),
            })?;

        // Subscribe to property changes before enabling notifications so
        // no frames are missed.
        let properties = PropertiesProxy::builder(connection)
            .destination(BLUEZ_SERVICE)?
            .path(characteristic_path.clone())?
            .build()
            .await?;
        let signals = properties.receive_properties_changed().await?;

        let mut characteristic = ::bluebus::GattCharacteristic1Proxy::builder(connection)
            .destination(BLUEZ_SERVICE)?
            .path(characteristic_path)?
            .build()
            .await?;
        characteristic.start_notify().await?;

        Ok(Self { signals })
    }

    /// Scans for `timeout` and returns the UT325F meters known to
    /// BlueZ, strongest signal first. Devices with `rssi: None` come
    /// from BlueZ's cache (e.g. paired meters currently out of range).
    ///
    /// Scans on every powered adapter; a scan already started by
    /// another Bluetooth client is reused.
    pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredMeter>> {
        let connection = ::bluebus::get_system_connection().await?;
        Self::discover_on(&connection, timeout).await
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

        // Start discovery on every powered adapter. BlueZ returns
        // InProgress if another client is already scanning; that scan
        // serves us just as well.
        let mut adapters = Vec::new();
        for (path, interfaces) in &objects {
            if !interfaces.contains_key(ADAPTER_IFACE) {
                continue;
            }
            let adapter = ::bluebus::AdapterProxy::builder(connection)
                .path(path.clone())?
                .build()
                .await?;
            if !adapter.powered().await.unwrap_or(false) {
                continue;
            }
            let we_started = match adapter.start_discovery().await {
                Ok(()) => true,
                Err(e) if e.to_string().contains("InProgress") => false,
                Err(e) => {
                    return Err(Error::AdapterUnusable {
                        adapter: path.to_string(),
                        source: Box::new(e),
                    });
                }
            };
            adapters.push((adapter, we_started));
        }
        if adapters.is_empty() {
            return Err(Error::NoUsableAdapter);
        }

        tokio::time::sleep(timeout).await;

        let objects = object_manager.get_managed_objects().await;
        for (adapter, we_started) in &adapters {
            if *we_started {
                let _ = adapter.stop_discovery().await;
            }
        }

        let mut meters = Vec::new();
        for interfaces in objects?.values() {
            let Some(properties) = interfaces.get(DEVICE_IFACE) else {
                continue;
            };
            let Some(name) = properties.get("Name") else {
                continue;
            };
            let name: &str = name.downcast_ref()?;
            if !name.starts_with(METER_NAME_PREFIX) {
                continue;
            }
            let Some(address) = properties.get("Address") else {
                continue;
            };
            let address: &str = address.downcast_ref()?;
            let rssi = properties
                .get("RSSI")
                .and_then(|v| v.downcast_ref::<i16>().ok());
            meters.push(DiscoveredMeter {
                address: address.to_owned(),
                name: name.to_owned(),
                rssi,
            });
        }
        Ok(finalize_discovered(meters))
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
        let Some(device_address) = properties.get("Address") else {
            continue;
        };
        let device_address: &str = device_address.downcast_ref()?;
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
        let Some(characteristic_uuid) = properties.get("UUID") else {
            continue;
        };
        let characteristic_uuid: &str = characteristic_uuid.downcast_ref()?;
        if characteristic_uuid.eq_ignore_ascii_case(uuid) {
            return Ok(Some(path.clone()));
        }
    }
    Ok(None)
}

async fn wait_services_resolved(device: &::bluebus::DeviceProxy<'_>) -> Result<()> {
    let mut resolved_changes = device.receive_services_resolved_changed().await;
    if device.services_resolved().await? {
        return Ok(());
    }
    tokio::time::timeout(RESOLVE_TIMEOUT, async {
        while let Some(change) = resolved_changes.next().await {
            if change.get().await.unwrap_or(false) {
                return;
            }
        }
    })
    .await
    .map_err(|_| Error::ConnectTimeout("GATT service discovery".to_owned()))?;
    Ok(())
}
