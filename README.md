# xenomit-core

Shared functionality for xenomit services. Currently: `logging`, a `tracing`
setup wired to a redacting console layer and OTLP (OpenTelemetry) export.

## logging

Services instrument with the `tracing` facade (`info!`, `warn!`,
`#[instrument]`, …) and call `init` once at startup:

```rust
use xenomit_core::logging;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = logging::init("xenomit-client")?;
    logging::tracing::info!("started");
    Ok(())
}
```

* **Console**: events print to stderr as `[service] LEVEL target: message
  fields`. URLs, session ids, cookies and tokens are redacted (ported from
  `xenomit-client`'s diagnostics), and each `(target, level)` pair is
  rate-capped so a noisy dependency cannot flood the terminal.
* **OpenTelemetry**: when `OTEL_EXPORTER_OTLP_ENDPOINT` (or the
  signal-specific `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` /
  `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`) is set, spans export as OTLP traces and
  events as OTLP log records over gRPC. Log records carry `trace_id` /
  `span_id`, so they correlate with `#[tracing::instrument]` spans in the
  backend. `OTEL_SDK_DISABLED=true` forces export off.

Drop the returned guard before the tokio runtime exits to flush buffered
telemetry.

```rust
use xenomit_core::logging::{self, Config, Otlp};

let _guard = logging::init_with(
    Config::new("xenomit-backpage")
        .with_service_version(env!("CARGO_PKG_VERSION"))
        .with_filter("info,xenomit_backpage=debug")
        .with_otlp(Otlp::Enabled { endpoint: None }),
)?;
```

Filtering follows the `RUST_LOG`/`EnvFilter` directives; `with_filter`
overrides `RUST_LOG`. Try the demo:

```sh
cargo run -p xenomit_core --example logging
```
