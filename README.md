# ble-chirp

Cross-platform CLI messenger that uses **Bluetooth Low Energy advertising** to broadcast and relay tiny messages — a lightweight mesh chat without internet or pairing.

## Build

```bash
cargo build
```

## Usage

Transmit a message with a limited chunk rate:

```
cargo run -- tx --msg "hello" --rate 2
```

`--rate` caps transmissions using a token bucket scheduler, reducing radio congestion and conserving battery.
