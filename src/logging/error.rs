use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid log filter directive: {0}")]
    Filter(#[from] tracing_subscriber::filter::ParseError),

    #[error("a tracing subscriber is already installed: {0}")]
    AlreadyInstalled(#[from] tracing_subscriber::util::TryInitError),

    #[error("failed to build OTLP exporter: {0}")]
    Otlp(#[from] opentelemetry_otlp::ExporterBuildError),
}
