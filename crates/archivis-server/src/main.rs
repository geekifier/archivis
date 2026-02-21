mod config;
mod telemetry;

use std::collections::HashMap;
use std::sync::Arc;

use archivis_api::state::{ApiConfig, AppState};
use archivis_auth::{AuthService, LocalAuthAdapter};
use archivis_storage::local::LocalStorage;
use archivis_tasks::import::{BulkImportService, ImportConfig, ImportService};
use archivis_tasks::queue::{self, TaskQueue, Worker};
use archivis_tasks::workers::{ImportDirectoryWorker, ImportFileWorker};
use clap::Parser;
use config::{AppConfig, Cli};
use tokio::net::TcpListener;

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

    // 1. Database
    let db_path = config.data_dir.join("archivis.db");
    let db_pool = match archivis_db::create_pool(&db_path).await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(%err, "Failed to create database pool");
            std::process::exit(1);
        }
    };

    if let Err(err) = archivis_db::run_migrations(&db_pool).await {
        tracing::error!(%err, "Failed to run database migrations");
        std::process::exit(1);
    }

    // 2. Storage
    let storage = match LocalStorage::new(&config.book_storage_path).await {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(%err, "Failed to initialize storage backend");
            std::process::exit(1);
        }
    };

    // 3. Auth
    let auth_adapter = LocalAuthAdapter::new(db_pool.clone());
    let auth_service = AuthService::new(db_pool.clone(), auth_adapter);

    // 4. Task queue + workers
    let (task_queue, dispatch_rx) = TaskQueue::new(db_pool.clone());
    let task_queue = Arc::new(task_queue);

    let import_config = ImportConfig {
        data_dir: config.data_dir.clone(),
        ..ImportConfig::default()
    };
    let import_service = Arc::new(ImportService::new(
        db_pool.clone(),
        storage.clone(),
        import_config,
    ));
    let bulk_import_service = Arc::new(BulkImportService::new(ImportService::new(
        db_pool.clone(),
        storage.clone(),
        ImportConfig {
            data_dir: config.data_dir.clone(),
            ..ImportConfig::default()
        },
    )));

    let mut workers: HashMap<archivis_core::models::TaskType, Arc<dyn Worker>> = HashMap::new();
    workers.insert(
        archivis_core::models::TaskType::ImportFile,
        Arc::new(ImportFileWorker::new(Arc::clone(&import_service))),
    );
    workers.insert(
        archivis_core::models::TaskType::ImportDirectory,
        Arc::new(ImportDirectoryWorker::new(Arc::clone(&bulk_import_service))),
    );

    let progress = task_queue.progress_sender();
    let dispatcher_pool = db_pool.clone();
    tokio::spawn(async move {
        queue::run_dispatcher(dispatch_rx, workers, progress, dispatcher_pool).await;
    });

    // Recover interrupted tasks from previous run
    if let Err(err) = queue::recover_tasks(&db_pool, &task_queue.dispatch_sender()).await {
        tracing::warn!(%err, "Failed to recover interrupted tasks");
    }

    // 5. Build application state and router
    let api_config = ApiConfig {
        data_dir: config.data_dir.clone(),
    };
    let state = AppState::new(db_pool, task_queue, auth_service, storage, api_config);
    let router = archivis_api::build_router(state);

    // 6. Bind and serve
    let bind_addr = config.bind_address();
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(%err, addr = %bind_addr, "Failed to bind TCP listener");
            std::process::exit(1);
        }
    };

    tracing::info!(addr = %bind_addr, "Archivis ready — listening for connections");

    let server = axum::serve(listener, router).with_graceful_shutdown(shutdown_signal());

    if let Err(err) = server.await {
        tracing::error!(%err, "Server error");
        std::process::exit(1);
    }

    tracing::info!("Archivis stopped");
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    tracing::info!("Shutdown signal received");
}
