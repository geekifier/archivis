mod config;
mod telemetry;

use std::collections::HashMap;
use std::sync::Arc;

use archivis_api::state::{ApiConfig, AppState};
use archivis_auth::{AuthService, LocalAuthAdapter};
use archivis_metadata::{
    HardcoverProvider, MetadataHttpClient, MetadataResolver, OpenLibraryProvider, ProviderRegistry,
};
use archivis_storage::local::LocalStorage;
use archivis_storage::watcher::{service::WatcherRuntimeConfig, WatcherService};
use archivis_tasks::identify::IdentificationService;
use archivis_tasks::import::{BulkImportService, ImportConfig, ImportService};
use archivis_tasks::isbn_scan::{IsbnScanConfig as TaskIsbnScanConfig, IsbnScanService};
use archivis_tasks::merge::MergeService;
use archivis_tasks::queue::{self, TaskQueue, Worker};
use archivis_tasks::workers::{
    watcher_processor, IdentifyWorker, ImportDirectoryWorker, ImportFileWorker, IsbnScanWorker,
};
use clap::Parser;
use config::{AppConfig, Cli};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let mut config = match AppConfig::load(&cli) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Failed to load configuration: {err}");
            std::process::exit(1);
        }
    };

    telemetry::init_logging(&config.log_level);

    let frontend_display = config.frontend_dir.as_deref().map(std::path::Path::display);
    tracing::info!(
        listen = %config.bind_address(),
        data_dir = %config.data_dir.display(),
        book_storage_path = %config.book_storage_path.display(),
        frontend_dir = ?frontend_display,
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

    // 1a. Settings: load DB overrides and build ConfigService
    let config_service = init_config_service(&cli, &mut config, &db_pool).await;

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

    // 4. Metadata providers
    let provider_registry = init_metadata_providers(&config.metadata);

    // 5. Task queue, workers, and services
    let (router, watcher_service) = init_services_and_router(
        db_pool,
        storage,
        auth_service,
        provider_registry,
        &config,
        config_service,
    )
    .await;

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

    // Graceful shutdown: stop filesystem watchers before exiting.
    if let Some(ws) = &watcher_service {
        ws.read().await.shutdown().await;
    }

    tracing::info!("Archivis stopped");
}

/// Load settings from the database, apply them to the effective config, and build
/// the `ConfigService` that powers the admin settings API.
async fn init_config_service(
    cli: &Cli,
    config: &mut AppConfig,
    db_pool: &archivis_db::DbPool,
) -> Arc<archivis_api::settings::service::ConfigService> {
    let db_settings = archivis_db::SettingRepository::get_all(db_pool)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(%err, "Failed to load settings from database");
            Vec::new()
        });
    let db_keys: Vec<String> = db_settings.iter().map(|(k, _)| k.clone()).collect();

    // Load file-only config once (defaults + TOML, no env/CLI).
    // Used for bootstrap source detection and as the base for "configured" values.
    let config_path = cli.config.to_str().unwrap_or("config.toml");
    let file_cli = Cli::parse_from::<[&str; 3], &str>(["archivis", "--config", config_path]);
    let file_config = AppConfig::load(&file_cli).unwrap_or_default();
    let file_flat = config::flatten_config(&file_config);

    // Build "configured" config: file-loaded base (bootstrap) + DB overlay (runtime)
    let configured_config = {
        let mut base = file_config;
        config::apply_db_settings(&mut base, &db_settings);
        base
    };

    // Apply DB settings to the effective config (which already has env/CLI)
    config::apply_db_settings(config, &db_settings);

    // Flatten for source detection and API exposure
    let default_flat = config::flatten_config(&AppConfig::default());
    let configured_flat = config::flatten_config(&configured_config);
    let effective_flat = config::flatten_config(config);

    let configured_sources = config::detect_configured_sources(&default_flat, &file_flat, &db_keys);
    let env_overrides = config::detect_env_overrides(cli);

    Arc::new(archivis_api::settings::service::ConfigService::new(
        effective_flat,
        configured_flat,
        configured_sources,
        env_overrides,
        db_pool.clone(),
    ))
}

/// Initialize the task queue, background workers, and all application services,
/// then build the Axum router.
///
/// Returns the router and an optional watcher service handle for graceful shutdown.
async fn init_services_and_router(
    db_pool: archivis_db::DbPool,
    storage: LocalStorage,
    auth_service: AuthService<LocalAuthAdapter>,
    provider_registry: Arc<ProviderRegistry>,
    config: &AppConfig,
    config_service: Arc<archivis_api::settings::service::ConfigService>,
) -> (axum::Router, Option<Arc<RwLock<WatcherService>>>) {
    let (task_queue, dispatch_rx) = TaskQueue::new(db_pool.clone());
    let task_queue = Arc::new(task_queue);

    // Build shared identification service (used by both worker and API handlers)
    let resolver = Arc::new(MetadataResolver::new(
        Arc::clone(&provider_registry),
        config.metadata.auto_identify_threshold,
    ));
    let identify_service = Arc::new(IdentificationService::new(
        db_pool.clone(),
        Arc::clone(&resolver),
        storage.clone(),
        config.data_dir.clone(),
    ));

    let workers = init_workers(
        &db_pool,
        &storage,
        config,
        &provider_registry,
        &identify_service,
        &task_queue,
    );
    let progress = task_queue.progress_sender();
    let dispatcher_pool = db_pool.clone();
    let cancellation_registry = Arc::clone(task_queue.cancellation_registry());
    tokio::spawn(async move {
        queue::run_dispatcher(
            dispatch_rx,
            workers,
            progress,
            dispatcher_pool,
            cancellation_registry,
        )
        .await;
    });

    // Recover interrupted tasks from previous run
    if let Err(err) = queue::recover_tasks(&db_pool, &task_queue.dispatch_sender()).await {
        tracing::warn!(%err, "Failed to recover interrupted tasks");
    }

    // Initialize filesystem watcher if enabled
    let watcher_service = if config.watcher.enabled {
        match init_watcher(&db_pool, &task_queue).await {
            Ok(ws) => Some(ws),
            Err(err) => {
                tracing::error!(%err, "Failed to initialize watcher service");
                None
            }
        }
    } else {
        tracing::info!("Filesystem watcher disabled");
        None
    };

    // Build merge service
    let merge_service = Arc::new(MergeService::new(
        db_pool.clone(),
        storage.clone(),
        config.data_dir.clone(),
    ));

    let api_config = ApiConfig {
        data_dir: config.data_dir.clone(),
        frontend_dir: config.frontend_dir.clone(),
    };

    // Keep a clone of the watcher handle for graceful shutdown.
    let watcher_handle = watcher_service.clone();

    let state = AppState::new(
        db_pool,
        task_queue,
        auth_service,
        storage,
        provider_registry,
        identify_service,
        merge_service,
        api_config,
        config_service,
        watcher_service,
    );
    (archivis_api::build_router(state), watcher_handle)
}

/// Create and register background task workers.
fn init_workers(
    db_pool: &archivis_db::DbPool,
    storage: &LocalStorage,
    config: &AppConfig,
    _provider_registry: &Arc<ProviderRegistry>,
    identify_service: &Arc<IdentificationService<LocalStorage>>,
    task_queue: &Arc<TaskQueue>,
) -> HashMap<archivis_core::models::TaskType, Arc<dyn Worker>> {
    let import_config = ImportConfig {
        data_dir: config.data_dir.clone(),
        scoring_profile: config.metadata.scoring_profile,
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
            scoring_profile: config.metadata.scoring_profile,
            ..ImportConfig::default()
        },
    )));

    let isbn_scan_on_import = config.isbn_scan.scan_on_import;

    let mut workers: HashMap<archivis_core::models::TaskType, Arc<dyn Worker>> = HashMap::new();
    workers.insert(
        archivis_core::models::TaskType::ImportFile,
        Arc::new(
            ImportFileWorker::new(Arc::clone(&import_service))
                .with_isbn_scan(Arc::clone(task_queue), isbn_scan_on_import),
        ),
    );
    workers.insert(
        archivis_core::models::TaskType::ImportDirectory,
        Arc::new(
            ImportDirectoryWorker::new(Arc::clone(&bulk_import_service))
                .with_isbn_scan(Arc::clone(task_queue), isbn_scan_on_import),
        ),
    );
    workers.insert(
        archivis_core::models::TaskType::IdentifyBook,
        Arc::new(IdentifyWorker::new(Arc::clone(identify_service))),
    );

    // ISBN content-scan worker
    let isbn_scan_config = TaskIsbnScanConfig::from_app_config(
        config.isbn_scan.confidence,
        config.isbn_scan.skip_threshold,
        config.isbn_scan.epub_spine_items,
        config.isbn_scan.pdf_pages,
        config.isbn_scan.fb2_sections,
        config.isbn_scan.txt_bytes,
        config.isbn_scan.mobi_bytes,
    );
    let isbn_scan_service = Arc::new(IsbnScanService::new(
        db_pool.clone(),
        storage.clone(),
        isbn_scan_config,
    ));
    workers.insert(
        archivis_core::models::TaskType::ScanIsbn,
        Arc::new(IsbnScanWorker::new(isbn_scan_service)),
    );

    workers
}

/// Initialize the filesystem watcher service, load runtime settings from DB,
/// start watching configured directories, and spawn the event processing loop.
async fn init_watcher(
    db_pool: &archivis_db::DbPool,
    task_queue: &Arc<TaskQueue>,
) -> Result<Arc<RwLock<WatcherService>>, Box<dyn std::error::Error + Send + Sync>> {
    // Load runtime watcher settings from DB, falling back to defaults.
    let debounce_ms = archivis_db::SettingRepository::get(db_pool, "watcher.debounce_ms")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2000);

    let default_poll_interval_secs =
        archivis_db::SettingRepository::get(db_pool, "watcher.default_poll_interval_secs")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);

    let watcher_config = WatcherRuntimeConfig {
        debounce_ms,
        default_poll_interval_secs,
    };

    let directories = archivis_db::WatchedDirectoryRepository::list_enabled(db_pool).await?;

    // Validate paths before starting the watcher.
    let mut valid_directories = Vec::with_capacity(directories.len());
    for dir in directories {
        let path = std::path::Path::new(&dir.path);
        if !path.exists() {
            tracing::warn!(
                path = %dir.path,
                "watched directory does not exist, skipping (will be retried on next restart)"
            );
            continue;
        }
        if !path.is_dir() {
            tracing::warn!(
                path = %dir.path,
                "watched path is not a directory, skipping"
            );
            continue;
        }
        match path.metadata() {
            Ok(meta) => {
                if meta.permissions().readonly() {
                    // On Unix, readonly() checks the write bit, but what we really care
                    // about is readability. We try to read the directory instead.
                }
                // Attempt to read the directory to verify access.
                if let Err(e) = std::fs::read_dir(path) {
                    tracing::warn!(
                        path = %dir.path,
                        error = %e,
                        "watched directory not accessible, skipping"
                    );
                    continue;
                }
            }
            Err(e) => {
                tracing::warn!(
                    path = %dir.path,
                    error = %e,
                    "cannot read metadata for watched directory, skipping"
                );
                continue;
            }
        }
        valid_directories.push(dir);
    }

    let watcher_service = WatcherService::new(watcher_config)?;
    let event_rx = watcher_service.event_receiver().await;
    watcher_service.start(valid_directories).await?;

    // Spawn the event processing loop if we got the receiver.
    if let Some(event_rx) = event_rx {
        let task_queue_clone = Arc::clone(task_queue);
        let db_pool_clone = db_pool.clone();
        tokio::spawn(async move {
            watcher_processor::run(event_rx, task_queue_clone, db_pool_clone).await;
        });
    }

    let watcher_arc = Arc::new(RwLock::new(watcher_service));
    tracing::info!("Filesystem watcher initialized");

    Ok(watcher_arc)
}

/// Build and configure the metadata provider registry from the application config.
fn init_metadata_providers(metadata_config: &config::MetadataConfig) -> Arc<ProviderRegistry> {
    let version = env!("CARGO_PKG_VERSION");
    let mut http_client =
        MetadataHttpClient::new(version, metadata_config.contact_email.as_deref());

    // Register rate limiters before wrapping in Arc
    OpenLibraryProvider::register_rate_limiter_with_limit(
        &mut http_client,
        metadata_config.open_library.max_requests_per_minute,
    );
    HardcoverProvider::register_rate_limiter_with_limit(
        &mut http_client,
        metadata_config.hardcover.max_requests_per_minute,
    );

    let http_client = Arc::new(http_client);

    let ol_provider = OpenLibraryProvider::new(
        Arc::clone(&http_client),
        metadata_config.enabled && metadata_config.open_library.enabled,
    );
    let hc_provider = HardcoverProvider::new(
        Arc::clone(&http_client),
        metadata_config.hardcover.api_token.clone(),
        metadata_config.enabled && metadata_config.hardcover.enabled,
    );

    if metadata_config.hardcover.enabled && metadata_config.hardcover.api_token.is_none() {
        tracing::warn!(
            "Hardcover provider is enabled but no API token is configured — provider will be unavailable"
        );
    }

    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(ol_provider));
    registry.register(Arc::new(hc_provider));

    let available = registry.available();
    tracing::info!(
        providers = available.len(),
        names = ?available.iter().map(|p| p.name()).collect::<Vec<_>>(),
        "Metadata providers initialized"
    );

    Arc::new(registry)
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    tracing::info!("Shutdown signal received");
}
