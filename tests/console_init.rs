//! `init` installs a process-global subscriber, so every test binary may
//! only initialise once — this file covers the console-only path.

use xenomit_core::logging::{self, Config, Otlp};

#[test]
fn installs_console_logging_and_rejects_second_init() {
    let guard = logging::init_with(
        Config::new("xenomit-test")
            .with_filter("debug")
            .with_otlp(Otlp::Disabled),
    )
    .expect("first init succeeds");

    // A second install must fail rather than silently replace the pipeline.
    assert!(logging::init("other-service").is_err());

    logging::tracing::info!("event after init");

    // No OTLP means no trace context to report.
    assert_eq!(logging::current_trace_id(), None);

    guard.shutdown();
}
