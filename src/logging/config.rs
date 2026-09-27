use std::borrow::Cow;

use tracing_subscriber::filter::EnvFilter;

use super::Error;

/// Default verbosity when neither [`Config::filter`] nor `RUST_LOG` is set.
const DEFAULT_FILTER: &str = "info";

/// Default cap on printed events per `(target, level)` pair; matches the
/// spirit of `xenomit-client`'s diagnostics, which rate-limits noisy warnings.
const DEFAULT_EVENT_CAP: usize = 64;

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

    pub(crate) fn env_filter(&self) -> Result<EnvFilter, Error> {
        match &self.filter {
            Some(filter) => Ok(EnvFilter::try_new(filter)?),
            None => Ok(EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))),
        }
    }
}
