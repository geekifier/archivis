pub mod handlers;
pub mod normalize;
/// Back-compat re-export: downstream code still writes
/// `use crate::settings::registry::...`. The authoritative registry lives in
/// `archivis_core::settings::registry` — this module is a thin alias.
pub mod registry {
    pub use archivis_core::settings::{
        all_settings, canonical_setting_key, get_bootstrap_meta, get_runtime_meta,
        get_setting_meta, legacy_setting_keys, runtime_settings, SettingMeta, SettingScope,
        SettingType,
    };
}
pub mod service;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/",
        get(handlers::get_settings).put(handlers::update_settings),
    )
}
