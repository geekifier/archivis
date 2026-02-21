mod config;
mod telemetry;

use clap::Parser;
use config::{AppConfig, Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let config = match AppConfig::load(&cli) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Failed to load configuration: {err}");
            std::process::exit(1);
        }
    };

    telemetry::init_logging(&config.log_level);

    tracing::info!(
        listen = %config.bind_address(),
        data_dir = %config.data_dir.display(),
        book_storage_path = %config.book_storage_path.display(),
        "Archivis starting"
    );

    if let Err(err) = config.ensure_directories() {
        tracing::error!(%err, "Failed to create required directories");
        std::process::exit(1);
    }

    // TODO: Initialize database, services, and HTTP server

    tracing::info!("Archivis ready — press Ctrl+C to stop");

    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!("Shutdown signal received"),
        Err(err) => tracing::error!(%err, "Failed to listen for shutdown signal"),
    }

    tracing::info!("Archivis stopped");
}
