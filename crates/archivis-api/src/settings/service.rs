//! Persistence + API facade over the core `SettingStore`.
//!
//! `ConfigService` is the API-layer owner of the DB connection and the store.
//! Handlers read `get_all_entries()` and call `update()`; both operate in
//! terms of canonical JSON values and the core registry.
//!
//! Bootstrap keys are exposed read-only for the admin UI. Their values come
//! from the in-process `BootstrapConfig` passed in at construction.

use std::collections::HashMap;
use std::sync::Arc;

use archivis_core::settings::{
    all_settings, canonicalize, get_runtime_meta, ApplyMode, ConfigSource as CoreConfigSource,
    RuntimeSettingMeta, SettingMeta, SettingScope, SettingStore, SettingType, SettingValue,
    SettingsReader,
};
use archivis_db::{DbPool, SettingRepository};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// Provenance label surfaced to the admin UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    File,
    Database,
    Env,
    Cli,
}

impl From<&CoreConfigSource> for ConfigSource {
    fn from(s: &CoreConfigSource) -> Self {
        match s {
            CoreConfigSource::Default => Self::Default,
            CoreConfigSource::Database => Self::Database,
            CoreConfigSource::File => Self::File,
            CoreConfigSource::EnvPin { .. } => Self::Env,
            CoreConfigSource::CliPin { .. } => Self::Cli,
        }
    }
}

/// Origin of a pin that makes a runtime setting read-only in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PinSource {
    Env,
    Cli,
}

/// Structured pin detail surfaced to the admin UI.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PinDetail {
    pub source: PinSource,
    /// For `env`: the environment variable name. For `cli`: the flag name.
    pub var_or_flag: String,
}

/// A single setting as exposed by `GET /api/settings`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SettingEntry {
    pub key: String,
    pub scope: SettingScope,
    pub value_type: SettingType,

    /// Value the admin has asked for (default or DB-persisted).
    pub configured_value: Value,
    /// Where the configured value came from.
    pub configured_source: ConfigSource,

    /// Value the server is actually using right now. Differs from
    /// `configured_value` when either (a) a pin is active, or (b) a
    /// `RestartRequired` key has been changed but not yet reloaded.
    pub effective_value: Value,
    pub effective_source: ConfigSource,

    /// Whether the admin UI should render this as read-only. True for all
    /// bootstrap keys and for runtime keys held by an env/CLI pin.
    pub readonly: bool,
    /// Whether a change to this key needs a server restart to take effect.
    pub requires_restart: bool,
    /// Present when a pin is active. The UI surfaces this as a badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_detail: Option<PinDetail>,

    // Descriptive metadata from the registry.
    pub label: String,
    pub description: String,
    pub section: String,
    pub sensitive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// `None` for bootstrap keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_mode: Option<ApplyMode>,
    /// For sensitive optional-string keys, whether a value is persisted.
    /// The actual token is never returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_set: Option<bool>,
}

/// Bootstrap values surfaced read-only to the admin UI.
#[derive(Debug, Clone, Default)]
pub struct BootstrapView {
    pub values: HashMap<String, Value>,
    /// Effective source for each bootstrap key.
    pub sources: HashMap<String, ConfigSource>,
    /// Optional env/CLI detail for bootstrap keys.
    pub pin_details: HashMap<String, PinDetail>,
}

/// Per-key error on `PUT /api/settings`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingErrorCode {
    /// Value could not be canonicalized as the declared type.
    TypeInvalid,
    /// Key is pinned by env/CLI and cannot be changed via the API.
    Pinned,
    /// Key is a bootstrap setting and is not editable at runtime.
    Bootstrap,
    /// Key is not known to the registry.
    Unknown,
    /// Value failed the registry validator.
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SettingError {
    pub key: String,
    pub code: SettingErrorCode,
    pub message: String,
}

#[derive(Debug)]
pub struct UpdateResult {
    pub updated: Vec<String>,
    pub requires_restart: bool,
}

pub struct ConfigService {
    store: Arc<SettingStore>,
    bootstrap: BootstrapView,
    db_pool: DbPool,
}

impl ConfigService {
    pub fn new(store: Arc<SettingStore>, bootstrap: BootstrapView, db_pool: DbPool) -> Self {
        Self {
            store,
            bootstrap,
            db_pool,
        }
    }

    /// Direct access to the store (for wiring `SettingsReader` / `subscribe`).
    pub fn store(&self) -> &Arc<SettingStore> {
        &self.store
    }

    /// Convenience constructor for tests: empty bootstrap view, empty DB rows,
    /// no pins.
    pub fn for_tests(db_pool: DbPool) -> Self {
        let store = Arc::new(
            SettingStore::from_initial(vec![], vec![])
                .expect("default store should build from empty rows"),
        );
        Self::new(store, BootstrapView::default(), db_pool)
    }

    /// API listing for `GET /api/settings`.
    pub fn get_all_entries(&self) -> Vec<SettingEntry> {
        let snap = self.store.snapshot();
        let mut out = Vec::new();

        for meta in all_settings() {
            let entry = match &meta {
                SettingMeta::Bootstrap(bm) => {
                    let raw = self
                        .bootstrap
                        .values
                        .get(bm.key)
                        .cloned()
                        .unwrap_or(Value::Null);
                    let source = self
                        .bootstrap
                        .sources
                        .get(bm.key)
                        .copied()
                        .unwrap_or(ConfigSource::Default);
                    let pin_detail = self.bootstrap.pin_details.get(bm.key).cloned();
                    let (display, is_set) = mask_if_sensitive(bm.sensitive, &raw);
                    SettingEntry {
                        key: bm.key.to_string(),
                        scope: SettingScope::Bootstrap,
                        value_type: bm.value_type,
                        configured_value: display.clone(),
                        configured_source: source,
                        effective_value: display,
                        effective_source: source,
                        readonly: true,
                        requires_restart: true,
                        pin_detail,
                        label: bm.label.to_string(),
                        description: bm.description.to_string(),
                        section: bm.section.to_string(),
                        sensitive: bm.sensitive,
                        options: bm
                            .options
                            .map(|opts| opts.iter().map(|s| (*s).to_string()).collect()),
                        apply_mode: None,
                        is_set,
                    }
                }
                SettingMeta::Runtime(rm) => {
                    let Some(resolved) = snap.get(rm.key) else {
                        continue;
                    };
                    let configured_source = ConfigSource::from(&resolved.configured_source);
                    let effective_source = ConfigSource::from(&resolved.effective_source);
                    let pin_detail = match &resolved.effective_source {
                        CoreConfigSource::EnvPin { var } => Some(PinDetail {
                            source: PinSource::Env,
                            var_or_flag: var.clone(),
                        }),
                        CoreConfigSource::CliPin { flag } => Some(PinDetail {
                            source: PinSource::Cli,
                            var_or_flag: flag.clone(),
                        }),
                        _ => None,
                    };
                    let (configured_display, is_set) =
                        mask_if_sensitive(rm.sensitive, &resolved.configured_value);
                    let (effective_display, _) =
                        mask_if_sensitive(rm.sensitive, &resolved.effective_value);

                    SettingEntry {
                        key: rm.key.to_string(),
                        scope: SettingScope::Runtime,
                        value_type: rm.value_type,
                        configured_value: configured_display,
                        configured_source,
                        effective_value: effective_display,
                        effective_source,
                        readonly: resolved.is_pinned(),
                        requires_restart: rm.requires_restart(),
                        pin_detail,
                        label: rm.label.to_string(),
                        description: rm.description.to_string(),
                        section: rm.section.to_string(),
                        sensitive: rm.sensitive,
                        options: rm
                            .options
                            .map(|opts| opts.iter().map(|s| (*s).to_string()).collect()),
                        apply_mode: Some(rm.apply_mode),
                        is_set,
                    }
                }
            };
            out.push(entry);
        }
        out
    }

    /// Apply runtime setting changes. All-or-nothing: prevalidate everything
    /// before persisting anything. Null value = reset to default.
    ///
    /// On any error, nothing is written and a list of per-key errors is
    /// returned. The caller maps this into a 400 response.
    #[allow(clippy::too_many_lines)]
    pub async fn update(
        &self,
        updates: &HashMap<String, Value>,
    ) -> Result<UpdateResult, Vec<SettingError>> {
        struct Prepared<'a> {
            meta: &'a RuntimeSettingMeta,
            canonical: Option<SettingValue>, // None = reset
        }

        let snap = self.store.snapshot();
        let mut prepared: Vec<Prepared<'_>> = Vec::with_capacity(updates.len());
        let mut errors: Vec<SettingError> = Vec::new();
        let mut requires_restart = false;

        for (key, value) in updates {
            if archivis_core::settings::get_bootstrap_meta(key).is_some() {
                errors.push(SettingError {
                    key: key.clone(),
                    code: SettingErrorCode::Bootstrap,
                    message: format!(
                        "\"{key}\" is a bootstrap setting — change it via config file, \
                         environment variable, or CLI flag"
                    ),
                });
                continue;
            }
            let Some(meta) = get_runtime_meta(key) else {
                errors.push(SettingError {
                    key: key.clone(),
                    code: SettingErrorCode::Unknown,
                    message: format!("unknown setting: {key}"),
                });
                continue;
            };

            if let Some(rv) = snap.get(meta.key) {
                if rv.is_pinned() {
                    errors.push(SettingError {
                        key: key.clone(),
                        code: SettingErrorCode::Pinned,
                        message: format!(
                            "\"{}\" is pinned by env/CLI and cannot be changed via the API",
                            meta.key
                        ),
                    });
                    continue;
                }
            }

            if matches!(meta.apply_mode, ApplyMode::RestartRequired) {
                requires_restart = true;
            }

            let canonical = if value.is_null() && meta.value_type != SettingType::OptionalString {
                None
            } else {
                match canonicalize(meta.value_type, value) {
                    Err(e) => {
                        errors.push(SettingError {
                            key: key.clone(),
                            code: SettingErrorCode::TypeInvalid,
                            message: e.message,
                        });
                        continue;
                    }
                    Ok(c) => {
                        if let Err(e) = (meta.validator)(&c) {
                            errors.push(SettingError {
                                key: key.clone(),
                                code: SettingErrorCode::Invalid,
                                message: e.message,
                            });
                            continue;
                        }
                        Some(c)
                    }
                }
            };

            prepared.push(Prepared { meta, canonical });
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // Persist: reset → delete; value → upsert canonical JSON.
        for p in &prepared {
            let res = match &p.canonical {
                None => SettingRepository::delete(&self.db_pool, p.meta.key).await,
                Some(v) => {
                    let s = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
                    SettingRepository::set(&self.db_pool, p.meta.key, &s).await
                }
            };
            if let Err(e) = res {
                return Err(vec![SettingError {
                    key: p.meta.key.to_string(),
                    code: SettingErrorCode::Invalid,
                    message: format!("failed to persist: {e}"),
                }]);
            }
        }

        let store_batch: Vec<(String, Option<SettingValue>)> = prepared
            .iter()
            .map(|p| (p.meta.key.to_string(), p.canonical.clone()))
            .collect();
        if let Err(e) = self.store.apply_batch(store_batch) {
            return Err(vec![SettingError {
                key: String::new(),
                code: SettingErrorCode::Invalid,
                message: format!("store rejected update: {e}"),
            }]);
        }

        let updated_keys = prepared.iter().map(|p| p.meta.key.to_string()).collect();
        Ok(UpdateResult {
            updated: updated_keys,
            requires_restart,
        })
    }
}

fn mask_if_sensitive(sensitive: bool, v: &Value) -> (Value, Option<bool>) {
    if sensitive {
        let is_set = match v {
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            _ => true,
        };
        (Value::String("***".to_string()), Some(is_set))
    } else {
        (v.clone(), None)
    }
}

impl SettingsReader for ConfigService {
    fn get_setting(&self, key: &str) -> Option<Value> {
        self.store.get_setting(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_KEY: &str = "metadata.hardcover.enabled";

    async fn test_service() -> (ConfigService, tempfile::TempDir) {
        test_service_with_pins(vec![]).await
    }

    async fn test_service_with_pins(
        pins: Vec<(String, CoreConfigSource, SettingValue)>,
    ) -> (ConfigService, tempfile::TempDir) {
        test_service_with_bootstrap(BootstrapView::default(), pins).await
    }

    async fn test_service_with_bootstrap(
        bootstrap: BootstrapView,
        pins: Vec<(String, CoreConfigSource, SettingValue)>,
    ) -> (ConfigService, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&pool).await.unwrap();

        let store = Arc::new(SettingStore::from_initial(vec![], pins).unwrap());
        let svc = ConfigService::new(store, bootstrap, pool);
        (svc, tmp)
    }

    #[tokio::test]
    async fn reset_restores_default_value() {
        let (svc, _tmp) = test_service().await;

        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), json!(true));
        svc.update(&updates).await.unwrap();

        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.configured_value, json!(true));
        assert_eq!(entry.configured_source, ConfigSource::Database);

        let mut reset = HashMap::new();
        reset.insert(TEST_KEY.to_string(), Value::Null);
        svc.update(&reset).await.unwrap();

        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.configured_value, json!(false));
        assert_eq!(entry.configured_source, ConfigSource::Default);
    }

    #[tokio::test]
    async fn update_propagates_to_effective_value() {
        let (svc, _tmp) = test_service().await;

        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), json!(true));
        svc.update(&updates).await.unwrap();

        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.effective_value, json!(true));
    }

    #[tokio::test]
    async fn pinned_key_rejects_update_with_pinned_code() {
        let (svc, _tmp) = test_service_with_pins(vec![(
            TEST_KEY.to_string(),
            CoreConfigSource::EnvPin {
                var: "ARCHIVIS_METADATA__HARDCOVER__ENABLED".to_string(),
            },
            json!(false),
        )])
        .await;

        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), json!(true));
        let errs = svc.update(&updates).await.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0].code, SettingErrorCode::Pinned));
        assert_eq!(errs[0].key, TEST_KEY);
    }

    #[tokio::test]
    async fn pinned_entry_exposes_pin_detail_and_readonly() {
        let (svc, _tmp) = test_service_with_pins(vec![(
            TEST_KEY.to_string(),
            CoreConfigSource::EnvPin {
                var: "ARCHIVIS_METADATA__HARDCOVER__ENABLED".to_string(),
            },
            json!(true),
        )])
        .await;

        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert!(entry.readonly);
        let pin = entry.pin_detail.unwrap();
        assert!(matches!(pin.source, PinSource::Env));
        assert_eq!(pin.var_or_flag, "ARCHIVIS_METADATA__HARDCOVER__ENABLED");
        assert_eq!(entry.effective_source, ConfigSource::Env);
    }

    #[tokio::test]
    async fn unknown_key_rejects_update_with_unknown_code() {
        let (svc, _tmp) = test_service().await;
        let mut updates = HashMap::new();
        updates.insert("bogus.key".to_string(), json!(1));
        let errs = svc.update(&updates).await.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0].code, SettingErrorCode::Unknown));
    }

    #[tokio::test]
    async fn bootstrap_key_rejects_update_with_bootstrap_code() {
        let (svc, _tmp) = test_service().await;
        let mut updates = HashMap::new();
        updates.insert("port".to_string(), json!(9090));
        let errs = svc.update(&updates).await.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0].code, SettingErrorCode::Bootstrap));
    }

    #[tokio::test]
    async fn type_invalid_gets_dedicated_code() {
        let (svc, _tmp) = test_service().await;
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), json!("not a bool"));
        let errs = svc.update(&updates).await.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0].code, SettingErrorCode::TypeInvalid));
    }

    #[tokio::test]
    async fn validator_failure_gets_invalid_code() {
        let (svc, _tmp) = test_service().await;
        let mut updates = HashMap::new();
        updates.insert("metadata.auto_apply_threshold".to_string(), json!(5.0));
        let errs = svc.update(&updates).await.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0].code, SettingErrorCode::Invalid));
    }

    #[tokio::test]
    async fn batch_is_all_or_nothing() {
        let (svc, _tmp) = test_service().await;
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), json!(true));
        updates.insert("bogus.key".to_string(), json!(1));
        let errs = svc.update(&updates).await.unwrap_err();
        assert_eq!(errs.len(), 1);

        // The valid key must NOT have been persisted.
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.configured_value, json!(false));
    }

    #[tokio::test]
    async fn settings_reader_returns_effective_value() {
        let (svc, _tmp) = test_service().await;

        assert_eq!(svc.get_setting(TEST_KEY), Some(json!(false)));

        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), json!(true));
        svc.update(&updates).await.unwrap();

        assert_eq!(svc.get_setting(TEST_KEY), Some(json!(true)));
    }

    #[tokio::test]
    async fn restart_required_key_reports_flag() {
        let (svc, _tmp) = test_service().await;
        let mut updates = HashMap::new();
        updates.insert("watcher.debounce_ms".to_string(), json!(1500));
        let result = svc.update(&updates).await.unwrap();
        assert!(result.requires_restart);
    }
}
