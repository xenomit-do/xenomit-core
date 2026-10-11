//! W3C `traceparent`/`tracestate` inject/extract helpers (docs/06 §6.2).
//!
//! These are the seams later control-plane hops (gRPC metadata, panel
//! WebSocket envelopes) use to keep one trace across processes. The
//! propagator is installed by [`init_with`]; before init — or without a
//! valid active span — every helper is a safe no-op.
//!
//! [`init_with`]: crate::logging::init_with

use opentelemetry::Context;
use opentelemetry::propagation::{Extractor, Injector};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Inject the *current tracing span's* context into `carrier`. Any
/// [`Injector`] works — `HashMap<String, String>` implements it out of the
/// box, and thin adapters cover gRPC metadata maps.
///
/// No-op when there is no valid active span, so callers may invoke it
/// unconditionally.
pub fn inject(carrier: &mut dyn Injector) {
    inject_context(&tracing::Span::current().context(), carrier);
}

/// Inject an explicit [`Context`] into `carrier` via the global
/// propagator.
pub fn inject_context(context: &Context, carrier: &mut dyn Injector) {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(context, carrier);
    });
}

/// Extract a remote [`Context`] from `carrier` (`traceparent`,
/// `tracestate`). Attach it as the handling span's parent:
///
/// ```ignore
/// use tracing_opentelemetry::OpenTelemetrySpanExt;
///
/// let context = propagation::extract(&headers);
/// let span = tracing::info_span!("handle");
/// let _ = span.set_parent(context);
/// ```
pub fn extract(carrier: &dyn Extractor) -> Context {
    opentelemetry::global::get_text_map_propagator(|propagator| propagator.extract(carrier))
}
