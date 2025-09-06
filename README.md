# ble-chirp

## What This Is

- Peer-to-peer, offline messenger over BLE advertising with hop relaying — no pairing, servers, or phone numbers.
- Lightweight mesh via compact frames: chunking/reassembly, TTL-based relay, and duplicate suppression to keep propagation local and efficient.
- Optional end-to-end encryption: passphrase-derived key (SHA‑256) with ChaCha20‑Poly1305 AEAD; deterministic per-chunk nonce from `msg_id` + `seq`.
- CLI and interactive terminal UI (clap + ratatui); room names hash to topics for simple ad-hoc channels.
- Token-bucket rate limiting to reduce RF congestion and save battery during advertising bursts.
- Cross-platform: Rust core using `btleplug`; Node.js transmitter (`@abandonware/bleno`) to work around macOS advertising limits.

Peer-to-peer, offline chat over Bluetooth Low Energy (BLE) advertising. ble-chirp broadcasts tiny message chunks that nearby devices can read and optionally relay, forming a lightweight, infrastructure-free mesh.

## Inspiration: Bitchat

This project is inspired by Jack Dorsey’s Bitchat — a peer-to-peer, offline messenger that uses BLE mesh-style relaying so people nearby can chat without internet, phone numbers, or servers. 

## Core Fundamentals

- Bluetooth mesh-style relaying: Messages are split into small frames that hop device-to-device using BLE advertising. A per-frame TTL limits propagation to keep things local.
- No accounts or servers: There’s no login, profile, or backend. You pick a topic (or a room name that hashes to a topic) and start chatting.
- Optional encryption: Provide a `--passphrase` to encrypt payloads end-to-end per message chunk using ChaCha20‑Poly1305 with a key derived from SHA‑256 of the passphrase. Without a passphrase, messages are plaintext.
- Local resilience: Works without internet or cell service — useful at festivals, events, remote areas, or during outages.
- Rapid, experimental build: Like Bitchat’s “vibe coding” ethos, ble-chirp prioritizes a minimal, working core you can read and adapt.

## Status & Security Caveats

- Experimental: This is prototype software and has not undergone external security review.
- No identity/auth: There’s no identity layer, so spoofing and impersonation are possible. Don’t rely on this for high-assurance scenarios.
- Metadata leakage: BLE advertisement timing and radio metadata can be observed. Use at your own risk.
- Platform limits: Advertising via `btleplug` is not supported on macOS; use the Node sender below to test TX on macOS.

## Build

```bash
cargo build
```

## Usage

Transmit a message (Rust implementation):

```
cargo run -- tx --msg "hello world" --rate 2
```

Receive and optionally relay messages:

```
cargo run -- rx --relay true
```

Room names (hashed to a topic):

```
cargo run -- tx --room "my-room" --msg "hi"
cargo run -- rx --room "my-room"
```

End-to-end payload encryption via passphrase:

```
cargo run -- tx --room "my-room" --passphrase "correct horse" --msg "secret"
cargo run -- rx --room "my-room" --passphrase "correct horse"
```

Interactive chat UI (single topic/room):

```
cargo run -- chat --room "my-room"
```

Rate limiting

- `--rate` caps transmissions using a token-bucket scheduler to reduce radio congestion and conserve battery.

## macOS TX via Node (workaround)

Advertising is not supported by `btleplug` on macOS. Use the included Node transmitter to send messages from macOS while receiving with the Rust app on other devices:

```
cd node-tx
npm install
node tx.js --topic 7 --ttl 3 --pass "correct horse" hello from node
```

The Node script uses the same frame format and ChaCha20‑Poly1305 encryption as the Rust code, so it interoperates with `rx` and `chat` modes.
