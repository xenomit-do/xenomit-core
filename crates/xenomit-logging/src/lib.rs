//! Shared logging setup for xenomit services.
//!
//! Services keep emitting `tracing` events as usual; this crate decides where
//! they go:
//!
//! * a redacting console layer on stderr (modeled on `xenomit-client`'s
//!   diagnostics — URLs, session ids and tokens never reach the terminal, and
//!   noisy targets are rate-capped), and
//! * OpenTelemetry: spans are exported as OTLP traces and events as OTLP log
//!   records when `OTEL_EXPORTER_OTLP_ENDPOINT` (or a signal-specific
//!   endpoint) points at a collector. Log records carry `trace_id`/`span_id`
//!   so they correlate with the traces produced by `#[tracing::instrument]`.
//!
//! ```no_run
//! use xenomit_logging as logging;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Keep the guard alive until shutdown; dropping it flushes exporters.
//!     let _guard = logging::init("xenomit-client")?;
//!     tracing::info!("hello");
//!     Ok(())
//! }
//! ```

mod config;
mod console;
mod error;
mod otel;
mod redact;

pub use config::{Config, Otlp};
pub use error::Error;

// Re-export the instrumentation facade so services only need this crate.
pub use tracing;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Install logging with the default [`Config`] for `service_name`: console
/// output with redaction, `RUST_LOG` (default `info`) filtering, and OTLP
/// export when a collector endpoint is configured in the environment.
///
/// When OTLP is enabled this must be called from inside a tokio runtime
/// (e.g. `#[tokio::main]`); without one the exporters cannot run and a
/// warning is printed instead.
pub fn init(service_name: impl Into<std::borrow::Cow<'static, str>>) -> Result<Guard, Error> {
    init_with(Config::new(service_name))
}

/// Install logging with an explicit [`Config`].
pub fn init_with(config: Config) -> Result<Guard, Error> {
    let filter = config.env_filter()?;

    let console = config
        .console
        .then(|| console::layer(&config.service_name, std::io::stderr, config.event_cap));

    let pipelines = otel::build(&config)?;

    // tracing spans -> OTLP spans (also supplies the trace context the log
    // bridge attaches to every record).
    let spans = pipelines
        .as_ref()
        .map(|p| tracing_opentelemetry::OpenTelemetryLayer::new(p.tracer()));

    // tracing events -> OTLP log records.
    let logs = pipelines.as_ref().map(|p| {
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&p.logger_provider)
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(console)
        .with(spans)
        .with(logs)
        .try_init()?;

    let guard = Guard { pipelines };
    tracing::debug!(
        service = %config.service_name,
        otlp = guard.pipelines.is_some(),
        "logging initialized"
    );
    Ok(guard)
}

/// The `trace_id` of the span the caller is currently inside, if OTLP
/// tracing is active. Useful for echoing correlation ids in error responses
/// or CLI output.
pub fn current_trace_id() -> Option<String> {
    use opentelemetry::trace::TraceContextExt as _;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let context = tracing::Span::current().context();
    let span = context.span();
    let span_context = span.span_context();
    span_context
        .is_valid()
        .then(|| span_context.trace_id().to_string())
}

/// Keeps the telemetry pipelines alive. Drop it (or call [`Guard::shutdown`])
/// before the process exits — ideally before the tokio runtime shuts down —
/// to flush buffered spans and log records.
#[must_use = "the guard flushes telemetry on drop; keep it alive for the service's lifetime"]
pub struct Guard {
    pipelines: Option<otel::Pipelines>,
}

impl Guard {
    /// Flush and shut the exporters down. Equivalent to dropping the guard,
    /// but explicit about when the process blocks on delivery.
    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        let Some(pipelines) = self.pipelines.take() else {
            return;
        };
        // Logs go last so a "shutting down" record still finds its trace.
        if let Err(error) = pipelines.tracer_provider.shutdown() {
            eprintln!("[xenomit-logging] tracer shutdown failed: {error:?}");
        }
        if let Err(error) = pipelines.logger_provider.shutdown() {
            eprintln!("[xenomit-logging] logger shutdown failed: {error:?}");
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}
