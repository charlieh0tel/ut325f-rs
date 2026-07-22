A package and CLI to read temperatures from Uni-T UT325F 4-channel
temperature meter.

## Transports

- **USB serial** (feature `serial`, on by default):

  ```sh
  ut325f /dev/ttyUSB0
  ```

- **Bluetooth LE**, with a choice of backend. The meter must already be
  paired with / known to the Bluetooth stack:

  - feature `bluebus`: the BlueZ D-Bus API via the `bluebus` crate
    (Linux only)
  - feature `btleplug`: the cross-platform `btleplug` crate

  ```sh
  cargo build --features bluebus   # or --features btleplug
  ut325f --ble E8:26:CF:F1:23:61
  ```

  If both BLE features are enabled, `Meter::open_ble` uses `bluebus`;
  the concrete `BluebusTransport`/`BtleplugTransport` types select a
  backend explicitly.

## Library

```rust
let mut meter = ut325f_rs::Meter::open_serial("/dev/ttyUSB0").await?; // feature "serial"
let mut meter = ut325f_rs::Meter::open_ble("E8:26:CF:F1:23:61").await?; // feature "bluebus" or "btleplug"
let reading = meter.read().await?;
```

Transports are pluggable: anything implementing the `Transport` trait
(a source of arbitrarily chunked bytes) can back a `Meter`; framing and
parsing are handled by `FrameDecoder` and `Reading`. To use another
stack, implement `Transport` on top of its notification stream for the
`0000ff02-...` characteristic and pass it to `Meter::new`.
