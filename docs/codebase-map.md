# Codebase Map

Single crate: a library (`src/lib.rs`) holding all logic and a thin binary
(`src/main.rs`).

## Files

- `src/main.rs` — CLI (`Args`), logging setup, signal handling (`cancel_on_signal`).
- `src/app.rs` — task wiring and shutdown sequencing (`run`).
- `src/config.rs` — TOML schema, defaults, validation (`Config`, `ServerConfig`,
  `TraccarConfig`, `UplinkConfig`).
- `src/aprs/callsign.rs` — callsign-SSID rules and device IDs (`is_valid`, `device_id`).

## By concern

- Configuration keys and defaults: `src/config.rs`.
- Callsign validation: `src/aprs/callsign.rs`.
