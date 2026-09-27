//! xenomit-core: shared functionality for xenomit services.
//!
//! This crate is a facade: each concern lives in its own `xenomit_*` crate
//! and is re-exported here as a module, so services can depend on a single
//! `xenomit_core` package and import `xenomit_core::<area>::*` paths.
//!
//! * [`logging`] — `tracing` instrumentation wired to a redacting console
//!   layer and OTLP (OpenTelemetry) export.
//!
//! ```no_run
//! use xenomit_core::logging;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let _guard = logging::init("xenomit-client")?;
//!     logging::tracing::info!("started");
//!     Ok(())
//! }
//! ```

/// Shared logging setup: `tracing` facade routed to a redacting console
/// layer and OTLP export.
///
/// Re-exported from [`xenomit_logging`]; see that crate's documentation for
/// the full API ([`logging::init`], [`logging::Config`], [`logging::Otlp`],
/// [`logging::Guard`], [`logging::current_trace_id`], and the [`tracing`]
/// facade at [`logging::tracing`]).
pub use xenomit_logging as logging;
