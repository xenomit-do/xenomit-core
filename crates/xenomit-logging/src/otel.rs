use std::env;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};

use crate::Error;
use crate::config::{Config, Otlp};

/// Environment variables that turn on [`Otlp::Auto`] export.
const ENDPOINT_VARS: &[&str] = &[
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
];

/// The tracer and logger providers. Dropping the last clone shuts the
/// pipeline down, but [`crate::Guard`] keeps explicit handles so buffered
/// telemetry can be flushed deterministically.
pub(crate) struct Pipelines {
    pub(crate) tracer_provider: SdkTracerProvider,
    pub(crate) logger_provider: SdkLoggerProvider,
}

impl Pipelines {
    pub(crate) fn tracer(&self) -> SdkTracer {
        self.tracer_provider.tracer("xenomit")
    }
}

/// Build OTLP exporters and SDK providers when the configuration calls for
/// them. Returns `None` when export is off and logging stays local.
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

    let resource = {
        let mut builder = Resource::builder().with_service_name(config.service_name.clone());
        if let Some(version) = &config.service_version {
            builder = builder.with_attribute(opentelemetry::KeyValue::new(
                "service.version",
                version.clone(),
            ));
        }
        builder.build()
    };

    let mut span_exporter = SpanExporter::builder().with_tonic();
    let mut log_exporter = LogExporter::builder().with_tonic();
    if let Some(endpoint) = endpoint {
        span_exporter = span_exporter.with_endpoint(endpoint.clone());
        log_exporter = log_exporter.with_endpoint(endpoint);
    }
    let span_exporter = span_exporter.build()?;
    let log_exporter = log_exporter.build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter)
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(log_exporter)
        .build();

    // Make the provider reachable for instrumentation that looks up the
    // global (e.g. middleware), and advertise W3C trace context propagation
    // so services can join incoming `traceparent` headers.
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    Ok(Some(Pipelines {
        tracer_provider,
        logger_provider,
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
