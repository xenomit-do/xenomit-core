//! OTLP path inside a tokio runtime. With no collector reachable the gRPC
//! channel connects lazily, so init succeeds and shutdown simply reports the
//! undeliverable batches.

use xenomit_logging::{Config, Otlp};

#[tokio::test(flavor = "multi_thread")]
async fn otlp_pipelines_install_inside_runtime() {
    let guard = xenomit_logging::init_with(
        Config::new("xenomit-test-otlp").with_otlp(Otlp::Enabled { endpoint: None }),
    )
    .expect("OTLP init succeeds inside tokio");

    xenomit_logging::tracing::info_span!("op").in_scope(|| {
        xenomit_logging::tracing::error!("boom");
        assert!(xenomit_logging::current_trace_id().is_some());
    });

    guard.shutdown();
}
