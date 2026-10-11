//! In-memory OTLP capture for consumer tests.
//!
//! [`capture`] installs a *scoped* `tracing` subscriber that drives the
//! exact pipeline services run — tracing spans/events → SDK providers →
//! redacting exporters — except the wire is replaced by the SDK's
//! in-memory exporters. It exists to prove claims like "a secret logged
//! inside the service never reaches the exporter" without a collector.
//!
//! ```ignore
//! let capture = logging::testing::capture();
//! logging::tracing::warn!(dosid = "s3cr3t", "login failed");
//! let exported = format!("{:?}", capture.emitted_logs());
//! assert!(!exported.contains("s3cr3t"));
//! ```

use opentelemetry::trace::TracerProvider as _;
pub use opentelemetry_sdk::logs::InMemoryLogExporter;
pub use opentelemetry_sdk::logs::in_memory_exporter::LogDataWithResource;
use opentelemetry_sdk::logs::{SdkLoggerProvider, SimpleLogProcessor};
pub use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use super::otel::{RedactingLogExporter, RedactingSpanExporter};
use super::redact::Redactor;

/// A scoped redacting pipeline backed by in-memory exporters. Restores
/// the previous default subscriber and shuts the providers down on drop —
/// declare order matters: the subscriber guard drops first so late events
/// cannot hit a dead provider.
pub struct Capture {
    _subscriber: tracing::dispatcher::DefaultGuard,
    spans: InMemorySpanExporter,
    logs: InMemoryLogExporter,
    _tracer_provider: SdkTracerProvider,
    _logger_provider: SdkLoggerProvider,
}

/// Capture telemetry through the default [`Redactor`].
pub fn capture() -> Capture {
    capture_with(Redactor::default())
}

/// Capture telemetry through a caller-supplied [`Redactor`].
pub fn capture_with(redactor: Redactor) -> Capture {
    let spans = InMemorySpanExporter::default();
    let logs = InMemoryLogExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(RedactingSpanExporter::new(
            spans.clone(),
            redactor.clone(),
        ))
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_log_processor(SimpleLogProcessor::new(RedactingLogExporter::new(
            logs.clone(),
            redactor,
        )))
        .build();
    let spans_layer =
        tracing_opentelemetry::OpenTelemetryLayer::new(tracer_provider.tracer("xenomit-test"));
    let logs_layer =
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&logger_provider);
    let subscriber = tracing_subscriber::registry()
        .with(spans_layer)
        .with(logs_layer)
        .set_default();
    Capture {
        _subscriber: subscriber,
        spans,
        logs,
        _tracer_provider: tracer_provider,
        _logger_provider: logger_provider,
    }
}

impl Capture {
    /// Spans as the exporter saw them — already redacted.
    pub fn finished_spans(&self) -> Vec<SpanData> {
        self.spans.get_finished_spans().unwrap_or_default()
    }

    /// Log records as the exporter saw them — already redacted.
    pub fn emitted_logs(&self) -> Vec<LogDataWithResource> {
        self.logs.get_emitted_logs().unwrap_or_default()
    }
}
