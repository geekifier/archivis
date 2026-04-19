//! Static registry of all known settings.
//!
//! Runtime settings declare an `ApplyMode` (read at every use, subscribed, or
//! restart-required), a `default` provider, and a `validator`. Bootstrap
//! settings are purely descriptive: they describe keys loaded from the TOML
//! file / env / CLI at boot and are read-only in the admin UI.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::validation::{
    bytes_range, concurrency_range, debounce_ms_range, pass, poll_interval_range, rpm_range,
    small_count_range, unit_interval, validate_enum, ValidationError, Validator,
};
use super::value::{SettingDefault, SettingValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    String,
    OptionalString,
    Bool,
    Integer,
    Float,
    Select,
}

/// Whether a setting is set at boot (bootstrap) or at runtime (admin UI / DB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingScope {
    Bootstrap,
    Runtime,
}

/// How a changed runtime value propagates to its consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    /// Consumer re-reads the value at every unit of work.
    PerUse,
    /// Consumer subscribes to change notifications and refreshes long-lived
    /// state when they fire.
    Subscribed,
    /// New value only takes effect after the server is restarted.
    RestartRequired,
}

/// Metadata for a runtime setting.
pub struct RuntimeSettingMeta {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub section: &'static str,
    pub value_type: SettingType,
    pub apply_mode: ApplyMode,
    pub default: SettingDefault,
    pub validator: Validator,
    pub sensitive: bool,
    pub options: Option<&'static [&'static str]>,
}

impl RuntimeSettingMeta {
    pub fn requires_restart(&self) -> bool {
        matches!(self.apply_mode, ApplyMode::RestartRequired)
    }
}

/// Metadata for a bootstrap setting (file / env / CLI only).
pub struct BootstrapSettingMeta {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub section: &'static str,
    pub value_type: SettingType,
    pub sensitive: bool,
    pub options: Option<&'static [&'static str]>,
}

/// Unified view used by API handlers iterating both scopes.
pub enum SettingMeta {
    Bootstrap(&'static BootstrapSettingMeta),
    Runtime(&'static RuntimeSettingMeta),
}

impl SettingMeta {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Bootstrap(m) => m.key,
            Self::Runtime(m) => m.key,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Bootstrap(m) => m.label,
            Self::Runtime(m) => m.label,
        }
    }
    pub fn description(&self) -> &'static str {
        match self {
            Self::Bootstrap(m) => m.description,
            Self::Runtime(m) => m.description,
        }
    }
    pub fn section(&self) -> &'static str {
        match self {
            Self::Bootstrap(m) => m.section,
            Self::Runtime(m) => m.section,
        }
    }
    pub fn value_type(&self) -> SettingType {
        match self {
            Self::Bootstrap(m) => m.value_type,
            Self::Runtime(m) => m.value_type,
        }
    }
    pub fn scope(&self) -> SettingScope {
        match self {
            Self::Bootstrap(_) => SettingScope::Bootstrap,
            Self::Runtime(_) => SettingScope::Runtime,
        }
    }
    pub fn sensitive(&self) -> bool {
        match self {
            Self::Bootstrap(m) => m.sensitive,
            Self::Runtime(m) => m.sensitive,
        }
    }
    pub fn options(&self) -> Option<&'static [&'static str]> {
        match self {
            Self::Bootstrap(m) => m.options,
            Self::Runtime(m) => m.options,
        }
    }
    pub fn apply_mode(&self) -> Option<ApplyMode> {
        match self {
            Self::Bootstrap(_) => None,
            Self::Runtime(m) => Some(m.apply_mode),
        }
    }
    pub fn requires_restart(&self) -> bool {
        match self {
            Self::Bootstrap(_) => true,
            Self::Runtime(m) => m.requires_restart(),
        }
    }
}

/// All runtime settings.
pub fn runtime_settings() -> &'static [RuntimeSettingMeta] {
    RUNTIME
}

/// All bootstrap settings.
pub fn bootstrap_settings() -> &'static [BootstrapSettingMeta] {
    BOOTSTRAP
}

/// All settings (bootstrap + runtime) for API enumeration.
pub fn all_settings() -> Vec<SettingMeta> {
    let mut v: Vec<SettingMeta> = Vec::with_capacity(BOOTSTRAP.len() + RUNTIME.len());
    for m in BOOTSTRAP {
        v.push(SettingMeta::Bootstrap(m));
    }
    for m in RUNTIME {
        v.push(SettingMeta::Runtime(m));
    }
    v
}

/// Find a setting by key, in either scope.
pub fn get_setting_meta(key: &str) -> Option<SettingMeta> {
    if let Some(m) = BOOTSTRAP.iter().find(|m| m.key == key) {
        return Some(SettingMeta::Bootstrap(m));
    }
    if let Some(m) = RUNTIME.iter().find(|m| m.key == key) {
        return Some(SettingMeta::Runtime(m));
    }
    None
}

pub fn get_runtime_meta(key: &str) -> Option<&'static RuntimeSettingMeta> {
    RUNTIME.iter().find(|m| m.key == key)
}

pub fn get_bootstrap_meta(key: &str) -> Option<&'static BootstrapSettingMeta> {
    BOOTSTRAP.iter().find(|m| m.key == key)
}

// ── Default providers ────────────────────────────────────────────────

fn d_null() -> Value {
    Value::Null
}
fn d_true() -> Value {
    json!(true)
}
fn d_false() -> Value {
    json!(false)
}

// Metadata defaults
fn d_auto_apply_threshold() -> Value {
    json!(0.85)
}
fn d_max_concurrent_resolutions() -> Value {
    json!(2)
}
fn d_scoring_profile() -> Value {
    json!("balanced")
}

// Provider defaults
fn d_ol_rpm() -> Value {
    json!(100)
}
fn d_hc_rpm() -> Value {
    json!(50)
}
fn d_loc_rpm() -> Value {
    json!(20)
}

// ISBN scan defaults
fn d_confidence() -> Value {
    json!(0.5)
}
fn d_skip_threshold() -> Value {
    json!(0.95)
}
fn d_epub_spine() -> Value {
    json!(5)
}
fn d_pdf_pages() -> Value {
    json!(5)
}
fn d_fb2_sections() -> Value {
    json!(3)
}
fn d_txt_bytes() -> Value {
    json!(4000)
}
fn d_mobi_bytes() -> Value {
    json!(8000)
}

// Watcher defaults
fn d_debounce_ms() -> Value {
    json!(2000)
}
fn d_poll_interval() -> Value {
    json!(30)
}

// ── Validator adapters for enums needing options ────────────────────

fn scoring_profile_validator(v: &Value) -> Result<(), ValidationError> {
    validate_enum(v, &["strict", "balanced", "permissive"])
}

// ── Runtime registry ─────────────────────────────────────────────────

static RUNTIME: &[RuntimeSettingMeta] = &[
    // Metadata
    RuntimeSettingMeta {
        key: "metadata.enabled",
        label: "Metadata Lookups",
        description: "Enable metadata provider lookups",
        section: "metadata",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_true,
        validator: pass,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.contact_email",
        label: "Contact Email",
        description: "Contact email included in User-Agent for API identification",
        section: "metadata",
        value_type: SettingType::OptionalString,
        apply_mode: ApplyMode::PerUse,
        default: d_null,
        validator: pass,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.auto_apply_threshold",
        label: "Auto-Apply Threshold",
        description: "Automatically apply a resolved candidate when its score meets this threshold",
        section: "metadata",
        value_type: SettingType::Float,
        apply_mode: ApplyMode::PerUse,
        default: d_auto_apply_threshold,
        validator: unit_interval,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.max_concurrent_resolutions",
        label: "Max Concurrent Resolutions",
        description: "Maximum concurrent metadata resolution tasks",
        section: "metadata",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::PerUse,
        default: d_max_concurrent_resolutions,
        validator: concurrency_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.scoring_profile",
        label: "Scoring Profile",
        description:
            "How strictly to score embedded metadata quality. Strict: ISBN required for Identified. \
             Balanced: rich metadata can reach Identified. Permissive: trusts embedded metadata more.",
        section: "metadata",
        value_type: SettingType::Select,
        apply_mode: ApplyMode::PerUse,
        default: d_scoring_profile,
        validator: scoring_profile_validator,
        sensitive: false,
        options: Some(&["strict", "balanced", "permissive"]),
    },
    // Open Library
    RuntimeSettingMeta {
        key: "metadata.open_library.enabled",
        label: "Enabled",
        description: "Whether Open Library lookups are enabled",
        section: "metadata.open_library",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_true,
        validator: pass,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.open_library.max_requests_per_minute",
        label: "Rate Limit",
        description: "Maximum requests per minute to Open Library",
        section: "metadata.open_library",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::Subscribed,
        default: d_ol_rpm,
        validator: rpm_range,
        sensitive: false,
        options: None,
    },
    // Hardcover
    RuntimeSettingMeta {
        key: "metadata.hardcover.enabled",
        label: "Enabled",
        description: "Whether Hardcover lookups are enabled",
        section: "metadata.hardcover",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_false,
        validator: pass,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.hardcover.api_token",
        label: "API Token",
        description: "Bearer token for the Hardcover GraphQL API",
        section: "metadata.hardcover",
        value_type: SettingType::OptionalString,
        apply_mode: ApplyMode::PerUse,
        default: d_null,
        validator: pass,
        sensitive: true,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.hardcover.max_requests_per_minute",
        label: "Rate Limit",
        description: "Maximum requests per minute to Hardcover",
        section: "metadata.hardcover",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::Subscribed,
        default: d_hc_rpm,
        validator: rpm_range,
        sensitive: false,
        options: None,
    },
    // Library of Congress
    RuntimeSettingMeta {
        key: "metadata.loc.enabled",
        label: "Enabled",
        description: "Whether Library of Congress lookups are enabled",
        section: "metadata.loc",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_true,
        validator: pass,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "metadata.loc.max_requests_per_minute",
        label: "Rate Limit",
        description:
            "Maximum requests per minute to Library of Congress (strict: exceeding may cause a 1-hour IP ban)",
        section: "metadata.loc",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::Subscribed,
        default: d_loc_rpm,
        validator: rpm_range,
        sensitive: false,
        options: None,
    },
    // Import
    RuntimeSettingMeta {
        key: "import.auto_link_formats",
        label: "Auto-link multiple formats",
        description:
            "When importing a file whose title and author closely match an existing book in a \
             different format, automatically attach it as an additional format instead of creating \
             a separate book entry. Disable if auto-linking produces incorrect matches for your library.",
        section: "import",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_true,
        validator: pass,
        sensitive: false,
        options: None,
    },
    // ISBN scan
    RuntimeSettingMeta {
        key: "isbn_scan.scan_on_import",
        label: "Scan on Import",
        description: "Automatically scan imported books for ISBNs in their content",
        section: "isbn_scan",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_true,
        validator: pass,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.confidence",
        label: "Confidence",
        description: "Confidence value assigned to ISBNs found via content scanning (0.0-1.0)",
        section: "isbn_scan",
        value_type: SettingType::Float,
        apply_mode: ApplyMode::PerUse,
        default: d_confidence,
        validator: unit_interval,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.skip_threshold",
        label: "Skip Threshold",
        description: "Skip scanning if any existing ISBN has confidence >= this threshold",
        section: "isbn_scan",
        value_type: SettingType::Float,
        apply_mode: ApplyMode::PerUse,
        default: d_skip_threshold,
        validator: unit_interval,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.epub_spine_items",
        label: "EPUB Spine Items",
        description: "Number of EPUB spine items to read from front and back",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::PerUse,
        default: d_epub_spine,
        validator: small_count_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.pdf_pages",
        label: "PDF Pages",
        description: "Number of PDF pages to read from front and back",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::PerUse,
        default: d_pdf_pages,
        validator: small_count_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.fb2_sections",
        label: "FB2 Sections",
        description: "Number of FB2 sections to read from front and back",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::PerUse,
        default: d_fb2_sections,
        validator: small_count_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.txt_bytes",
        label: "TXT Bytes",
        description: "Bytes to read from front and back of TXT files",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::PerUse,
        default: d_txt_bytes,
        validator: bytes_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "isbn_scan.mobi_bytes",
        label: "MOBI/AZW3 Bytes",
        description: "Bytes to read from front and back of MOBI/AZW3 text",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::PerUse,
        default: d_mobi_bytes,
        validator: bytes_range,
        sensitive: false,
        options: None,
    },
    // Watcher (boot-frozen / restart required)
    RuntimeSettingMeta {
        key: "watcher.debounce_ms",
        label: "Debounce (ms)",
        description:
            "Window during which rapid filesystem events collapse into a single change, in milliseconds",
        section: "watcher",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::RestartRequired,
        default: d_debounce_ms,
        validator: debounce_ms_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "watcher.default_poll_interval_secs",
        label: "Default Poll Interval (s)",
        description:
            "Fallback interval for polling watched directories on filesystems where native events aren't available",
        section: "watcher",
        value_type: SettingType::Integer,
        apply_mode: ApplyMode::RestartRequired,
        default: d_poll_interval,
        validator: poll_interval_range,
        sensitive: false,
        options: None,
    },
    RuntimeSettingMeta {
        key: "watcher.delete_source_after_import",
        label: "Delete Source After Import",
        description: "After a successful import from a watched directory, delete the source file",
        section: "watcher",
        value_type: SettingType::Bool,
        apply_mode: ApplyMode::PerUse,
        default: d_false,
        validator: pass,
        sensitive: false,
        options: None,
    },
];

// ── Bootstrap registry ───────────────────────────────────────────────

static BOOTSTRAP: &[BootstrapSettingMeta] = &[
    BootstrapSettingMeta {
        key: "listen_address",
        label: "Listen Address",
        description: "Address to bind the HTTP server to",
        section: "server",
        value_type: SettingType::String,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "port",
        label: "Port",
        description: "Port to bind the HTTP server to",
        section: "server",
        value_type: SettingType::Integer,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "data_dir",
        label: "Data Directory",
        description: "Root directory for application data (database, cache, etc.)",
        section: "server",
        value_type: SettingType::String,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "book_storage_path",
        label: "Book Storage Path",
        description: "Root directory for book file storage",
        section: "server",
        value_type: SettingType::String,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "frontend_dir",
        label: "Frontend Directory",
        description: "Directory containing the built frontend assets to serve",
        section: "server",
        value_type: SettingType::OptionalString,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "log_level",
        label: "Log Level",
        description: "Log level filter (trace, debug, info, warn, error)",
        section: "server",
        value_type: SettingType::Select,
        sensitive: false,
        options: Some(&["trace", "debug", "info", "warn", "error"]),
    },
    BootstrapSettingMeta {
        key: "watcher.enabled",
        label: "Filesystem Watcher",
        description:
            "Enable the filesystem watcher subsystem. Other watcher settings live at runtime.",
        section: "server",
        value_type: SettingType::Bool,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "auth.proxy.enabled",
        label: "Proxy Authentication",
        description: "Enable reverse proxy (ForwardAuth) authentication",
        section: "auth",
        value_type: SettingType::Bool,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "auth.proxy.trusted_proxies",
        label: "Trusted Proxies",
        description: "Comma-separated list of trusted proxy IP addresses or CIDR ranges",
        section: "auth",
        value_type: SettingType::String,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "auth.proxy.user_header",
        label: "User Header",
        description: "HTTP header containing the authenticated username",
        section: "auth",
        value_type: SettingType::String,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "auth.proxy.email_header",
        label: "Email Header",
        description: "HTTP header containing the user's email address",
        section: "auth",
        value_type: SettingType::OptionalString,
        sensitive: false,
        options: None,
    },
    BootstrapSettingMeta {
        key: "auth.proxy.groups_header",
        label: "Groups Header",
        description: "HTTP header containing comma-separated group names",
        section: "auth",
        value_type: SettingType::OptionalString,
        sensitive: false,
        options: None,
    },
];

/// Canonical key for a potentially-legacy key (for future rename compat).
pub fn canonical_setting_key(key: &str) -> &str {
    key
}

/// Any legacy key aliases that should resolve to the given canonical key.
pub fn legacy_setting_keys(key: &str) -> &'static [&'static str] {
    let _ = key;
    &[]
}

/// Compute the default value for a runtime key.
pub fn runtime_default(meta: &RuntimeSettingMeta) -> SettingValue {
    (meta.default)()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::value::canonicalize;

    #[test]
    fn every_runtime_default_canonicalizes_and_validates() {
        for meta in runtime_settings() {
            let d = (meta.default)();
            let c = canonicalize(meta.value_type, &d).unwrap_or_else(|e| {
                panic!("default for {} fails canonicalize: {e}", meta.key);
            });
            (meta.validator)(&c).unwrap_or_else(|e| {
                panic!("default for {} fails validator: {e}", meta.key);
            });
        }
    }

    #[test]
    fn no_duplicate_keys_across_scopes() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for m in BOOTSTRAP {
            assert!(seen.insert(m.key), "duplicate key: {}", m.key);
        }
        for m in RUNTIME {
            assert!(seen.insert(m.key), "duplicate key: {}", m.key);
        }
    }

    #[test]
    fn requires_restart_derivation() {
        for m in RUNTIME {
            assert_eq!(
                m.requires_restart(),
                matches!(m.apply_mode, ApplyMode::RestartRequired),
                "{}",
                m.key
            );
        }
    }
}
