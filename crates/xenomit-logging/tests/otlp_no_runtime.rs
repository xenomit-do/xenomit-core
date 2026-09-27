//! `Otlp::Enabled` without a tokio runtime must degrade to console logging
//! instead of panicking inside the SDK's batch processors.

use xenomit_logging::{Config, Otlp};

#[test]
fn otlp_enabled_without_runtime_degrades() {
    let guard = xenomit_logging::init_with(
        Config::new("xenomit-test-no-rt").with_otlp(Otlp::Enabled { endpoint: None }),
    )
    .expect("init degrades gracefully without a runtime");

    xenomit_logging::tracing::warn!("console only");
    guard.shutdown();
}
