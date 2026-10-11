//! `Otlp::Enabled` without a tokio runtime must degrade to console logging
//! instead of panicking inside the SDK's batch processors.

use xenomit_core::logging::{self, Config, Otlp};

#[test]
fn otlp_enabled_without_runtime_degrades() {
    let guard = logging::init_with(
        Config::new("xenomit-test-no-rt").with_otlp(Otlp::Enabled { endpoint: None }),
    )
    .expect("init degrades gracefully without a runtime");

    logging::tracing::warn!("console only");
    guard.shutdown();
}
