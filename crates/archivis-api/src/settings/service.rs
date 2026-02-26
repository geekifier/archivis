use std::collections::HashMap;
use std::sync::RwLock;

use archivis_db::{DbPool, SettingRepository};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::registry::{self, SettingType};

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
    effective_config: HashMap<String, serde_json::Value>,
    /// Configured values = what the admin has chosen (DB > file > default).
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
}

pub struct UpdateResult {
    pub updated: Vec<String>,
    pub requires_restart: bool,
}

impl ConfigService {
    pub fn new(
        effective_config: HashMap<String, serde_json::Value>,
        configured: HashMap<String, serde_json::Value>,
        configured_sources: HashMap<String, ConfigSource>,
        overrides: HashMap<String, ConfigOverride>,
        db_pool: DbPool,
    ) -> Self {
        Self {
            effective_config,
            configured: RwLock::new(configured),
            configured_sources: RwLock::new(configured_sources),
            overrides,
            db_pool,
        }
    }

    /// Get all settings with their metadata for the API response.
    pub fn get_all_entries(&self) -> Vec<SettingEntry> {
        let configured = self.configured.read().expect("configured lock poisoned");
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

                let effective_value = self
                    .effective_config
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
                    override_info,
                    requires_restart: meta.requires_restart,
                    label: meta.label.to_string(),
                    description: meta.description.to_string(),
                    section: meta.section.to_string(),
                    value_type: meta.value_type,
                    sensitive: if meta.sensitive { Some(true) } else { None },
                    is_set,
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

        // Remove from configured_sources so source display reverts.
        self.configured_sources
            .write()
            .expect("sources lock poisoned")
            .remove(key);

        Ok(())
    }
}
