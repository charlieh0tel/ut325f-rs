/// Errors returned by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("bad sync header")]
    BadSyncHeader,

    #[error("checksum mismatch")]
    ChecksumMismatch,

    #[error("invalid hold type {0:#04x}")]
    InvalidHoldType(u8),

    #[error("malformed frame: {0}")]
    MalformedFrame(&'static str),

    #[error("timeout reading data")]
    ReadTimeout,

    #[error("transport disconnected: {0}")]
    Disconnected(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[cfg(feature = "serial")]
    #[error("failed to open serial port {port}: {source}")]
    SerialOpen {
        port: String,
        source: tokio_serial::Error,
    },

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("timeout connecting to {0}")]
    ConnectTimeout(String),

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("failed to connect to {address}: {source}")]
    ConnectFailed {
        address: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("Bluetooth device {0} is not known; pair it or run discovery first")]
    DeviceNotKnown(String),

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("characteristic {uuid} not found on {address}; is this a UT325F?")]
    CharacteristicNotFound { uuid: String, address: String },

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("no UT325F meters found")]
    NoMetersFound,

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("multiple UT325F meters found ({}); open one by address", .0.join(", "))]
    MultipleMetersFound(Vec<String>),

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("no powered Bluetooth adapter found")]
    NoUsableAdapter,

    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    #[error("Bluetooth adapter {adapter} unusable: {source}")]
    AdapterUnusable {
        adapter: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[cfg(feature = "btleplug")]
    #[error("invalid Bluetooth address '{0}'")]
    InvalidAddress(String),

    #[cfg(feature = "btleplug")]
    #[error("device {address} not found; an adapter also failed enumeration: {source}")]
    DeviceSearchIncomplete {
        address: String,
        source: btleplug::Error,
    },

    #[cfg(feature = "bluebus")]
    #[error(transparent)]
    Zbus(#[from] zbus::Error),

    #[cfg(feature = "bluebus")]
    #[error(transparent)]
    ZbusFdo(#[from] zbus::fdo::Error),

    #[cfg(feature = "bluebus")]
    #[error(transparent)]
    Zvariant(#[from] zbus::zvariant::Error),

    #[cfg(feature = "btleplug")]
    #[error(transparent)]
    Btleplug(#[from] btleplug::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
