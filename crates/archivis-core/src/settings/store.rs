//! In-memory, change-notifying store for all runtime settings.
//!
//! `SettingStore` is the single source of truth for effective runtime values.
//! The store is built at boot from:
//!
//! * The registry (defaults)
//! * DB rows (already normalized and canonicalized upstream in `archivis-api`)
//! * Active env/CLI pins detected at startup
//!
//! After boot, `apply_update` mutates a single key in the snapshot and emits
//! one watch-channel notification. Callers that need batching can skip the
//! channel send by using the explicit batch methods (see `apply_batch`).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::Value;
use tokio::sync::watch;

use super::registry::{self, ApplyMode, RuntimeSettingMeta};
use super::source::{ConfigSource, ResolvedValue};
use super::validation::ValidationError;
use super::value::{canonicalize, SettingValue};
use super::SettingsReader;

/// A lazily-shared read-only snapshot of all runtime settings.
#[derive(Debug)]
pub struct SettingsSnapshot {
    entries: HashMap<&'static str, ResolvedValue>,
}

impl SettingsSnapshot {
    pub fn new(entries: HashMap<&'static str, ResolvedValue>) -> Self {
        Self { entries }
    }

    pub fn get(&self, key: &str) -> Option<&ResolvedValue> {
        self.entries.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&&'static str, &ResolvedValue)> {
        self.entries.iter()
    }
}

/// Errors raised during `SettingStore::from_initial`.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("unknown runtime key in DB: {0}")]
    UnknownKey(String),
    #[error("invalid value for {key}: {source}")]
    Invalid {
        key: String,
        #[source]
        source: ValidationError,
    },
}

/// Pure in-memory store for resolved runtime settings.
pub struct SettingStore {
    snapshot: RwLock<Arc<SettingsSnapshot>>,
    tx: watch::Sender<Arc<SettingsSnapshot>>,
}

impl SettingStore {
    /// Build a store from the registry + normalized DB rows + detected pins.
    ///
    /// `db_rows` must already be canonicalized (keys valid, values matched to
    /// their registry type). `pins` lists environment / CLI overrides detected
    /// at startup.
    pub fn from_initial(
        db_rows: Vec<(String, SettingValue)>,
        pins: Vec<(String, ConfigSource, SettingValue)>,
    ) -> Result<Self, BootError> {
        let mut db_map: HashMap<String, SettingValue> = HashMap::new();
        for (k, v) in db_rows {
            db_map.insert(k, v);
        }

        let mut pin_map: HashMap<String, (ConfigSource, SettingValue)> = HashMap::new();
        for (k, src, v) in pins {
            pin_map.insert(k, (src, v));
        }

        let mut entries: HashMap<&'static str, ResolvedValue> = HashMap::new();
        for meta in registry::runtime_settings() {
            let resolved = resolve_entry(meta, db_map.get(meta.key), pin_map.get(meta.key))?;
            entries.insert(meta.key, resolved);
        }

        let snapshot = Arc::new(SettingsSnapshot::new(entries));
        let (tx, _rx) = watch::channel(Arc::clone(&snapshot));
        Ok(Self {
            snapshot: RwLock::new(snapshot),
            tx,
        })
    }

    /// Current snapshot.
    pub fn snapshot(&self) -> Arc<SettingsSnapshot> {
        Arc::clone(&self.snapshot.read().expect("snapshot lock poisoned"))
    }

    /// Subscribe to snapshot changes.
    pub fn subscribe(&self) -> watch::Receiver<Arc<SettingsSnapshot>> {
        self.tx.subscribe()
    }

    /// Apply a single update. `None` resets to registry default.
    ///
    /// Returns an error if the key is unknown, pinned, or fails validation.
    /// Emits one notification on success.
    pub fn apply_update(
        &self,
        key: &str,
        new_configured: Option<SettingValue>,
    ) -> Result<(), ApplyError> {
        self.apply_batch(vec![(key.to_string(), new_configured)])
    }

    /// Apply multiple updates atomically and emit a single notification.
    pub fn apply_batch(
        &self,
        updates: Vec<(String, Option<SettingValue>)>,
    ) -> Result<(), ApplyError> {
        let mut guard = self.snapshot.write().expect("snapshot lock poisoned");
        let mut next: HashMap<&'static str, ResolvedValue> = guard.entries.clone();

        for (key, new_configured) in updates {
            let meta =
                registry::get_runtime_meta(&key).ok_or_else(|| ApplyError::Unknown(key.clone()))?;
            let existing = next
                .get(meta.key)
                .ok_or_else(|| ApplyError::Unknown(key.clone()))?
                .clone();
            if existing.is_pinned() {
                return Err(ApplyError::Pinned(meta.key.to_string()));
            }

            let (configured_value, configured_source) = match new_configured {
                None => ((meta.default)(), ConfigSource::Default),
                Some(v) => {
                    let c = canonicalize(meta.value_type, &v).map_err(|e| ApplyError::Invalid {
                        key: meta.key.to_string(),
                        source: e,
                    })?;
                    (meta.validator)(&c).map_err(|e| ApplyError::Invalid {
                        key: meta.key.to_string(),
                        source: e,
                    })?;
                    (c, ConfigSource::Database)
                }
            };

            // RestartRequired keeps its effective value frozen until the next
            // boot; other modes apply immediately.
            let (effective_value, effective_source) =
                if matches!(meta.apply_mode, ApplyMode::RestartRequired) {
                    (
                        existing.effective_value.clone(),
                        existing.effective_source.clone(),
                    )
                } else {
                    (configured_value.clone(), configured_source.clone())
                };

            let resolved = ResolvedValue {
                effective_value,
                effective_source,
                configured_value,
                configured_source,
                pin: None,
            };
            next.insert(meta.key, resolved);
        }

        let snapshot = Arc::new(SettingsSnapshot::new(next));
        *guard = Arc::clone(&snapshot);
        drop(guard);
        let _ = self.tx.send(snapshot);
        Ok(())
    }
}

fn resolve_entry(
    meta: &RuntimeSettingMeta,
    db_value: Option<&SettingValue>,
    pin: Option<&(ConfigSource, SettingValue)>,
) -> Result<ResolvedValue, BootError> {
    let (configured_value, configured_source) = match db_value {
        Some(v) => {
            let c = canonicalize(meta.value_type, v).map_err(|e| BootError::Invalid {
                key: meta.key.to_string(),
                source: e,
            })?;
            (meta.validator)(&c).map_err(|e| BootError::Invalid {
                key: meta.key.to_string(),
                source: e,
            })?;
            (c, ConfigSource::Database)
        }
        None => ((meta.default)(), ConfigSource::Default),
    };

    let (effective_value, effective_source, pin_opt) = if let Some((src, val)) = pin {
        let c = canonicalize(meta.value_type, val).map_err(|e| BootError::Invalid {
            key: meta.key.to_string(),
            source: e,
        })?;
        (c.clone(), src.clone(), Some((c, src.clone())))
    } else {
        (configured_value.clone(), configured_source.clone(), None)
    };

    Ok(ResolvedValue {
        configured_value,
        configured_source,
        pin: pin_opt,
        effective_value,
        effective_source,
    })
}

impl SettingsReader for SettingStore {
    fn get_setting(&self, key: &str) -> Option<Value> {
        let snap = self.snapshot();
        snap.get(key).map(|r| r.effective_value.clone())
    }
}

impl SettingsReader for Arc<SettingStore> {
    fn get_setting(&self, key: &str) -> Option<Value> {
        (**self).get_setting(key)
    }
}

/// Errors from `apply_update` / `apply_batch`.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("unknown runtime key: {0}")]
    Unknown(String),
    #[error("key is pinned by env/CLI and cannot be changed via the API: {0}")]
    Pinned(String),
    #[error("invalid value for {key}: {source}")]
    Invalid {
        key: String,
        #[source]
        source: ValidationError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_for_unset_keys() {
        let store = SettingStore::from_initial(vec![], vec![]).unwrap();
        assert_eq!(store.get_setting("metadata.enabled"), Some(json!(true)));
        assert_eq!(store.get_setting("isbn_scan.pdf_pages"), Some(json!(5)));
    }

    #[test]
    fn db_rows_override_defaults() {
        let store = SettingStore::from_initial(
            vec![("isbn_scan.pdf_pages".to_string(), json!(10))],
            vec![],
        )
        .unwrap();
        assert_eq!(store.get_setting("isbn_scan.pdf_pages"), Some(json!(10)));
    }

    #[test]
    fn pins_beat_db() {
        let store = SettingStore::from_initial(
            vec![("metadata.enabled".to_string(), json!(false))],
            vec![(
                "metadata.enabled".to_string(),
                ConfigSource::EnvPin {
                    var: "ARCHIVIS_METADATA__ENABLED".to_string(),
                },
                json!(true),
            )],
        )
        .unwrap();
        assert_eq!(store.get_setting("metadata.enabled"), Some(json!(true)));
        let snap = store.snapshot();
        let e = snap.get("metadata.enabled").unwrap();
        assert!(e.is_pinned());
        assert_eq!(e.configured_value, json!(false));
        assert_eq!(e.effective_value, json!(true));
    }

    #[test]
    fn apply_update_bumps_snapshot_and_notifies() {
        let store = SettingStore::from_initial(vec![], vec![]).unwrap();
        let mut rx = store.subscribe();
        store
            .apply_update("isbn_scan.pdf_pages", Some(json!(8)))
            .unwrap();
        assert!(rx.has_changed().unwrap());
        let effective = rx
            .borrow_and_update()
            .get("isbn_scan.pdf_pages")
            .unwrap()
            .effective_value
            .clone();
        assert_eq!(effective, json!(8));
    }

    #[test]
    fn apply_reset_restores_default() {
        let store = SettingStore::from_initial(
            vec![("isbn_scan.pdf_pages".to_string(), json!(10))],
            vec![],
        )
        .unwrap();
        store.apply_update("isbn_scan.pdf_pages", None).unwrap();
        assert_eq!(store.get_setting("isbn_scan.pdf_pages"), Some(json!(5)));
    }

    #[test]
    fn pinned_key_rejects_update() {
        let store = SettingStore::from_initial(
            vec![],
            vec![(
                "metadata.enabled".to_string(),
                ConfigSource::CliPin {
                    flag: "--metadata-enabled".to_string(),
                },
                json!(false),
            )],
        )
        .unwrap();
        let err = store
            .apply_update("metadata.enabled", Some(json!(true)))
            .unwrap_err();
        assert!(matches!(err, ApplyError::Pinned(_)));
    }

    #[test]
    fn unknown_key_rejects_update() {
        let store = SettingStore::from_initial(vec![], vec![]).unwrap();
        let err = store.apply_update("bogus.key", Some(json!(1))).unwrap_err();
        assert!(matches!(err, ApplyError::Unknown(_)));
    }

    #[test]
    fn subscribe_sees_new_snapshot_on_per_use_change() {
        let store = SettingStore::from_initial(vec![], vec![]).unwrap();
        let mut rx = store.subscribe();
        store
            .apply_update(
                "metadata.open_library.max_requests_per_minute",
                Some(json!(50)),
            )
            .unwrap();
        assert!(rx.has_changed().unwrap());
        let rpm = rx
            .borrow_and_update()
            .get("metadata.open_library.max_requests_per_minute")
            .unwrap()
            .effective_value
            .clone();
        assert_eq!(rpm, json!(50));
    }

    #[test]
    fn restart_required_keeps_effective_frozen() {
        // Boot with debounce_ms = 1500 (a DB row). After apply, configured
        // should flip to 2500 but effective must still be 1500 until restart.
        let store = SettingStore::from_initial(
            vec![("watcher.debounce_ms".to_string(), json!(1500))],
            vec![],
        )
        .unwrap();
        assert_eq!(store.get_setting("watcher.debounce_ms"), Some(json!(1500)));

        store
            .apply_update("watcher.debounce_ms", Some(json!(2500)))
            .unwrap();

        let snap = store.snapshot();
        let entry = snap.get("watcher.debounce_ms").unwrap();
        assert_eq!(entry.configured_value, json!(2500));
        assert_eq!(entry.effective_value, json!(1500));
    }

    #[test]
    fn batch_emits_single_notification() {
        let store = SettingStore::from_initial(vec![], vec![]).unwrap();
        let mut rx = store.subscribe();
        store
            .apply_batch(vec![
                ("isbn_scan.pdf_pages".to_string(), Some(json!(9))),
                ("metadata.enabled".to_string(), Some(json!(false))),
            ])
            .unwrap();
        assert!(rx.has_changed().unwrap());
        let effective = rx
            .borrow_and_update()
            .get("isbn_scan.pdf_pages")
            .unwrap()
            .effective_value
            .clone();
        assert_eq!(effective, json!(9));
        // Calling borrow_and_update again should see no further change.
        assert!(!rx.has_changed().unwrap());
    }
}
