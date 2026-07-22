//! Discover UT325F meters over Bluetooth LE.
//!
//! Requires a BLE feature: `cargo run --example discover --features bluebus`
//! (or `--features btleplug`).

#[cfg(any(feature = "bluebus", feature = "btleplug"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let timeout = std::time::Duration::from_secs(8);
    eprintln!("Scanning for {}s...", timeout.as_secs());
    let meters = ut325f_rs::BleTransport::discover(timeout).await?;
    if meters.is_empty() {
        eprintln!("No meters found.");
    }
    for meter in &meters {
        let rssi = meter
            .rssi
            .map_or_else(|| "cached".to_owned(), |rssi| format!("{rssi} dBm"));
        println!("{}  {}  [{}]", meter.address, meter.name, rssi);
    }
    Ok(())
}

#[cfg(not(any(feature = "bluebus", feature = "btleplug")))]
fn main() {
    eprintln!("Rebuild with `--features bluebus` or `--features btleplug`.");
}
