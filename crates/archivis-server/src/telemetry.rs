use tracing_subscriber::EnvFilter;

/// Initialize the tracing subscriber with structured logging.
///
/// If the `RUST_LOG` environment variable is set, it takes precedence over the
/// configured `log_level`. This lets developers override logging for debugging
/// without changing the application config.
pub fn init_logging(log_level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{log_level},lopdf=error")));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
