use std::borrow::Cow;
use std::time::Duration;

use opentelemetry::{Key, KeyValue, Value};
use tracing_subscriber::filter::EnvFilter;

use super::Error;
use super::redact::Redactor;

/// Default verbosity when neither [`Config::filter`] nor `RUST_LOG` is set.
const DEFAULT_FILTER: &str = "info";

/// Default cap on printed events per `(target, level)` pair; matches the
/// spirit of `xenomit-client`'s diagnostics, which rate-limits noisy warnings.
const DEFAULT_EVENT_CAP: usize = 64;

/// Default OTLP metrics export interval (the SDK periodic reader default).
const DEFAULT_METRIC_INTERVAL: Duration = Duration::from_secs(60);

/// How strongly OTLP export is coupled to the environment.
#[derive(Debug, Clone)]
pub enum Otlp {
    /// Export only when `OTEL_EXPORTER_OTLP_ENDPOINT` (or a signal-specific
    /// endpoint such as `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`) is set in the
    /// environment. This is the default: services keep console logging until a
    /// collector is configured, with no code change.
    Auto,

    /// Never export to a collector.
    Disabled,

    /// Always export. `endpoint` overrides `OTEL_EXPORTER_OTLP_ENDPOINT`;
    /// pass `None` to use the environment/default (`http://localhost:4317`).
    Enabled { endpoint: Option<String> },
}

#[derive(Debug)]
pub struct Config {
    /// OTLP `service.name` resource attribute and console log prefix.
    pub(crate) service_name: Cow<'static, str>,

    /// OTLP `service.version` resource attribute, when set.
    pub(crate) service_version: Option<Cow<'static, str>>,

    /// `tracing` filter directives (the `RUST_LOG` syntax). When `None`,
    /// `RUST_LOG` is honoured and defaults to `info`.
    pub(crate) filter: Option<String>,

    /// Whether to print redacted log lines to stderr.
    pub(crate) console: bool,

    /// OTLP export behaviour.
    pub(crate) otlp: Otlp,

    /// Cap on console events per `(target, level)` pair. `None` disables
    /// rate limiting.
    pub(crate) event_cap: Option<usize>,

    /// Extra OTLP resource attributes (`xenomit.bot.id`, `xenomit.tenant`,
    /// `service.instance.id`, `deployment.environment`, …). `service.name`
    /// and `service.version` are set from their dedicated fields;
    /// `OTEL_RESOURCE_ATTRIBUTES`/`OTEL_SERVICE_NAME` are honored too.
    pub(crate) resource_attrs: Vec<KeyValue>,

    /// Redaction applied on the console layer and on every OTLP signal —
    /// log bodies, log attributes, span attributes, span events and error
    /// descriptions.
    pub(crate) redactor: Redactor,

    /// Whether the OTLP metrics pipeline is built alongside traces/logs.
    pub(crate) metrics: bool,

    /// Periodic reader export interval for metrics.
    pub(crate) metric_interval: Duration,

    /// Head sampling ratio for traces; `None` keeps the SDK default
    /// (parent-based always-on). A value in `0.0..=1.0` installs a
    /// parent-based trace-id ratio sampler so a hot loop cannot flood the
    /// backend (the collector's tail sampler then applies on top).
    pub(crate) trace_ratio: Option<f64>,
}

impl Config {
    pub fn new(service_name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            service_name: service_name.into(),
            service_version: None,
            filter: None,
            console: true,
            otlp: Otlp::Auto,
            event_cap: Some(DEFAULT_EVENT_CAP),
            resource_attrs: Vec::new(),
            redactor: Redactor::default(),
            metrics: true,
            metric_interval: DEFAULT_METRIC_INTERVAL,
            trace_ratio: None,
        }
    }

    /// `service.version` resource attribute on exported telemetry.
    pub fn with_service_version(mut self, version: impl Into<Cow<'static, str>>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// Filter directives in `RUST_LOG` syntax, e.g. `"debug"` or
    /// `"info,xenomit_backpage=trace"`. Overrides `RUST_LOG` entirely.
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Disable the stderr console layer (e.g. when another writer handles
    /// human-readable output).
    pub fn without_console(mut self) -> Self {
        self.console = false;
        self
    }

    /// Change OTLP export behaviour; see [`Otlp`].
    pub fn with_otlp(mut self, otlp: Otlp) -> Self {
        self.otlp = otlp;
        self
    }

    /// Cap console output at `cap` events per `(target, level)` pair.
    pub fn with_event_cap(mut self, cap: usize) -> Self {
        self.event_cap = Some(cap);
        self
    }

    /// Print every event the filter allows through.
    pub fn without_event_cap(mut self) -> Self {
        self.event_cap = None;
        self
    }

    /// Extra OTLP resource attribute, e.g.
    /// `.with_resource_attr("xenomit.bot.id", bot_id)`,
    /// `.with_resource_attr("service.instance.id", pod_name)`.
    pub fn with_resource_attr(mut self, key: impl Into<Key>, value: impl Into<Value>) -> Self {
        self.resource_attrs.push(KeyValue::new(key, value));
        self
    }

    /// Extend the OTLP resource attribute set.
    pub fn with_resource_attrs(mut self, attrs: impl IntoIterator<Item = KeyValue>) -> Self {
        self.resource_attrs.extend(attrs);
        self
    }

    /// Replace the credential [`Redactor`] applied on the console layer and
    /// on every OTLP signal. The default covers passwords, `sid`/`dosid`
    /// session ids, tokens and `Authorization`; extend it instead of
    /// weakening it.
    pub fn with_redactor(mut self, redactor: Redactor) -> Self {
        self.redactor = redactor;
        self
    }

    /// Mark attribute keys containing `marker` (case-insensitive) as
    /// always-secret, e.g. `.with_sensitive_key("vault_key")`.
    pub fn with_sensitive_key(mut self, marker: impl Into<Cow<'static, str>>) -> Self {
        self.redactor = self.redactor.with_key_marker(marker);
        self
    }

    /// Extra word marker for free-text redaction in log bodies/messages.
    pub fn with_redaction_marker(mut self, marker: impl Into<Cow<'static, str>>) -> Self {
        self.redactor = self.redactor.with_marker(marker);
        self
    }

    /// Skip the OTLP metrics pipeline (traces/logs still export).
    pub fn without_metrics(mut self) -> Self {
        self.metrics = false;
        self
    }

    /// Periodic reader export interval for OTLP metrics (default 60 s).
    pub fn with_metric_interval(mut self, interval: Duration) -> Self {
        self.metric_interval = interval;
        self
    }

    /// Sample traces at `ratio` in `0.0..=1.0` (parent-based trace-id ratio
    /// sampler). The collector tail-sampler still applies on top — errors
    /// and slow spans stay visible.
    pub fn with_trace_ratio(mut self, ratio: f64) -> Self {
        self.trace_ratio = Some(ratio.clamp(0.0, 1.0));
        self
    }

    pub(crate) fn env_filter(&self) -> Result<EnvFilter, Error> {
        match &self.filter {
            Some(filter) => Ok(EnvFilter::try_new(filter)?),
            None => Ok(EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))),
        }
    }
}
