# Agent Onboarding

aprs2traccar is a single Rust binary that acts as an APRS-IS compatible
server for local APRS clients and forwards decoded positions and status
reports to Traccar over the OsmAnd HTTP protocol.

## Stack and topology

- Rust (edition 2024) on tokio; exact dependency versions live in
  [`Cargo.toml`](../Cargo.toml) and `Cargo.lock`.
- Inbound: APRS-IS clients connect over TCP (default port 14580); an optional
  uplink pulls a filtered feed from an APRS-IS server.
- Outbound: HTTP POSTs to Traccar's OsmAnd port (default 5055).
- Deployment: the binary or the container image built from
  [`Dockerfile`](../Dockerfile).

## Conventions and invariants

- The bridge only receives APRS data. Clients get nothing but `#` lines;
  APRS-IS gets nothing but the uplink login, always with passcode `-1`.
- Only verified client logins may submit packets.
- APRS-IS behavior follows the aprs-is.net specifications; where they are
  silent, defaults and callsign rules mirror aprsc.
- Reports without coordinates are never sent for a device without a recent
  position, because Traccar would place them at 0°,0°.
- Traccar delivery is in order; the queue is bounded, in memory, and drops
  the oldest report when full.
- Configuration is TOML with `deny_unknown_fields`; every key has a default
  and is validated at startup. Keep the README configuration table and
  `aprs2traccar.example.toml` in sync with `src/config.rs`.
- Tests must not depend on proxy settings: point HTTP clients at
  `localhost`, which `NO_PROXY` conventionally covers.

## Documentation index

| Question | Source |
| --- | --- |
| Setup, commands, configuration, operations | [`README.md`](../README.md) |
| Forwarded data, Traccar setup, delivery semantics | [`README.md`](../README.md) |
| Where code for a concern lives | [`docs/codebase-map.md`](codebase-map.md) |
| Repository rules for agents | [`AGENTS.md`](../AGENTS.md) |
