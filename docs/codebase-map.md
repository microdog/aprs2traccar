# Codebase Map

Single crate: a library (`src/lib.rs`) holding all logic and a thin binary
(`src/main.rs`).

## Files

- `src/main.rs` — CLI (`Args`), logging setup, signal handling (`cancel_on_signal`).
- `src/app.rs` — task wiring and shutdown sequencing (`run`).
- `src/config.rs` — TOML schema, defaults, validation (`Config`, `ServerConfig`,
  `TraccarConfig`, `UplinkConfig`).
- `src/aprs/mod.rs` — report dispatch by data type (`decode`, `Report`, `Status`).
- `src/aprs/tnc2.rs` — TNC2 header parsing and validation, iGate and
  third-party extraction (`parse`, `Tnc2::igate`, `Tnc2::third_party`).
- `src/aprs/callsign.rs` — aprsc callsign rules and device IDs (`is_valid`,
  `is_valid_path_element`, `device_id`).
- `src/aprs/position.rs` — uncompressed/compressed positions, `CSE/SPD`,
  `/A=`, DAO, ambiguity (`Position`, `parse`).
- `src/aprs/mic_e.rs` — Mic-E decoding and message codes (`decode`, `MicEMessage`).
- `src/aprs/time.rs` — report timestamps and resolution (`PacketTime::resolve`).

## By concern

- Configuration keys and defaults: `src/config.rs`.
- Which packets are forwarded: `src/aprs/mod.rs` (`decode`).
- Header validity (what counts as a mangled packet): `src/aprs/tnc2.rs`,
  `src/aprs/callsign.rs`.
