use anyhow::Result;
use anyhow::anyhow;
use clap::Parser;
use clap_derive::Parser;

use ut325f_rs::{Meter, Transport};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The serial port to use
    #[clap(required_unless_present = "ble", conflicts_with = "ble")]
    port: Option<String>,

    /// Connect over Bluetooth LE to the meter with this Bluetooth
    /// address (e.g. E8:26:CF:F1:23:61)
    #[clap(short, long, value_name = "ADDRESS")]
    ble: Option<String>,

    /// Print the held temperatures as well.
    #[clap(short = 'H', long, action)]
    held_temps: bool,
}

async fn run<T: Transport>(mut meter: Meter<T>, held_temps: bool) -> Result<()> {
    loop {
        match meter.read().await {
            Ok(reading) => {
                if held_temps {
                    reading.print_all_temps();
                } else {
                    reading.print_current_temps();
                }
            }
            Err(e) => return Err(anyhow!("Error reading data: {}", e)),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(address) = &args.ble {
        #[cfg(any(feature = "bluebus", feature = "btleplug"))]
        {
            return run(Meter::open_ble(address).await?, args.held_temps).await;
        }
        #[cfg(not(any(feature = "bluebus", feature = "btleplug")))]
        {
            let _ = address;
            return Err(anyhow!(
                "Built without Bluetooth support; rebuild with `--features bluebus` or `--features btleplug`"
            ));
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
