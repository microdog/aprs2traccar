# Codebase Map

Single crate: a library (`src/lib.rs`) holding all logic and a thin binary
(`src/main.rs`).

## Files

- `src/main.rs` — CLI (`Args`), logging setup, signal handling (`cancel_on_signal`).
- `src/app.rs` — task wiring and shutdown sequencing (`run`).
- `src/config.rs` — TOML schema, defaults, validation (`Config`, `ServerConfig`,
  `TraccarConfig`, `UplinkConfig`).
- `src/aprsis/server.rs` — local APRS-IS listener and client sessions (`serve`,
  `read_login`).
- `src/aprsis/uplink.rs` — receive-only APRS-IS client (`run`).
- `src/aprsis/login.rs` — login line and passcode (`Login`, `passcode`).
- `src/aprsis/lines.rs` — bounded CR/LF line framing (`LineReader`, `MAX_LINE_LEN`).
- `src/ingest.rs` — packet acceptance and mapping to observations (`Ingest::submit`,
  `Origin`, `RecentFixes`).
- `src/dedupe.rs` — 30-second duplicate filter (`Dedupe`).
- `src/traccar/mod.rs` — OsmAnd form encoding (`Observation`, `Fix`).
- `src/traccar/queue.rs` — bounded drop-oldest queue (`Queue`).
- `src/traccar/forwarder.rs` — HTTP delivery and retries (`Forwarder`).
- `src/aprs/mod.rs` — report dispatch by data type (`decode`, `Report`, `Status`).
- `src/aprs/tnc2.rs` — TNC2 header parsing and validation, iGate and
  third-party extraction (`parse`, `Tnc2::igate`, `Tnc2::third_party`).
- `src/aprs/callsign.rs` — aprsc callsign rules and device IDs (`is_valid`,
  `is_valid_path_element`, `device_id`).
- `src/aprs/position.rs` — uncompressed/compressed positions, `CSE/SPD`,
  `/A=`, DAO, ambiguity (`Position`, `parse`).
- `src/aprs/mic_e.rs` — Mic-E decoding and message codes (`decode`, `MicEMessage`).
- `src/aprs/time.rs` — report timestamps and resolution (`PacketTime::resolve`).
- `tests/bridge.rs` — end-to-end tests with fake Traccar and APRS-IS peers
  (`FakeTraccar`, `Bridge`, `Client`).
- `Dockerfile` — container image; `aprs2traccar.example.toml` — annotated
  configuration, checked against the defaults by a `config` test.

## By concern

- Configuration keys and defaults: `src/config.rs`.
- Which packets are forwarded: `src/aprs/mod.rs` (`decode`), `src/ingest.rs`.
- Traccar request fields and attribute names: `src/ingest.rs` (`observation`),
  `src/traccar/mod.rs` (`Observation::to_form`).
- Header validity (what counts as a mangled packet): `src/aprs/tnc2.rs`,
  `src/aprs/callsign.rs`.
- APRS-IS protocol lines (banner, `logresp`, keepalive, refusals):
  `src/aprsis/server.rs`, `src/aprsis/uplink.rs`.
- Delivery guarantees (retries, drops, shutdown flush): `src/traccar/forwarder.rs`,
  `src/traccar/queue.rs`, `src/app.rs`.
- Log setup and rate-limited warnings: `src/main.rs`, `RejectionLog` in
  `src/traccar/forwarder.rs`, `Queue::push`.
