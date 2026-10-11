//! OTLP metrics helpers (docs/06 §6.6).
//!
//! Instruments built here are inert no-ops until [`init_with`] installs a
//! `MeterProvider` — which only happens when OTLP export is enabled — so
//! services may create and use them unconditionally. Create instruments
//! *after* init: handles bind to the provider that is global at
//! construction time.
//!
//! [`init_with`]: crate::logging::init_with

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

/// The shared meter for xenomit services.
pub fn meter() -> Meter {
    opentelemetry::global::meter("xenomit")
}

/// Monotonic `u64` counter — `add(1, &labels)` on each occurrence.
/// Keep labels bounded (docs/06 §6.3: no `bot.id` on metrics).
pub fn counter(name: &'static str, description: &'static str) -> Counter<u64> {
    meter()
        .u64_counter(name)
        .with_description(description)
        .build()
}

/// `f64` histogram; `unit` follows UCUM (`"s"` seconds, `"By"` bytes).
pub fn histogram(name: &'static str, description: &'static str, unit: &'static str) -> Histogram<f64> {
    meter()
        .f64_histogram(name)
        .with_description(description)
        .with_unit(unit)
        .build()
}

/// `u64` gauge recording the latest value via `record`.
pub fn gauge(name: &'static str, description: &'static str) -> Gauge<u64> {
    meter()
        .u64_gauge(name)
        .with_description(description)
        .build()
}
