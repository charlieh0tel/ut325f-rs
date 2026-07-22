use crate::error::Result;

#[cfg(feature = "bluebus")]
mod bluebus;
#[cfg(feature = "btleplug")]
mod btleplug;
#[cfg(feature = "serial")]
mod serial;

#[cfg(feature = "bluebus")]
pub use bluebus::BluebusTransport;
#[cfg(feature = "btleplug")]
pub use btleplug::BtleplugTransport;
#[cfg(feature = "serial")]
pub use serial::SerialTransport;

/// UUID of the meter's BLE UART bridge "Data Out" characteristic. The
/// meter streams its readings here as GATT notifications, one frame per
/// notification.
#[cfg(any(feature = "bluebus", feature = "btleplug"))]
pub const DATA_OUT_UUID: &str = "0000ff02-0000-1000-8000-00805f9b34fb";

/// Prefix of the name the meter advertises (e.g. "UT325F AF37C47963D4").
#[cfg(any(feature = "bluebus", feature = "btleplug"))]
pub const METER_NAME_PREFIX: &str = "UT325F";

/// A meter found by a BLE backend's `discover`.
#[cfg(any(feature = "bluebus", feature = "btleplug"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredMeter {
    /// Bluetooth address, suitable for [`Meter::open_ble`](crate::Meter).
    pub address: String,
    /// Advertised device name.
    pub name: String,
    /// Signal strength if the device was seen during the scan; `None`
    /// for devices only known from the stack's cache (e.g. paired but
    /// currently out of range or powered off).
    pub rssi: Option<i16>,
}

/// Sorts strongest signal first (unseen devices last), keeping only the
/// strongest observation of each address (a meter can be seen by more
/// than one adapter).
#[cfg(any(feature = "bluebus", feature = "btleplug"))]
fn finalize_discovered(mut meters: Vec<DiscoveredMeter>) -> Vec<DiscoveredMeter> {
    meters.sort_by(|a, b| {
        a.address
            .to_ascii_uppercase()
            .cmp(&b.address.to_ascii_uppercase())
            .then(b.rssi.unwrap_or(i16::MIN).cmp(&a.rssi.unwrap_or(i16::MIN)))
    });
    meters.dedup_by(|a, b| a.address.eq_ignore_ascii_case(&b.address));
    meters.sort_by_key(|m| std::cmp::Reverse(m.rssi.unwrap_or(i16::MIN)));
    meters
}

#[cfg(all(test, any(feature = "bluebus", feature = "btleplug")))]
mod tests {
    use super::*;

    fn meter(address: &str, rssi: Option<i16>) -> DiscoveredMeter {
        DiscoveredMeter {
            address: address.to_owned(),
            name: format!("UT325F {address}"),
            rssi,
        }
    }

    #[test]
    fn test_finalize_keeps_strongest_observation() {
        let meters = finalize_discovered(vec![
            meter("AA:00:00:00:00:01", Some(-90)),
            meter("AA:00:00:00:00:02", Some(-60)),
            meter("aa:00:00:00:00:01", Some(-30)),
            meter("AA:00:00:00:00:03", None),
        ]);
        let summary: Vec<_> = meters
            .iter()
            .map(|m| (m.address.as_str(), m.rssi))
            .collect();
        assert_eq!(
            summary,
            [
                ("aa:00:00:00:00:01", Some(-30)),
                ("AA:00:00:00:00:02", Some(-60)),
                ("AA:00:00:00:00:03", None),
            ]
        );
    }
}

#[cfg(any(feature = "bluebus", feature = "btleplug"))]
fn exactly_one(mut meters: Vec<DiscoveredMeter>) -> Result<DiscoveredMeter> {
    match meters.len() {
        0 => Err(crate::error::Error::NoMetersFound),
        1 => Ok(meters.remove(0)),
        _ => Err(crate::error::Error::MultipleMetersFound(
            meters.into_iter().map(|m| m.address).collect(),
        )),
    }
}

/// The Bluetooth LE transport backend selected by feature flags. When
/// both BLE features are enabled, `bluebus` is preferred; use the
/// concrete transport types to pick explicitly.
#[cfg(feature = "bluebus")]
pub type BleTransport = BluebusTransport;
#[cfg(all(feature = "btleplug", not(feature = "bluebus")))]
pub type BleTransport = BtleplugTransport;

/// A byte-oriented connection to a UT325F meter.
///
/// Implementations deliver the meter's output as chunks of bytes with no
/// size or alignment guarantees; framing is handled by
/// [`FrameDecoder`](crate::FrameDecoder).
pub trait Transport {
    /// Receives the next non-empty chunk of bytes from the meter.
    fn recv(&mut self) -> impl Future<Output = Result<Vec<u8>>> + Send;
}
