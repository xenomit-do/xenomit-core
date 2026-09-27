//! `init` installs a process-global subscriber, so every test binary may
//! only initialise once — this file covers the console-only path.

use xenomit_logging::{Config, Otlp};

#[test]
fn installs_console_logging_and_rejects_second_init() {
    let guard = xenomit_logging::init_with(
        Config::new("xenomit-test")
            .with_filter("debug")
            .with_otlp(Otlp::Disabled),
    )
    .expect("first init succeeds");

    // A second install must fail rather than silently replace the pipeline.
    assert!(xenomit_logging::init("other-service").is_err());

    xenomit_logging::tracing::info!("event after init");

    // No OTLP means no trace context to report.
    assert_eq!(xenomit_logging::current_trace_id(), None);

    guard.shutdown();
}
