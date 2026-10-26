# aprs2traccar

An APRS-IS compatible server that accepts APRS packets from local clients
(iGates, APRSdroid, other APRS-IS clients) and forwards station positions and
status reports to [Traccar](https://www.traccar.org/) through its
[OsmAnd protocol](https://www.traccar.org/osmand/).

## Requirements

- Rust toolchain with edition 2024 support (developed with Rust 1.99).
- A Traccar server with the OsmAnd protocol port reachable (5055 by default).

## Running

```sh
cargo run --release -- --config aprs2traccar.toml
```

Without `--config`, built-in defaults apply. Log verbosity follows `RUST_LOG`
(default `info`), e.g. `RUST_LOG=aprs2traccar=debug`. SIGINT and SIGTERM stop
the process gracefully.

## Configuration

TOML with three optional sections; unknown keys are rejected.

| Key | Default | Meaning |
|---|---|---|
| `server.listen` | `0.0.0.0:14580` | APRS-IS listener address |
| `server.server_call` | `APRS2TRC` | Server identity in `# logresp` and keepalive lines |
| `server.allowed_logins` | `[]` | Logins allowed to connect (case-insensitive); empty allows any |
| `server.login_timeout_secs` | `30` | Time to send the login line |
| `server.keepalive_secs` | `20` | Interval of `#` keepalive lines |
| `server.client_timeout_secs` | `172800` | Disconnect after this long without input |
| `traccar.url` | `http://127.0.0.1:5055/` | OsmAnd endpoint; must not contain a query string |
| `traccar.request_timeout_secs` | `10` | Per-request timeout |
| `traccar.queue_capacity` | `10000` | Buffered positions; oldest dropped when full |
| `traccar.max_attempts` | `5` | Failed sends before a position is dropped |
| `traccar.shutdown_grace_secs` | `10` | Time to flush the queue on shutdown |
| `uplink.enabled` | `false` | Pull a filtered feed from APRS-IS |
| `uplink.server` | `rotate.aprs2.net:14580` | Upstream APRS-IS server |
| `uplink.callsign` | — | Login callsign (required when enabled) |
| `uplink.filter` | — | Server-side filter (required when enabled) |
| `uplink.idle_timeout_secs` | `120` | Reconnect after this long without input |

## Development

| Task | Command |
|---|---|
| Build | `cargo build` |
| Test | `cargo test` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| Format | `cargo fmt` (check: `cargo fmt --check`) |
| Type-check | `cargo check --all-targets` |
