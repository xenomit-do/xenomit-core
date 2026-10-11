use std::borrow::Cow;
use std::env;
use std::time::Duration;

use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, LoggerProvider as _, Severity};
use opentelemetry::trace::{Status, TracerProvider as _};
use opentelemetry::{Array, KeyValue, Value};
use opentelemetry_otlp::{MetricExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord, SdkLoggerProvider};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{Sampler, SpanData, SpanExporter, SdkTracer, SdkTracerProvider};

use super::Error;
use super::config::{Config, Otlp};
use super::redact::{REDACTED, Redactor};

/// Environment variables that turn on [`Otlp::Auto`] export.
const ENDPOINT_VARS: &[&str] = &[
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
];

/// The tracer, logger and meter providers. Dropping the last clone shuts a
/// pipeline down, but [`crate::logging::Guard`] keeps explicit handles so
/// buffered telemetry can be flushed deterministically.
pub(crate) struct Pipelines {
    pub(crate) tracer_provider: SdkTracerProvider,
    pub(crate) logger_provider: SdkLoggerProvider,
    /// `None` when [`Config::metrics`] is off; export stays on traces/logs.
    pub(crate) meter_provider: Option<SdkMeterProvider>,
}

impl Pipelines {
    pub(crate) fn tracer(&self) -> SdkTracer {
        self.tracer_provider.tracer("xenomit")
    }
}

/// Build OTLP exporters and SDK providers when the configuration calls for
/// them. Returns `None` when export is off and telemetry stays a no-op.
pub(crate) fn build(config: &Config) -> Result<Option<Pipelines>, Error> {
    let endpoint = match &config.otlp {
        Otlp::Disabled => return Ok(None),
        Otlp::Enabled { endpoint } => endpoint.clone(),
        Otlp::Auto => {
            if !auto_enabled() {
                return Ok(None);
            }
            None
        }
    };

    // The batch processors and the tonic transport spawn onto the current
    // runtime; without one the SDK would panic deep inside `build`.
    if tokio::runtime::Handle::try_current().is_err() {
        eprintln!("[xenomit-logging] OTLP export skipped: no tokio runtime is running");
        return Ok(None);
    }

    // `Resource::builder` already detects `OTEL_RESOURCE_ATTRIBUTES` and
    // `OTEL_SERVICE_NAME`; explicit config attributes win on conflicts.
    let resource = {
        let mut builder = Resource::builder().with_service_name(config.service_name.clone());
        if let Some(version) = &config.service_version {
            builder = builder.with_attribute(KeyValue::new("service.version", version.clone()));
        }
        builder
            .with_attributes(config.resource_attrs.iter().cloned())
            .build()
    };

    let mut span_exporter = opentelemetry_otlp::SpanExporter::builder().with_tonic();
    let mut log_exporter = opentelemetry_otlp::LogExporter::builder().with_tonic();
    let mut metric_exporter = config
        .metrics
        .then(|| MetricExporter::builder().with_tonic());
    if let Some(endpoint) = endpoint {
        span_exporter = span_exporter.with_endpoint(endpoint.clone());
        log_exporter = log_exporter.with_endpoint(endpoint.clone());
        metric_exporter = metric_exporter.map(|builder| builder.with_endpoint(endpoint));
    }

    // docs/06 §6.5: a session id in a log line must never reach the
    // backend. Every exported span and log record passes through the
    // redactor first.
    let span_exporter = RedactingSpanExporter::new(span_exporter.build()?, config.redactor.clone());
    let log_exporter = RedactingLogExporter::new(log_exporter.build()?, config.redactor.clone());

    let mut tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter);
    if let Some(ratio) = config.trace_ratio {
        tracer_provider = tracer_provider.with_sampler(Sampler::ParentBased(Box::new(
            Sampler::TraceIdRatioBased(ratio),
        )));
    }
    let tracer_provider = tracer_provider.build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(log_exporter)
        .build();

    let meter_provider = match metric_exporter {
        Some(builder) => {
            let reader = PeriodicReader::builder(builder.build()?)
                .with_interval(config.metric_interval)
                .build();
            let provider = SdkMeterProvider::builder()
                .with_resource(resource)
                .with_reader(reader)
                .build();
            // Global lookup keeps instrumentation call sites free of
            // plumbing (`logging::metrics` reads it back).
            opentelemetry::global::set_meter_provider(provider.clone());
            Some(provider)
        }
        None => None,
    };

    // Make the provider reachable for instrumentation that looks up the
    // global (e.g. middleware).
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    Ok(Some(Pipelines {
        tracer_provider,
        logger_provider,
        meter_provider,
    }))
}

/// `Otlp::Auto` resolves to on when a collector endpoint is configured.
/// `OTEL_SDK_DISABLED=true` always wins, matching the OTel spec.
fn auto_enabled() -> bool {
    let disabled = env::var("OTEL_SDK_DISABLED")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if disabled {
        return false;
    }
    ENDPOINT_VARS.iter().any(|var| {
        env::var(var)
            .map(|value| !value.is_empty())
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------
// Redaction on the export path (docs/06 §6.2/§6.5)
// ---------------------------------------------------------------------

/// A [`SpanExporter`] that scrubs names, attributes, span events, links
/// and error descriptions through a [`Redactor`] before handing the batch
/// on. Nothing the wrapped exporter sees can contain a secret.
#[derive(Debug)]
pub struct RedactingSpanExporter<E> {
    inner: E,
    redactor: Redactor,
}

impl<E> RedactingSpanExporter<E> {
    pub fn new(inner: E, redactor: Redactor) -> Self {
        Self { inner, redactor }
    }

    /// The wrapped exporter.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: SpanExporter> SpanExporter for RedactingSpanExporter<E> {
    fn export(
        &self,
        mut batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        for span in &mut batch {
            self.redactor.redact_span(span);
        }
        self.inner.export(batch)
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// A [`LogExporter`] that scrubs record bodies and attributes through a
/// [`Redactor`] before handing the batch on. Because the SDK offers no way
/// to strip attributes in place, records that reach `inner` are rebuilt
/// copies carrying only scrubbed data.
#[derive(Debug)]
pub struct RedactingLogExporter<E> {
    inner: E,
    redactor: Redactor,
    /// Pristine record cloned per rebuild (see [`Redactor::redact_record`]).
    blank: SdkLogRecord,
}

impl<E> RedactingLogExporter<E> {
    pub fn new(inner: E, redactor: Redactor) -> Self {
        Self {
            inner,
            redactor,
            blank: blank_record(),
        }
    }

    /// The wrapped exporter.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: LogExporter> LogExporter for RedactingLogExporter<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let mut clean = Vec::new();
        for (record, _) in batch.iter() {
            clean.push(self.redactor.redact_record(&self.blank, record));
        }
        let mut entries = Vec::with_capacity(clean.len());
        for (record, (_, scope)) in clean.iter().zip(batch.iter()) {
            entries.push((record, scope));
        }
        self.inner.export(LogBatch::new(&entries)).await
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
        self.inner.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// Fresh records exist only behind `Logger::create_log_record`; a
/// throwaway provider produces the pristine record clones are built from.
fn blank_record() -> SdkLogRecord {
    SdkLoggerProvider::builder()
        .build()
        .logger("xenomit.redact")
        .create_log_record()
}

impl Redactor {
    /// Scrub everything on a finished span that could carry a secret:
    /// attributes, span-event names/attributes, link attributes and the
    /// error description. Names are treated as free text.
    pub(crate) fn redact_span(&self, span: &mut SpanData) {
        span.name = self.redact_cow(std::mem::take(&mut span.name));
        for attribute in &mut span.attributes {
            self.redact_key_value(attribute);
        }
        for event in &mut span.events.events {
            event.name = self.redact_cow(std::mem::take(&mut event.name));
            for attribute in &mut event.attributes {
                self.redact_key_value(attribute);
            }
        }
        for link in &mut span.links.links {
            for attribute in &mut link.attributes {
                self.redact_key_value(attribute);
            }
        }
        if let Status::Error { description } = &mut span.status {
            *description = self.redact_cow(std::mem::take(description));
        }
    }

    /// Scrub a span/log attribute: sensitive keys are masked in full,
    /// string values pass through text redaction.
    pub(crate) fn redact_key_value(&self, attribute: &mut KeyValue) {
        attribute.value = if self.is_sensitive_key(attribute.key.as_str()) {
            Value::String(REDACTED.into())
        } else {
            self.redact_value(&attribute.value)
        };
    }

    /// Copy `record` into a blank record with every field scrubbed — the
    /// SDK offers no way to remove attributes in place, so the record is
    /// rebuilt field by field via the [`LogRecord`] trait.
    pub(crate) fn redact_record(
        &self,
        blank: &SdkLogRecord,
        record: &SdkLogRecord,
    ) -> SdkLogRecord {
        let mut clean = blank.clone();
        if let Some(name) = record.event_name() {
            clean.set_event_name(name);
        }
        if let Some(target) = record.target() {
            clean.set_target(target.to_string());
        }
        if let Some(timestamp) = record.timestamp() {
            clean.set_timestamp(timestamp);
        }
        if let Some(timestamp) = record.observed_timestamp() {
            clean.set_observed_timestamp(timestamp);
        }
        if let Some(text) = record.severity_text() {
            clean.set_severity_text(text);
        }
        if let Some(number) = record.severity_number() {
            clean.set_severity_number(number);
        }
        if let Some(context) = record.trace_context() {
            clean.set_trace_context(context.trace_id, context.span_id, context.trace_flags);
        }
        if let Some(body) = record.body() {
            clean.set_body(self.redact_any(None, body));
        }
        for (key, value) in record.attributes_iter() {
            clean.add_attribute(key.clone(), self.redact_any(Some(key.as_str()), value));
        }
        clean
    }

    /// Log-record `AnyValue` scrub: maps and lists are walked recursively
    /// so a nested `password` key is still masked.
    fn redact_any(&self, key: Option<&str>, value: &AnyValue) -> AnyValue {
        if key.is_some_and(|key| self.is_sensitive_key(key)) {
            return AnyValue::String(REDACTED.into());
        }
        match value {
            AnyValue::String(text) => AnyValue::String(self.text(text.as_str()).into()),
            AnyValue::ListAny(list) => AnyValue::ListAny(Box::new(
                list.iter().map(|item| self.redact_any(None, item)).collect(),
            )),
            AnyValue::Map(map) => AnyValue::Map(Box::new(
                map.iter()
                    .map(|(key, value)| {
                        (key.clone(), self.redact_any(Some(key.as_str()), value))
                    })
                    .collect(),
            )),
            other => other.clone(),
        }
    }

    /// Span-attribute `Value` scrub.
    fn redact_value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(text.as_str()).into()),
            Value::Array(Array::String(items)) => Value::Array(Array::String(
                items.iter().map(|item| self.text(item.as_str()).into()).collect(),
            )),
            other => other.clone(),
        }
    }

    fn redact_cow(&self, value: Cow<'static, str>) -> Cow<'static, str> {
        let redacted = self.text(&value);
        if redacted == value {
            value
        } else {
            Cow::Owned(redacted)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use opentelemetry::InstrumentationScope;
    use opentelemetry::trace::{Event, SpanContext, SpanId, SpanKind};
    use opentelemetry_sdk::logs::InMemoryLogExporter;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanEvents, SpanLinks};

    use super::*;

    const SECRET: &str = "s3cr3t-dosid";

    fn span_carrying_secrets() -> SpanData {
        let mut events = SpanEvents::default();
        events.events.push(Event::new(
            "retrying https://internal/?sid=s3cr3t-dosid",
            SystemTime::now(),
            vec![
                KeyValue::new("password", "hunter2"),
                KeyValue::new("note", "still failing"),
            ],
            0,
        ));
        SpanData {
            span_context: SpanContext::empty_context(),
            parent_span_id: SpanId::INVALID,
            parent_span_is_remote: false,
            span_kind: SpanKind::Internal,
            name: "bot.login https://internal/?sid=s3cr3t-dosid".into(),
            start_time: SystemTime::now(),
            end_time: SystemTime::now(),
            attributes: vec![
                KeyValue::new("dosid", SECRET),
                KeyValue::new("url", "https://int1.darkorbit.com/?sid=s3cr3t-dosid"),
                KeyValue::new("attempt", 1i64),
            ],
            dropped_attributes_count: 0,
            events,
            links: SpanLinks::default(),
            status: Status::Error {
                description: "auth failed dosid=s3cr3t-dosid".into(),
            },
            instrumentation_scope: InstrumentationScope::builder("test").build(),
        }
    }

    /// docs/06 §6.2: a secret in a span attribute, event or error
    /// description must never reach the exporter.
    #[tokio::test]
    async fn span_exporter_never_forwards_secrets() {
        let memory = InMemorySpanExporter::default();
        let exporter = RedactingSpanExporter::new(memory.clone(), Redactor::default());

        exporter.export(vec![span_carrying_secrets()]).await.unwrap();

        let exported = memory.get_finished_spans().unwrap();
        let rendered = format!("{exported:?}");
        assert!(!rendered.contains(SECRET), "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains(REDACTED), "{rendered}");

        // Structure survives: insensitive values and fields pass through.
        let span = &exported[0];
        assert!(span.name.starts_with("bot.login"));
        assert!(span.attributes.iter().any(|kv| kv.key.as_str() == "attempt"
            && kv.value == Value::I64(1)));
        assert!(span.attributes.iter().any(|kv| kv.key.as_str() == "dosid"
            && kv.value == Value::String(REDACTED.into())));
        assert_eq!(span.events[0].attributes[1].value.as_str(), "still failing");
    }

    /// docs/06 §6.2: a secret in a log line — body or attribute — must
    /// never reach the exporter.
    #[tokio::test]
    async fn log_exporter_never_forwards_secrets() {
        let memory = InMemoryLogExporter::default();
        let exporter = RedactingLogExporter::new(memory.clone(), Redactor::default());

        let provider = SdkLoggerProvider::builder().build();
        let logger = provider.logger("test");
        let mut record = logger.create_log_record();
        record.set_body(AnyValue::String(
            format!("login failed for dosid={SECRET} at https://internal/?sid={SECRET}").into(),
        ));
        record.add_attribute("dosid", SECRET);
        record.add_attribute("password", "hunter2");
        record.add_attribute("attempt", 1i64);
        record.set_severity_number(Severity::Warn);
        let scope = InstrumentationScope::builder("test").build();

        exporter.export(LogBatch::new(&[(&record, &scope)])).await.unwrap();

        let emitted = memory.get_emitted_logs().unwrap();
        let rendered = format!("{emitted:?}");
        assert!(!rendered.contains(SECRET), "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains(REDACTED), "{rendered}");

        let record = &emitted[0].record;
        assert_eq!(record.severity_number(), Some(Severity::Warn));
        assert!(record.attributes_iter().any(|(key, value)| {
            key.as_str() == "attempt" && *value == AnyValue::Int(1)
        }));
        assert!(record.attributes_iter().any(|(key, value)| {
            key.as_str() == "dosid" && *value == AnyValue::String(REDACTED.into())
        }));
    }
}
