use anyhow::Result;
use clap::Parser;
use clap_derive::Parser;

use ut325f_rs::Meter;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// The serial port to use
    port: String,

    /// Print the held temperatures as well.
    #[clap(short = 'H', long, action)]
    held_temps: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut meter = Meter::new(args.port);

    meter.open().await?;

    loop {
        match meter.read().await {
            Ok(reading) => {
                if args.held_temps {
                    reading.print_all_temps();
                } else {
                    reading.print_current_temps();
                }
            }
            Err(e) => eprintln!("Error reading data: {}", e),
        }
    }
}
