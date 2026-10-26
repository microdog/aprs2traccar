# Agent Onboarding

aprs2traccar is a single Rust binary that acts as an APRS-IS compatible
server for local APRS clients and forwards decoded positions and status
reports to Traccar over the OsmAnd HTTP protocol.

## Stack and topology

- Rust (edition 2024) on tokio; exact dependency versions live in
  [`Cargo.toml`](../Cargo.toml) and `Cargo.lock`.
- Inbound: APRS-IS clients connect over TCP (default port 14580).
- Outbound: HTTP POSTs to Traccar's OsmAnd port (default 5055).

## Conventions and invariants

- APRS-IS behavior follows the aprs-is.net specifications; where they are
  silent, defaults mirror aprsc.
- Configuration is TOML with `deny_unknown_fields`; every key has a default
  and is validated at startup.
- Keep the README configuration table in sync with `src/config.rs`.

## Documentation index

| Question | Source |
|---|---|
| Setup, commands, configuration, operations | [`README.md`](../README.md) |
| Where code for a concern lives | [`docs/codebase-map.md`](codebase-map.md) |
| Repository rules for agents | [`AGENTS.md`](../AGENTS.md) |
