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
#[command(group = clap::ArgGroup::new("bluetooth").args(["ble", "discover"]))]
// clap does not enforce `requires` aimed at an argument that belongs
// to a group; aim at a single-member group instead.
#[command(group = clap::ArgGroup::new("ble_mode").args(["ble"]))]
struct Args {
    /// The serial port to use
    #[arg(
        required_unless_present_any = ["ble", "discover"],
        conflicts_with_all = ["ble", "discover"]
    )]
    port: Option<String>,

    /// Connect over Bluetooth LE, either to ADDRESS
    /// (e.g. E8:26:CF:F1:23:61) or, with no address, to the only meter
    /// discovered
    #[arg(
        short,
        long,
        value_name = "ADDRESS",
        num_args = 0..=1,
        conflicts_with = "discover"
    )]
    ble: Option<Option<String>>,

    /// Discover meters over Bluetooth LE, print them, and exit
    #[arg(short, long)]
    discover: bool,

    /// Disconnect the meter on exit. By default it is left connected:
    /// a connected meter stays awake and the next run finds it without
    /// a scan.
    #[arg(long, requires = "ble_mode")]
    disconnect: bool,

    /// Bluetooth scan duration in seconds, for --discover and --ble
    /// without an address [default: 8].
    #[arg(long, value_name = "SECONDS", requires = "bluetooth",
          value_parser = clap::value_parser!(u64).range(1..=3600))]
    scan_time: Option<u64>,

    /// Print the held temperatures as well.
    #[arg(short = 'H', long)]
    held_temps: bool,
}

async fn run<T: Transport>(mut meter: Meter<T>, held_temps: bool, disconnect: bool) -> Result<()> {
    // Ctrl-C must also go through teardown: dying with a connection
    // held leaves it dangling in the Bluetooth stack instead of
    // deliberately kept (detach) or released (close).
    let result = tokio::select! {
        result = read_readings(&mut meter, held_temps) => result,
        interrupt = tokio::signal::ctrl_c() => interrupt.map_err(Into::into),
    };
    let torn_down = if disconnect {
        meter.close().await
    } else {
        meter.detach().await
    };
    // A read error is the story; a teardown failure matters only on an
    // otherwise clean exit.
    result.and(torn_down.map_err(Into::into))
}

async fn read_readings<T: Transport>(meter: &mut Meter<T>, held_temps: bool) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    loop {
        let reading = meter
            .read()
            .await
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
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(any(feature = "bluebus", feature = "btleplug"))]
async fn discover(scan_time: std::time::Duration) -> Result<()> {
    let meters = ut325f_rs::BleTransport::discover(scan_time).await?;
    if meters.is_empty() {
        eprintln!("No meters found.");
    }
    for meter in &meters {
        let status = match (meter.connected, meter.rssi) {
            (true, _) => "connected".to_owned(),
            (false, Some(rssi)) => format!("{rssi} dBm"),
            (false, None) => "cached".to_owned(),
        };
        println!("{}  {}  [{}]", meter.address, meter.name, status);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    #[cfg(any(feature = "bluebus", feature = "btleplug"))]
    let scan_time = std::time::Duration::from_secs(args.scan_time.unwrap_or(8));

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
            return run(meter, args.held_temps, args.disconnect).await;
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
        run(
            Meter::open_serial(&port).await?,
            args.held_temps,
            args.disconnect,
        )
        .await
    }
    #[cfg(not(feature = "serial"))]
    {
        let _ = port;
        Err(anyhow!(
            "Built without serial support; rebuild with `--features serial`"
        ))
    }
}
