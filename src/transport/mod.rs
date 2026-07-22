use anyhow::Result;

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
