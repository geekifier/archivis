//! Provenance for each resolved setting value.
//!
//! `ResolvedValue` keeps both the *configured* side (what the admin has asked
//! for, persisted in defaults or DB) and the *effective* side (what the running
//! server is actually using — which differs when an env/CLI pin is active or
//! when a `RestartRequired` key has been changed but not yet reloaded).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::value::SettingValue;

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigSource {
    /// Compiled-in registry default.
    Default,
    /// DB row (admin UI / API).
    Database,
    /// Pinned via environment variable. Name of the variable is carried.
    EnvPin { var: String },
    /// Pinned via CLI flag. Name of the flag is carried.
    CliPin { flag: String },
    /// Set via the TOML config file (bootstrap keys only).
    File,
}

impl ConfigSource {
    /// True when this source cannot be overridden from the admin UI.
    pub const fn is_pin(&self) -> bool {
        matches!(self, Self::EnvPin { .. } | Self::CliPin { .. })
    }
}

/// A resolved runtime setting — configured value, effective value, and provenance.
#[derive(Debug, Clone)]
pub struct ResolvedValue {
    pub configured_value: SettingValue,
    pub configured_source: ConfigSource,
    /// Present when an env/CLI pin is active — takes priority over the
    /// configured value.
    pub pin: Option<(SettingValue, ConfigSource)>,
    pub effective_value: SettingValue,
    pub effective_source: ConfigSource,
}

impl ResolvedValue {
    pub fn is_pinned(&self) -> bool {
        self.pin.is_some()
    }
}
