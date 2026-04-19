//! Settings policy layer.
//!
//! This module owns the **policy** for the settings subsystem: the registry of
//! known keys, per-key metadata (type, default, validator, apply-mode),
//! in-memory store with change notifications, and the `SettingsReader` trait
//! consumed throughout the codebase.
//!
//! Persistence lives in `archivis-api` (`ConfigService` + DB), and bootstrap
//! loading lives in `archivis-server`. This split matches the architectural
//! constraint that the domain layer (`archivis-core`) is free of HTTP, DB, and
//! filesystem concerns.

pub mod registry;
pub mod source;
pub mod store;
pub mod validation;
pub mod value;

pub use registry::{
    all_settings, bootstrap_settings, canonical_setting_key, get_bootstrap_meta, get_runtime_meta,
    get_setting_meta, legacy_setting_keys, runtime_default, runtime_settings, ApplyMode,
    BootstrapSettingMeta, RuntimeSettingMeta, SettingMeta, SettingScope, SettingType,
};
pub use source::{ConfigSource, ResolvedValue};
pub use store::{BootError, SettingStore, SettingsSnapshot};
pub use validation::{ValidationError, Validator};
pub use value::{canonicalize, values_equal, SettingDefault, SettingValue};

/// Read-only access to runtime configuration values.
///
/// Implemented by `SettingStore` and by the `ConfigService` facade. Consumers
/// that need live runtime values take `Arc<dyn SettingsReader>` and read at
/// point-of-use (for `PerUse` keys) or subscribe to the store's change stream
/// (for `Subscribed` keys).
pub trait SettingsReader: Send + Sync {
    /// Return the effective value for `key`, or `None` if unknown.
    fn get_setting(&self, key: &str) -> Option<serde_json::Value>;
}

/// Convenience helpers over `SettingsReader::get_setting`.
pub trait SettingsReaderExt: SettingsReader {
    fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_setting(key)?.as_bool()
    }

    fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_setting(key)?.as_i64()
    }

    fn get_u32(&self, key: &str) -> Option<u32> {
        u32::try_from(self.get_i64(key)?).ok()
    }

    fn get_u64(&self, key: &str) -> Option<u64> {
        u64::try_from(self.get_i64(key)?).ok()
    }

    fn get_usize(&self, key: &str) -> Option<usize> {
        usize::try_from(self.get_i64(key)?).ok()
    }

    fn get_f64(&self, key: &str) -> Option<f64> {
        self.get_setting(key)?.as_f64()
    }

    #[allow(clippy::cast_possible_truncation)]
    fn get_f32(&self, key: &str) -> Option<f32> {
        self.get_f64(key).map(|v| v as f32)
    }

    fn get_string(&self, key: &str) -> Option<String> {
        self.get_setting(key)?.as_str().map(str::to_owned)
    }

    fn get_optional_string(&self, key: &str) -> Option<String> {
        match self.get_setting(key)? {
            serde_json::Value::String(s) => Some(s),
            _ => None,
        }
    }
}

impl<T: SettingsReader + ?Sized> SettingsReaderExt for T {}
