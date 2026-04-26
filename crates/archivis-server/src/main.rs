mod config;
mod telemetry;

use std::collections::HashMap;
use std::sync::Arc;

use archivis_api::state::{ApiConfig, AppState};
use archivis_auth::{AuthService, LocalAuthAdapter};
use archivis_core::public_url::PublicBaseUrl;
use archivis_formats::transform::{FormatTransformer, TransformerRegistry};
use archivis_kepub::KepubTransformer;
use archivis_metadata::{
    HardcoverProvider, LocProvider, MetadataHttpClient, MetadataResolver, OpenLibraryProvider,
    ProviderRegistry,
};
use archivis_storage::local::LocalStorage;
use archivis_storage::watcher::{service::WatcherRuntimeConfig, WatcherService};
use archivis_tasks::import::{BulkImportService, ImportConfig, ImportService};
use archivis_tasks::isbn_scan::IsbnScanService;
use archivis_tasks::merge::MergeService;
use archivis_tasks::queue::{self, TaskQueue, Worker};
use archivis_tasks::resolve::ResolutionService;
use archivis_tasks::workers::{
    watcher_processor, BulkSetTagsWorker, BulkUpdateWorker, ImportDirectoryWorker,
    ImportFileWorker, IsbnScanWorker, ResolveWorker,
};
use clap::Parser;
use config::{AppConfig, Cli};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

#[tokio::main]
#[allow(clippy::too_many_lines)]
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
        public_base_url = ?config.public_base_url,
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

    // Backfill live metadata quality scores for existing books (runs in background)
    {
        let backfill_pool = db_pool.clone();
        tokio::spawn(async move {
            if let Err(e) =
                archivis_tasks::resolve::backfill_metadata_quality_scores(&backfill_pool).await
            {
                tracing::warn!("backfill metadata quality scores failed: {e}");
            }
        });
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

    // 3a. Bootstrap admin from environment variables (if no admin exists yet)
    bootstrap_admin(&auth_service).await;

    // 3b. Proxy auth (ForwardAuth)
    let proxy_auth = if config.auth.proxy.enabled {
        match archivis_auth::ProxyAuth::new(
            &config.auth.proxy.trusted_proxies,
            config.auth.proxy.user_header.clone(),
            config.auth.proxy.email_header.clone(),
            config.auth.proxy.groups_header.clone(),
        ) {
            Ok(pa) => {
                tracing::info!(
                    trusted_proxies = ?config.auth.proxy.trusted_proxies,
                    user_header = %config.auth.proxy.user_header,
                    "Reverse proxy authentication enabled"
                );
                Some(std::sync::Arc::new(pa))
            }
            Err(err) => {
                tracing::error!(%err, "Failed to initialize proxy auth — disabling");
                None
            }
        }
    } else {
        None
    };

    // 4. Metadata providers
    let settings_reader: Arc<dyn archivis_core::settings::SettingsReader> =
        Arc::clone(&config_service) as _;
    let provider_registry =
        init_metadata_providers(&config.metadata, &settings_reader, config_service.store());

    // 5. Task queue, workers, and services
    let (router, watcher_service) = init_services_and_router(
        db_pool,
        storage,
        auth_service,
        provider_registry,
        &config,
        config_service,
        Arc::clone(&settings_reader),
        proxy_auth,
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

    let router = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
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

/// Create an admin user from bootstrap environment variables.
///
/// Reads `ARCHIVIS_ADMIN_USERNAME`, `ARCHIVIS_ADMIN_PASSWORD` (or
/// `ARCHIVIS_ADMIN_PASSWORD_FILE`), and optionally `ARCHIVIS_ADMIN_EMAIL`.
/// These are intentionally NOT part of `AppConfig` / Figment — they are
/// one-time bootstrap vars that should never be persisted or shown in the
/// settings API.
async fn bootstrap_admin(auth_service: &AuthService<LocalAuthAdapter>) {
    let setup_required = match auth_service.is_setup_required().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(%e, "Failed to check setup status during bootstrap");
            std::process::exit(1);
        }
    };

    let has_env = std::env::var("ARCHIVIS_ADMIN_USERNAME").is_ok()
        || std::env::var("ARCHIVIS_ADMIN_PASSWORD").is_ok()
        || std::env::var("ARCHIVIS_ADMIN_PASSWORD_FILE").is_ok();

    if !setup_required {
        if has_env {
            tracing::warn!(
                "Bootstrap env vars (ARCHIVIS_ADMIN_*) are set but an admin already exists \
                 — consider removing them for security"
            );
        }
        return;
    }

    let username = match std::env::var("ARCHIVIS_ADMIN_USERNAME") {
        Ok(u) if !u.is_empty() => u,
        _ => return, // No bootstrap requested — setup wizard will handle it
    };

    // Read password: try _FILE variant first (for Docker secrets), then plain env var
    let password = match std::env::var("ARCHIVIS_ADMIN_PASSWORD_FILE") {
        Ok(path) if !path.is_empty() => match std::fs::read_to_string(&path) {
            Ok(contents) => contents.trim().to_string(),
            Err(e) => {
                tracing::error!(path = %path, %e, "Failed to read ARCHIVIS_ADMIN_PASSWORD_FILE");
                std::process::exit(1);
            }
        },
        _ => match std::env::var("ARCHIVIS_ADMIN_PASSWORD") {
            Ok(p) if !p.is_empty() => p,
            _ => {
                tracing::error!(
                    "ARCHIVIS_ADMIN_USERNAME is set but ARCHIVIS_ADMIN_PASSWORD \
                     (or ARCHIVIS_ADMIN_PASSWORD_FILE) is missing — cannot bootstrap admin"
                );
                std::process::exit(1);
            }
        },
    };

    let email = std::env::var("ARCHIVIS_ADMIN_EMAIL")
        .ok()
        .filter(|e| !e.is_empty());

    match auth_service
        .create_user(
            &username,
            &password,
            email.as_deref(),
            archivis_core::models::UserRole::Admin,
        )
        .await
    {
        Ok(_) => {
            tracing::info!(
                username = %username,
                "Admin user created from bootstrap environment variables"
            );
        }
        Err(e) => {
            tracing::error!(%e, "Failed to create bootstrap admin user");
            std::process::exit(1);
        }
    }
}

/// Normalize legacy DB rows, build the `SettingStore`, and wrap it in the
/// `ConfigService` that powers the admin settings API.
///
/// Side effect: after this function, `config` has DB overrides applied so that
/// the existing worker-init paths see runtime values. Phase 3 will remove this
/// coupling by making workers read from the store directly.
async fn init_config_service(
    cli: &Cli,
    config: &mut AppConfig,
    db_pool: &archivis_db::DbPool,
) -> Arc<archivis_api::settings::service::ConfigService> {
    // 1. Normalize legacy DB rows (one-shot: canonicalize, delete-if-default,
    //    hard-fail on unknown / bootstrap keys).
    let normalized = match archivis_api::settings::normalize::normalize_settings(db_pool).await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(%err, "Settings normalization failed");
            std::process::exit(1);
        }
    };

    // Convert normalized rows into a (String, String) list mirroring the old
    // shape that `apply_db_settings` consumes.
    let db_settings_stringy: Vec<(String, String)> = normalized
        .rows
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                serde_json::to_string(v).unwrap_or_else(|_| "null".into()),
            )
        })
        .collect();

    // 2. File-only config snapshot (defaults + TOML, no env/CLI) — used for
    //    bootstrap source detection.
    let config_path = cli.config.to_str().unwrap_or("config.toml");
    let file_cli = Cli::parse_from::<[&str; 3], &str>(["archivis", "--config", config_path]);
    let file_config = AppConfig::load(&file_cli).unwrap_or_default();
    let file_flat = config::flatten_config(&file_config);

    // 3. Apply DB overrides to the effective `AppConfig` so existing worker
    //    init code still sees runtime values. Phase 3 migrates consumers off
    //    of this shim and onto `SettingsReader`.
    config::apply_db_settings(config, &db_settings_stringy);
    let default_flat = config::flatten_config(&AppConfig::default());
    let effective_flat = config::flatten_config(config);

    // 4. Env/CLI pin detection.
    let env_overrides = config::detect_env_overrides(cli);
    let runtime_pins = config::build_runtime_pins(&env_overrides, &effective_flat);

    // 5. Build the core `SettingStore` from normalized runtime rows + pins.
    let store =
        match archivis_core::settings::SettingStore::from_initial(normalized.rows, runtime_pins) {
            Ok(s) => std::sync::Arc::new(s),
            Err(err) => {
                tracing::error!(%err, "Failed to build setting store");
                std::process::exit(1);
            }
        };

    // 6. Build the bootstrap view exposed read-only via the admin UI.
    let bootstrap_view =
        config::build_bootstrap_view(&default_flat, &file_flat, &effective_flat, &env_overrides);

    Arc::new(archivis_api::settings::service::ConfigService::new(
        store,
        bootstrap_view,
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
    settings_reader: Arc<dyn archivis_core::settings::SettingsReader>,
    proxy_auth: Option<Arc<archivis_auth::ProxyAuth>>,
) -> (axum::Router, Option<Arc<RwLock<WatcherService>>>) {
    let (task_queue, dispatch_rx) = TaskQueue::new(db_pool.clone());
    let task_queue = Arc::new(task_queue);

    // Build shared resolution service (used by both workers and compatibility handlers)
    let resolver = Arc::new(MetadataResolver::new(
        Arc::clone(&provider_registry),
        Arc::clone(&settings_reader),
    ));
    let resolve_service = Arc::new(ResolutionService::new(
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
        &resolve_service,
        &task_queue,
        &settings_reader,
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

    // Periodic DB maintenance (expired sessions + old tasks)
    let maintenance_pool = db_pool.clone();
    tokio::spawn(async move {
        archivis_tasks::maintenance::run_maintenance_loop(maintenance_pool).await;
    });

    // Recover interrupted tasks from previous run
    if let Err(err) = queue::recover_tasks(&db_pool, &task_queue.dispatch_sender()).await {
        tracing::warn!(%err, "Failed to recover interrupted tasks");
    }

    // Initialize filesystem watcher if enabled
    let watcher_service = if config.watcher.enabled {
        match init_watcher(&db_pool, &task_queue, &settings_reader).await {
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
        public_base_url: config
            .public_base_url
            .as_deref()
            .map(|value| PublicBaseUrl::parse(value).expect("public_base_url validated at load")),
    };

    // Generate ephemeral scope signing key (not persisted — server restart
    // invalidates outstanding scope tokens, which is the intended behavior).
    let scope_signing_key: [u8; 32] = rand::random();

    // Keep a clone of the watcher handle for graceful shutdown.
    let watcher_handle = watcher_service.clone();

    let transformers: Arc<TransformerRegistry> = Arc::new(TransformerRegistry::new(vec![
        Arc::new(KepubTransformer) as Arc<dyn FormatTransformer>,
    ]));

    let state = AppState::new(
        db_pool,
        task_queue,
        auth_service,
        storage,
        provider_registry,
        resolve_service,
        merge_service,
        api_config,
        config_service,
        transformers,
        watcher_service,
        proxy_auth,
        scope_signing_key,
    );
    (archivis_api::build_router(state), watcher_handle)
}

/// Create and register background task workers.
fn init_workers(
    db_pool: &archivis_db::DbPool,
    storage: &LocalStorage,
    config: &AppConfig,
    _provider_registry: &Arc<ProviderRegistry>,
    resolve_service: &Arc<ResolutionService<LocalStorage>>,
    task_queue: &Arc<TaskQueue>,
    settings_reader: &Arc<dyn archivis_core::settings::SettingsReader>,
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
        Arc::clone(settings_reader),
    ));
    let bulk_import_service = Arc::new(BulkImportService::new(ImportService::new(
        db_pool.clone(),
        storage.clone(),
        ImportConfig {
            data_dir: config.data_dir.clone(),
            scoring_profile: config.metadata.scoring_profile,
            ..ImportConfig::default()
        },
        Arc::clone(settings_reader),
    )));

    let mut workers: HashMap<archivis_core::models::TaskType, Arc<dyn Worker>> = HashMap::new();
    workers.insert(
        archivis_core::models::TaskType::ImportFile,
        Arc::new(
            ImportFileWorker::new(Arc::clone(&import_service))
                .with_isbn_scan(Arc::clone(task_queue), Arc::clone(settings_reader)),
        ),
    );
    workers.insert(
        archivis_core::models::TaskType::ImportDirectory,
        Arc::new(
            ImportDirectoryWorker::new(Arc::clone(&bulk_import_service))
                .with_isbn_scan(Arc::clone(task_queue), Arc::clone(settings_reader)),
        ),
    );
    workers.insert(
        archivis_core::models::TaskType::ResolveBook,
        Arc::new(ResolveWorker::new(Arc::clone(resolve_service))),
    );

    // ISBN content-scan worker — runtime knobs are read from the store at
    // task start (`PerUse`).
    let isbn_scan_service = Arc::new(IsbnScanService::new(
        db_pool.clone(),
        storage.clone(),
        Arc::clone(settings_reader),
    ));
    workers.insert(
        archivis_core::models::TaskType::ScanIsbn,
        Arc::new(
            IsbnScanWorker::new(isbn_scan_service).with_resolution_queue(Arc::clone(task_queue)),
        ),
    );

    // Bulk operation workers
    workers.insert(
        archivis_core::models::TaskType::BulkUpdate,
        Arc::new(BulkUpdateWorker::new(db_pool.clone())),
    );
    workers.insert(
        archivis_core::models::TaskType::BulkSetTags,
        Arc::new(BulkSetTagsWorker::new(db_pool.clone())),
    );

    workers
}

/// Initialize the filesystem watcher service, load runtime settings from DB,
/// start watching configured directories, and spawn the event processing loop.
async fn init_watcher(
    db_pool: &archivis_db::DbPool,
    task_queue: &Arc<TaskQueue>,
    settings_reader: &Arc<dyn archivis_core::settings::SettingsReader>,
) -> Result<Arc<RwLock<WatcherService>>, Box<dyn std::error::Error + Send + Sync>> {
    use archivis_core::settings::SettingsReaderExt;
    // Boot snapshot — watcher debounce/poll keys are RestartRequired, so a
    // one-shot read here is the authoritative source until the next restart.
    let debounce_ms = settings_reader
        .get_u64("watcher.debounce_ms")
        .unwrap_or(2000);
    let default_poll_interval_secs = settings_reader
        .get_u64("watcher.default_poll_interval_secs")
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
        let settings_clone = Arc::clone(settings_reader);
        tokio::spawn(async move {
            watcher_processor::run(event_rx, task_queue_clone, db_pool_clone, settings_clone).await;
        });
    }

    let watcher_arc = Arc::new(RwLock::new(watcher_service));
    tracing::info!("Filesystem watcher initialized");

    Ok(watcher_arc)
}

/// Build and configure the metadata provider registry from the application config.
fn init_metadata_providers(
    metadata_config: &config::MetadataConfig,
    settings: &Arc<dyn archivis_core::settings::SettingsReader>,
    store: &Arc<archivis_core::settings::SettingStore>,
) -> Arc<ProviderRegistry> {
    let version = env!("ARCHIVIS_VERSION");
    let mut http_client =
        MetadataHttpClient::new(version, metadata_config.contact_email.as_deref())
            .with_settings(Arc::clone(settings));

    // Register rate limiters before wrapping in Arc
    OpenLibraryProvider::register_rate_limiter_with_limit(
        &mut http_client,
        metadata_config.open_library.max_requests_per_minute,
    );
    HardcoverProvider::register_rate_limiter_with_limit(
        &mut http_client,
        metadata_config.hardcover.max_requests_per_minute,
    );
    LocProvider::register_rate_limiter_with_limit(
        &mut http_client,
        metadata_config.loc.max_requests_per_minute,
    );

    let http_client = Arc::new(http_client);

    // Subscribe to store changes and live-reload rate-limit settings.
    spawn_rate_limit_reloader(Arc::clone(&http_client), store);

    let ol_provider = OpenLibraryProvider::new(Arc::clone(&http_client), Arc::clone(settings));
    let hc_provider = HardcoverProvider::new(Arc::clone(&http_client), Arc::clone(settings));
    let loc_provider = LocProvider::new(Arc::clone(&http_client), Arc::clone(settings));

    if metadata_config.hardcover.enabled && metadata_config.hardcover.api_token.is_none() {
        tracing::warn!(
            "Hardcover provider is enabled but no API token is configured — provider will be unavailable"
        );
    }

    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(ol_provider));
    registry.register(Arc::new(hc_provider));
    registry.register(Arc::new(loc_provider));

    let available = registry.available();
    tracing::info!(
        providers = available.len(),
        names = ?available.iter().map(|p| p.name()).collect::<Vec<_>>(),
        "Metadata providers initialized"
    );

    Arc::new(registry)
}

/// Subscribe to setting changes and live-reload metadata provider rate limits.
///
/// One central reload path for all three `Subscribed` keys; in-flight requests
/// keep the limiter they observed at start, while new requests see the
/// updated rate.
fn spawn_rate_limit_reloader(
    http_client: Arc<MetadataHttpClient>,
    store: &Arc<archivis_core::settings::SettingStore>,
) {
    use archivis_core::settings::SettingsReaderExt;
    let mut rx = store.subscribe();
    let store = Arc::clone(store);
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            // Snapshot is already the new one. Apply fresh rates.
            if let Some(rpm) = store.get_u32("metadata.open_library.max_requests_per_minute") {
                http_client.update_rate("open_library", rpm);
            }
            if let Some(rpm) = store.get_u32("metadata.hardcover.max_requests_per_minute") {
                http_client.update_rate("hardcover", rpm);
            }
            if let Some(rpm) = store.get_u32("metadata.loc.max_requests_per_minute") {
                http_client.update_rate("loc", rpm);
            }
        }
    });
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    tracing::info!("Shutdown signal received");
}
