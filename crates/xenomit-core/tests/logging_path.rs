//! The `xenomit_core::logging` re-export exposes the full logging API.

use xenomit_core::logging::{self, Config, Otlp};

#[test]
fn facade_paths_resolve() {
    let config = Config::new("svc")
        .with_service_version("0.1.0")
        .with_filter("debug")
        .with_event_cap(8)
        .with_otlp(Otlp::Disabled);
    drop(config);

    // `tracing` is re-exported through the facade as well; without an
    // installed subscriber this is a no-op.
    logging::tracing::info!("probe");
    assert_eq!(logging::current_trace_id(), None);
}
