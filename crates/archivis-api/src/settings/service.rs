use std::collections::HashMap;
use std::sync::RwLock;

use archivis_core::settings::SettingsReader;
use archivis_db::{DbPool, SettingRepository};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::registry::{self, SettingScope, SettingType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    File,
    Database,
    Env,
    Cli,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigOverride {
    pub source: ConfigSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
}

pub struct ConfigService {
    /// Effective config = final merged values the server is actually using.
    /// Updated on every settings change (unless the key has an env/CLI override).
    effective_config: RwLock<HashMap<String, serde_json::Value>>,
    /// Pre-DB values (defaults + config file) — used to restore on reset.
    baseline: HashMap<String, serde_json::Value>,
    /// Configured values: bootstrap keys from file/default, runtime keys from DB/default.
    configured: RwLock<HashMap<String, serde_json::Value>>,
    /// Per-key: where the configured value came from.
    configured_sources: RwLock<HashMap<String, ConfigSource>>,
    /// Per-key: which env/CLI overrides are active.
    overrides: HashMap<String, ConfigOverride>,
    /// DB pool for persistence.
    db_pool: DbPool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SettingEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub effective_value: serde_json::Value,
    pub source: ConfigSource,
    pub scope: SettingScope,
    #[serde(rename = "override")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_info: Option<ConfigOverride>,
    pub requires_restart: bool,
    pub label: String,
    pub description: String,
    pub section: String,
    pub value_type: SettingType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_set: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

pub struct UpdateResult {
    pub updated: Vec<String>,
    pub requires_restart: bool,
}

impl ConfigService {
    pub fn new(
        effective_config: HashMap<String, serde_json::Value>,
        baseline: HashMap<String, serde_json::Value>,
        configured: HashMap<String, serde_json::Value>,
        configured_sources: HashMap<String, ConfigSource>,
        overrides: HashMap<String, ConfigOverride>,
        db_pool: DbPool,
    ) -> Self {
        Self {
            effective_config: RwLock::new(effective_config),
            baseline,
            configured: RwLock::new(configured),
            configured_sources: RwLock::new(configured_sources),
            overrides,
            db_pool,
        }
    }

    /// Get all settings with their metadata for the API response.
    pub fn get_all_entries(&self) -> Vec<SettingEntry> {
        let configured = self.configured.read().expect("configured lock poisoned");
        let effective = self
            .effective_config
            .read()
            .expect("effective lock poisoned");
        let sources = self
            .configured_sources
            .read()
            .expect("sources lock poisoned");

        registry::all_settings()
            .iter()
            .map(|meta| {
                let configured_value = configured
                    .get(meta.key)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                let effective_value = effective
                    .get(meta.key)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                let source = sources
                    .get(meta.key)
                    .copied()
                    .unwrap_or(ConfigSource::Default);

                let override_info = self.overrides.get(meta.key).cloned();

                // Mask sensitive values
                let (display_value, display_effective, is_set) = if meta.sensitive {
                    let is_set = match &configured_value {
                        serde_json::Value::Null => false,
                        serde_json::Value::String(s) => !s.is_empty(),
                        _ => true,
                    };
                    (
                        serde_json::Value::String("***".to_string()),
                        serde_json::Value::String("***".to_string()),
                        Some(is_set),
                    )
                } else {
                    (configured_value, effective_value, None)
                };

                SettingEntry {
                    key: meta.key.to_string(),
                    value: display_value,
                    effective_value: display_effective,
                    source,
                    scope: meta.scope,
                    override_info,
                    requires_restart: meta.requires_restart,
                    label: meta.label.to_string(),
                    description: meta.description.to_string(),
                    section: meta.section.to_string(),
                    value_type: meta.value_type,
                    sensitive: if meta.sensitive { Some(true) } else { None },
                    is_set,
                    options: meta
                        .options
                        .map(|opts| opts.iter().map(|s| (*s).to_string()).collect()),
                }
            })
            .collect()
    }

    /// Update one or more settings. Null value = reset to file/default.
    pub async fn update(
        &self,
        updates: &HashMap<String, serde_json::Value>,
    ) -> Result<UpdateResult, String> {
        let mut updated_keys = Vec::new();
        let mut requires_restart = false;

        for (key, value) in updates {
            let Some(meta) = registry::get_setting_meta(key) else {
                return Err(format!("unknown setting: {key}"));
            };

            if meta.scope == SettingScope::Bootstrap {
                return Err(format!(
                    "\"{key}\" is a bootstrap setting \
                     — change it via config file, environment variable, or CLI flag"
                ));
            }

            if meta.requires_restart {
                requires_restart = true;
            }

            if value.is_null() {
                // Reset: delete from DB, revert to file/default
                self.reset_key(key).await?;
            } else {
                // Validate value
                registry::validate_setting_value(meta, value)?;
                // Persist to DB
                let json_str = serde_json::to_string(value)
                    .map_err(|e| format!("failed to serialize value: {e}"))?;
                SettingRepository::set(&self.db_pool, key, &json_str)
                    .await
                    .map_err(|e| format!("failed to save setting: {e}"))?;

                // Update in-memory state
                let mut configured = self.configured.write().expect("configured lock poisoned");
                configured.insert(key.clone(), value.clone());
                drop(configured);

                // Update effective config unless an env/CLI override is active.
                if !self.overrides.contains_key(key) {
                    let mut effective = self
                        .effective_config
                        .write()
                        .expect("effective lock poisoned");
                    effective.insert(key.clone(), value.clone());
                }

                let mut sources = self
                    .configured_sources
                    .write()
                    .expect("sources lock poisoned");
                sources.insert(key.clone(), ConfigSource::Database);
            }

            updated_keys.push(key.clone());
        }

        Ok(UpdateResult {
            updated: updated_keys,
            requires_restart,
        })
    }

    async fn reset_key(&self, key: &str) -> Result<(), String> {
        // Delete from DB
        SettingRepository::delete(&self.db_pool, key)
            .await
            .map_err(|e| format!("failed to delete setting: {e}"))?;

        // Restore the configured value from baseline (defaults + config file).
        let baseline_value = self.baseline.get(key).cloned();

        let mut configured = self.configured.write().expect("configured lock poisoned");
        if let Some(ref val) = baseline_value {
            configured.insert(key.to_string(), val.clone());
        } else {
            configured.remove(key);
        }
        drop(configured);

        // Update effective config unless an env/CLI override is active.
        if !self.overrides.contains_key(key) {
            let mut effective = self
                .effective_config
                .write()
                .expect("effective lock poisoned");
            if let Some(val) = baseline_value {
                effective.insert(key.to_string(), val);
            } else {
                effective.remove(key);
            }
        }

        // Remove from configured_sources so source display reverts.
        self.configured_sources
            .write()
            .expect("sources lock poisoned")
            .remove(key);

        Ok(())
    }
}

impl SettingsReader for ConfigService {
    fn get_setting(&self, key: &str) -> Option<serde_json::Value> {
        self.effective_config
            .read()
            .expect("effective lock poisoned")
            .get(key)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "metadata.hardcover.enabled";

    async fn test_service(
        baseline: HashMap<String, serde_json::Value>,
    ) -> (ConfigService, tempfile::TempDir) {
        test_service_with_overrides(baseline, HashMap::new()).await
    }

    async fn test_service_with_overrides(
        baseline: HashMap<String, serde_json::Value>,
        overrides: HashMap<String, ConfigOverride>,
    ) -> (ConfigService, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db_pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&db_pool).await.unwrap();

        let configured = baseline.clone();
        let effective = baseline.clone();
        let svc = ConfigService::new(
            effective,
            baseline,
            configured,
            HashMap::new(),
            overrides,
            db_pool,
        );
        (svc, tmp)
    }

    #[tokio::test]
    async fn reset_restores_baseline_value() {
        let mut baseline = HashMap::new();
        baseline.insert(TEST_KEY.to_string(), serde_json::Value::Bool(false));

        let (svc, _tmp) = test_service(baseline).await;

        // Save a setting to DB (override to true)
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), serde_json::Value::Bool(true));
        svc.update(&updates).await.unwrap();

        // Verify it was set to true with source Database
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.value, serde_json::Value::Bool(true));
        assert_eq!(entry.source, ConfigSource::Database);

        // Reset (send null)
        let mut reset = HashMap::new();
        reset.insert(TEST_KEY.to_string(), serde_json::Value::Null);
        svc.update(&reset).await.unwrap();

        // Verify it reverted to baseline
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.value, serde_json::Value::Bool(false));
        assert_eq!(entry.source, ConfigSource::Default);
    }

    #[tokio::test]
    async fn reset_without_baseline_returns_null() {
        let (svc, _tmp) = test_service(HashMap::new()).await;

        // Save a setting to DB
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), serde_json::Value::Bool(true));
        svc.update(&updates).await.unwrap();

        // Reset
        let mut reset = HashMap::new();
        reset.insert(TEST_KEY.to_string(), serde_json::Value::Null);
        svc.update(&reset).await.unwrap();

        // Verify value is Null and source is Default
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.value, serde_json::Value::Null);
        assert_eq!(entry.source, ConfigSource::Default);
    }

    #[tokio::test]
    async fn update_propagates_to_effective_config() {
        let mut baseline = HashMap::new();
        baseline.insert(TEST_KEY.to_string(), serde_json::Value::Bool(false));

        let (svc, _tmp) = test_service(baseline).await;

        // effective_value should start at baseline
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.effective_value, serde_json::Value::Bool(false));

        // Update via DB
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), serde_json::Value::Bool(true));
        svc.update(&updates).await.unwrap();

        // effective_value should now reflect the change
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.effective_value, serde_json::Value::Bool(true));
    }

    #[tokio::test]
    async fn update_does_not_override_env_effective() {
        let mut baseline = HashMap::new();
        baseline.insert(TEST_KEY.to_string(), serde_json::Value::Bool(false));

        let mut overrides = HashMap::new();
        overrides.insert(
            TEST_KEY.to_string(),
            ConfigOverride {
                source: ConfigSource::Env,
                env_var: Some("ARCHIVIS_METADATA_HARDCOVER_ENABLED".to_string()),
            },
        );

        let (svc, _tmp) = test_service_with_overrides(baseline, overrides).await;

        // Update via DB — configured changes but effective stays at baseline (env wins)
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), serde_json::Value::Bool(true));
        svc.update(&updates).await.unwrap();

        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.value, serde_json::Value::Bool(true));
        assert_eq!(entry.effective_value, serde_json::Value::Bool(false));
    }

    #[tokio::test]
    async fn reset_propagates_to_effective_config() {
        let mut baseline = HashMap::new();
        baseline.insert(TEST_KEY.to_string(), serde_json::Value::Bool(false));

        let (svc, _tmp) = test_service(baseline).await;

        // Update, then reset
        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), serde_json::Value::Bool(true));
        svc.update(&updates).await.unwrap();

        let mut reset = HashMap::new();
        reset.insert(TEST_KEY.to_string(), serde_json::Value::Null);
        svc.update(&reset).await.unwrap();

        // effective_value should revert to baseline
        let entry = svc
            .get_all_entries()
            .into_iter()
            .find(|e| e.key == TEST_KEY)
            .unwrap();
        assert_eq!(entry.effective_value, serde_json::Value::Bool(false));
    }

    #[tokio::test]
    async fn settings_reader_returns_effective_value() {
        let mut baseline = HashMap::new();
        baseline.insert(TEST_KEY.to_string(), serde_json::Value::Bool(false));

        let (svc, _tmp) = test_service(baseline).await;

        assert_eq!(
            svc.get_setting(TEST_KEY),
            Some(serde_json::Value::Bool(false))
        );

        let mut updates = HashMap::new();
        updates.insert(TEST_KEY.to_string(), serde_json::Value::Bool(true));
        svc.update(&updates).await.unwrap();

        assert_eq!(
            svc.get_setting(TEST_KEY),
            Some(serde_json::Value::Bool(true))
        );
    }
}
