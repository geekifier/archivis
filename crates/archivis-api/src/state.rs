use std::path::PathBuf;
use std::sync::Arc;

use archivis_auth::{AuthService, LocalAuthAdapter, ProxyAuth};
use archivis_db::DbPool;
use archivis_metadata::ProviderRegistry;
use archivis_storage::local::LocalStorage;
use archivis_storage::watcher::WatcherService;
use archivis_tasks::identify::IdentificationService;
use archivis_tasks::merge::MergeService;
use archivis_tasks::queue::TaskQueue;
use tokio::sync::RwLock;

use crate::settings::service::ConfigService;

/// API-specific configuration extracted from the application config.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Directory for application data (covers, cache, etc.).
    pub data_dir: PathBuf,
    /// Directory containing the built frontend assets.
    /// When `Some`, the router serves static files and falls back to
    /// `index.html` for SPA client-side routing.
    pub frontend_dir: Option<PathBuf>,
}

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    db_pool: DbPool,
    task_queue: Arc<TaskQueue>,
    auth_service: AuthService<LocalAuthAdapter>,
    storage: LocalStorage,
    provider_registry: Arc<ProviderRegistry>,
    identify_service: Arc<IdentificationService<LocalStorage>>,
    merge_service: Arc<MergeService<LocalStorage>>,
    config: ApiConfig,
    config_service: Arc<ConfigService>,
    /// Optional — `None` when the watcher subsystem is disabled.
    watcher_service: Option<Arc<RwLock<WatcherService>>>,
    /// Optional — `None` when proxy auth is not configured.
    proxy_auth: Option<Arc<ProxyAuth>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_pool: DbPool,
        task_queue: Arc<TaskQueue>,
        auth_service: AuthService<LocalAuthAdapter>,
        storage: LocalStorage,
        provider_registry: Arc<ProviderRegistry>,
        identify_service: Arc<IdentificationService<LocalStorage>>,
        merge_service: Arc<MergeService<LocalStorage>>,
        config: ApiConfig,
        config_service: Arc<ConfigService>,
        watcher_service: Option<Arc<RwLock<WatcherService>>>,
        proxy_auth: Option<Arc<ProxyAuth>>,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db_pool,
                task_queue,
                auth_service,
                storage,
                provider_registry,
                identify_service,
                merge_service,
                config,
                config_service,
                watcher_service,
                proxy_auth,
            }),
        }
    }

    pub fn db_pool(&self) -> &DbPool {
        &self.inner.db_pool
    }

    pub fn task_queue(&self) -> &TaskQueue {
        &self.inner.task_queue
    }

    pub fn auth_service(&self) -> &AuthService<LocalAuthAdapter> {
        &self.inner.auth_service
    }

    pub fn storage(&self) -> &LocalStorage {
        &self.inner.storage
    }

    pub fn provider_registry(&self) -> &Arc<ProviderRegistry> {
        &self.inner.provider_registry
    }

    pub fn identify_service(&self) -> &Arc<IdentificationService<LocalStorage>> {
        &self.inner.identify_service
    }

    pub fn merge_service(&self) -> &Arc<MergeService<LocalStorage>> {
        &self.inner.merge_service
    }

    pub fn config(&self) -> &ApiConfig {
        &self.inner.config
    }

    pub fn config_service(&self) -> &ConfigService {
        &self.inner.config_service
    }

    pub fn watcher_service(&self) -> Option<&Arc<RwLock<WatcherService>>> {
        self.inner.watcher_service.as_ref()
    }

    pub fn proxy_auth(&self) -> Option<&Arc<ProxyAuth>> {
        self.inner.proxy_auth.as_ref()
    }
}
