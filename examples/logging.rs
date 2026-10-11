//! The facade import path: `use xenomit_core::logging`.
//!
//! `cargo run -p xenomit_core --example logging`

use xenomit_core::logging;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = logging::init_with(
        logging::Config::new("xenomit-example")
            .with_service_version(env!("CARGO_PKG_VERSION"))
            .with_otlp(logging::Otlp::Auto),
    )?;

    logging::tracing::info!(answer = 42, "hello via xenomit_core::logging");
    Ok(())
}
