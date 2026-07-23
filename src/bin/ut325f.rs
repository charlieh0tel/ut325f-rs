use anyhow::Result;
use anyhow::anyhow;
use clap::Parser;
use clap_derive::Parser;

use ut325f_rs::{Meter, Transport};

#[cfg(not(any(feature = "bluebus", feature = "btleplug")))]
const NO_BLE_SUPPORT: &str =
    "Built without Bluetooth support; rebuild with `--features bluebus` or `--features btleplug`";

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The serial port to use
    #[clap(
        required_unless_present_any = ["ble", "discover"],
        conflicts_with_all = ["ble", "discover"]
    )]
    port: Option<String>,

    /// Connect over Bluetooth LE, either to ADDRESS
    /// (e.g. E8:26:CF:F1:23:61) or, with no address, to the only meter
    /// discovered
    #[clap(
        short,
        long,
        value_name = "ADDRESS",
        num_args = 0..=1,
        conflicts_with = "discover",
        group = "bluetooth"
    )]
    ble: Option<Option<String>>,

    /// Discover meters over Bluetooth LE, print them, and exit
    #[clap(short, long, action, group = "bluetooth")]
    discover: bool,

    /// Bluetooth scan duration in seconds, for --discover and --ble
    /// without an address
    #[clap(
        long,
        default_value_t = 8,
        value_name = "SECONDS",
        requires = "bluetooth"
    )]
    scan_time: u64,

    /// Print the held temperatures as well.
    #[clap(short = 'H', long, action)]
    held_temps: bool,
}

async fn run<T: Transport>(mut meter: Meter<T>, held_temps: bool) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    loop {
        // Exit cleanly on Ctrl-C instead of dying by signal, so the
        // meter's transport is dropped and disconnects the BLE device.
        let reading = tokio::select! {
            reading = meter.read() => reading,
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
        .map_err(|e| anyhow!("Error reading data: {}", e))?;
        let written = if held_temps {
            reading.write_all_temps(&mut stdout)
        } else {
            reading.write_current_temps(&mut stdout)
        };
        match written {
            Ok(()) => {}
            // Reading stops when the consumer goes away (e.g. piped to
            // head).
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => break,
            Err(e) => return Err(e.into()),
        }
    }
    // Give the transport's disconnect, spawned by its Drop, a moment to
    // reach the Bluetooth stack before the runtime shuts down.
    drop(meter);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(())
}

#[cfg(any(feature = "bluebus", feature = "btleplug"))]
async fn discover(scan_time: std::time::Duration) -> Result<()> {
    let meters = ut325f_rs::BleTransport::discover(scan_time).await?;
    if meters.is_empty() {
        eprintln!("No meters found.");
    }
    for meter in &meters {
        let status = if meter.connected {
            "connected".to_owned()
        } else {
            meter
                .rssi
                .map_or_else(|| "cached".to_owned(), |rssi| format!("{rssi} dBm"))
        };
        println!("{}  {}  [{}]", meter.address, meter.name, status);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    let scan_time = std::time::Duration::from_secs(args.scan_time);

    if args.discover {
        #[cfg(any(feature = "bluebus", feature = "btleplug"))]
        {
            return discover(scan_time).await;
        }
        #[cfg(not(any(feature = "bluebus", feature = "btleplug")))]
        return Err(anyhow!(NO_BLE_SUPPORT));
    }

    if let Some(address) = &args.ble {
        #[cfg(any(feature = "bluebus", feature = "btleplug"))]
        {
            let meter = match address {
                Some(address) => Meter::open_ble(address).await?,
                None => Meter::open_ble_only(scan_time).await?,
            };
            return run(meter, args.held_temps).await;
        }
        #[cfg(not(any(feature = "bluebus", feature = "btleplug")))]
        {
            let _ = address;
            return Err(anyhow!(NO_BLE_SUPPORT));
        }
    }

    let port = args.port.expect("clap enforces port when --ble is absent");
    #[cfg(feature = "serial")]
    {
        run(Meter::open_serial(&port).await?, args.held_temps).await
    }
    #[cfg(not(feature = "serial"))]
    {
        let _ = port;
        Err(anyhow!(
            "Built without serial support; rebuild with `--features serial`"
        ))
    }
}
