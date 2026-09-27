//! xenomit-core: shared functionality for xenomit services.
//!
//! * [`logging`] — `tracing` instrumentation wired to a redacting console
//!   layer and OTLP (OpenTelemetry) export.

pub mod logging;
