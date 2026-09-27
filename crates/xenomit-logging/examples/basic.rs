//! Console + OTLP logging demo.
//!
//! `cargo run -p xenomit_logging --example basic` prints redacted log lines on
//! stderr. Point `OTEL_EXPORTER_OTLP_ENDPOINT` at a collector (e.g.
//! `http://localhost:4317`) and the same events arrive as OTLP logs, with
//! `work` exported as an OTLP span the records correlate to.

use xenomit_logging as logging;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = logging::init("xenomit-example")?;

    logging::tracing::info!(answer = 42, "hello from xenomit-core");

    let span = logging::tracing::info_span!("work", id = 7);
    let _entered = span.enter();
    logging::tracing::warn!(
        url = "https://example.com/secret",
        "fetch failed (url redacted on console, kept in OTLP attributes)"
    );
    if let Some(trace_id) = logging::current_trace_id() {
        logging::tracing::info!(%trace_id, "inside span, trace context active");
    }

    Ok(())
}
