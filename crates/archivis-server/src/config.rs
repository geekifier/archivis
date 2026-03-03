use std::path::PathBuf;

use archivis_core::models::ScoringProfile;
use clap::Parser;
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

/// Command-line arguments for the Archivis server.
#[derive(Debug, Parser)]
#[command(
    name = "archivis",
    about = "A modern, self-hosted e-book collection manager",
    version = env!("ARCHIVIS_VERSION")
)]
pub struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, env = "ARCHIVIS_CONFIG", default_value = "config.toml")]
    pub config: PathBuf,

    /// Address to bind the HTTP server to.
    #[arg(long)]
    pub listen_address: Option<String>,

    /// Port to bind the HTTP server to.
    #[arg(short, long)]
    pub port: Option<u16>,

    /// Directory for application data (database, cache, etc.).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Directory for book file storage (defaults to `data_dir`/books).
    #[arg(long)]
    pub book_storage_path: Option<PathBuf>,

    /// Directory containing the built frontend assets to serve.
    #[arg(long)]
    pub frontend_dir: Option<PathBuf>,

    /// Log level filter (trace, debug, info, warn, error).
    #[arg(long)]
    pub log_level: Option<String>,
}

/// Application configuration.
///
/// Settings are divided into two scopes (see `SettingScope`):
///
/// **Bootstrap** (server, paths, logging): loaded from compiled defaults → TOML
/// file → env vars → CLI flags. Read-only in the admin UI.
///
/// **Runtime** (metadata, ISBN scan): loaded from compiled defaults → DB (admin
/// UI). Env vars and CLI flags can still override for deployment purposes, but
/// the TOML file is **not** consulted for runtime keys.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    /// Address to bind the HTTP server to.
    pub listen_address: String,
    /// Port to bind the HTTP server to.
    pub port: u16,
    /// Root directory for application data (database, cache, etc.).
    pub data_dir: PathBuf,
    /// Root directory for book file storage.
    pub book_storage_path: PathBuf,
    /// Directory containing the built frontend assets to serve.
    /// When set and the directory exists, the server serves static files
    /// from this path and falls back to `index.html` for SPA routing.
    #[serde(default)]
    pub frontend_dir: Option<PathBuf>,
    /// Log level filter string (supports `tracing` directives).
    pub log_level: String,
    /// Metadata provider configuration.
    #[serde(default)]
    pub metadata: MetadataConfig,
    /// ISBN content-scan configuration.
    #[serde(default)]
    pub isbn_scan: IsbnScanConfig,
    /// Filesystem watcher configuration.
    #[serde(default)]
    pub watcher: WatcherConfig,
    /// Import behavior configuration.
    #[serde(default)]
    pub import: ImportAppConfig,
    /// Authentication configuration.
    #[serde(default)]
    pub auth: AuthAppConfig,
}

/// Configuration for metadata provider lookups.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct MetadataConfig {
    /// Enable metadata provider lookups.
    pub enabled: bool,
    /// Contact email included in User-Agent for API identification.
    /// Recommended for Open Library (triples rate limit).
    pub contact_email: Option<String>,
    /// Open Library provider settings.
    pub open_library: OpenLibraryConfig,
    /// Hardcover provider settings.
    pub hardcover: HardcoverConfig,
    /// Auto-identify books after import when confidence is below this threshold.
    pub auto_identify_threshold: f32,
    /// Maximum concurrent identification tasks.
    pub max_concurrent_identifies: usize,
    /// How strictly to score embedded metadata quality.
    pub scoring_profile: ScoringProfile,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            contact_email: None,
            open_library: OpenLibraryConfig::default(),
            hardcover: HardcoverConfig::default(),
            auto_identify_threshold: 0.85,
            max_concurrent_identifies: 2,
            scoring_profile: ScoringProfile::default(),
        }
    }
}

/// Open Library provider settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct OpenLibraryConfig {
    /// Whether Open Library lookups are enabled.
    pub enabled: bool,
    /// Maximum requests per minute (default: 100).
    pub max_requests_per_minute: u32,
}

impl Default for OpenLibraryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_requests_per_minute: 100,
        }
    }
}

/// Hardcover provider settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HardcoverConfig {
    /// Whether Hardcover lookups are enabled.
    pub enabled: bool,
    /// Bearer token for the Hardcover GraphQL API.
    pub api_token: Option<String>,
    /// Maximum requests per minute (default: 50).
    pub max_requests_per_minute: u32,
}

impl Default for HardcoverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_token: None,
            max_requests_per_minute: 50,
        }
    }
}

/// Configuration for the filesystem watcher subsystem.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct WatcherConfig {
    /// Master enable/disable for the filesystem watcher subsystem.
    /// Boot-only: controls whether the watcher infrastructure is initialized.
    /// All other watcher settings are managed at runtime via DB/API/UI.
    pub enabled: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Configuration for import behavior.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ImportAppConfig {
    /// Automatically link files as additional formats when a fuzzy title+author
    /// match exists for a different format.
    pub auto_link_formats: bool,
}

impl Default for ImportAppConfig {
    fn default() -> Self {
        Self {
            auto_link_formats: true,
        }
    }
}

/// Configuration for ISBN content-scan feature.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct IsbnScanConfig {
    /// Automatically scan imported books for ISBNs in their content.
    pub scan_on_import: bool,
    /// Confidence value assigned to ISBNs found via content scanning (0.0-1.0).
    pub confidence: f32,
    /// Skip scanning if any existing ISBN has confidence >= this threshold.
    pub skip_threshold: f32,
    /// Number of EPUB spine items to read from front and back.
    pub epub_spine_items: usize,
    /// Number of PDF pages to read from front and back.
    pub pdf_pages: usize,
    /// Number of FB2 sections to read from front and back.
    pub fb2_sections: usize,
    /// Bytes to read from front and back of TXT files.
    pub txt_bytes: usize,
    /// Bytes to read from front and back of MOBI/AZW3 text.
    pub mobi_bytes: usize,
}

impl Default for IsbnScanConfig {
    fn default() -> Self {
        Self {
            scan_on_import: true,
            confidence: 0.5,
            skip_threshold: 0.95,
            epub_spine_items: 5,
            pdf_pages: 5,
            fb2_sections: 3,
            txt_bytes: 4000,
            mobi_bytes: 8000,
        }
    }
}

/// Top-level authentication configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AuthAppConfig {
    /// Reverse proxy (`ForwardAuth`) settings.
    pub proxy: ProxyAuthConfig,
}

/// Reverse proxy (`ForwardAuth`) authentication configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProxyAuthConfig {
    /// Enable reverse proxy authentication.
    pub enabled: bool,
    /// List of trusted proxy IP addresses or CIDR ranges.
    pub trusted_proxies: Vec<String>,
    /// Header containing the authenticated username.
    pub user_header: String,
    /// Header containing the user's email address.
    pub email_header: Option<String>,
    /// Header containing comma-separated group names.
    pub groups_header: Option<String>,
}

impl Default for ProxyAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trusted_proxies: Vec::new(),
            user_header: "X-Forwarded-User".to_owned(),
            email_header: Some("X-Forwarded-Email".to_owned()),
            groups_header: Some("X-Forwarded-Groups".to_owned()),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_owned(),
            port: 9514,
            data_dir: PathBuf::from("data"),
            // Empty sentinel — resolved to {data_dir}/books in resolve_derived_defaults
            book_storage_path: PathBuf::new(),
            frontend_dir: None,
            log_level: "info".to_owned(),
            metadata: MetadataConfig::default(),
            isbn_scan: IsbnScanConfig::default(),
            watcher: WatcherConfig::default(),
            import: ImportAppConfig::default(),
            auth: AuthAppConfig::default(),
        }
    }
}

/// Partial config from CLI flags. Fields that are `None` are omitted during
/// serialization so they don't override values from lower-priority sources.
#[derive(Debug, Serialize)]
struct CliOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    listen_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    book_storage_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frontend_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_level: Option<String>,
}

impl AppConfig {
    /// Load configuration from all available sources.
    ///
    /// Priority: CLI flags > environment variables > TOML config file > defaults.
    /// A missing config file is not an error (defaults and env vars are sufficient).
    pub fn load(cli: &Cli) -> Result<Self, Box<figment::Error>> {
        let mut figment = Figment::from(Serialized::defaults(Self::default()));

        if cli.config.exists() {
            figment = figment.merge(Toml::file(&cli.config));
        }

        let overrides = CliOverrides {
            listen_address: cli.listen_address.clone(),
            port: cli.port,
            data_dir: cli.data_dir.clone(),
            book_storage_path: cli.book_storage_path.clone(),
            frontend_dir: cli.frontend_dir.clone(),
            log_level: cli.log_level.clone(),
        };

        figment = figment
            .merge(Env::prefixed("ARCHIVIS_").split("__"))
            .merge(Serialized::defaults(overrides));

        let mut config: Self = figment.extract()?;
        config.resolve_derived_defaults();
        Ok(config)
    }

    /// Resolve defaults that depend on other config values.
    pub fn resolve_derived_defaults(&mut self) {
        if self.book_storage_path.as_os_str().is_empty() {
            self.book_storage_path = self.data_dir.join("books");
        }
    }

    /// Create required directories if they don't already exist.
    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.book_storage_path)?;
        Ok(())
    }

    /// The socket address string the server should bind to.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.listen_address, self.port)
    }
}

// ── Config flattening and source detection ──────────────────────

use std::collections::HashMap;

use archivis_api::settings::registry::SettingScope;
use archivis_api::settings::service::{ConfigOverride, ConfigSource};

/// Flatten an `AppConfig` into a `HashMap<String, serde_json::Value>` with dotted keys.
pub fn flatten_config(config: &AppConfig) -> HashMap<String, serde_json::Value> {
    let value = serde_json::to_value(config).expect("AppConfig must be serializable");
    let mut map = HashMap::new();
    flatten_value(&value, "", &mut map);

    // Post-process: convert array fields to comma-separated strings for the
    // settings API (which represents them as `SettingType::String`).
    if let Some(arr) = map.remove("auth.proxy.trusted_proxies") {
        let csv = arr
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        map.insert(
            "auth.proxy.trusted_proxies".to_string(),
            serde_json::Value::String(csv),
        );
    }

    map
}

fn flatten_value(
    value: &serde_json::Value,
    prefix: &str,
    map: &mut HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (k, v) in obj {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_value(v, &key, map);
            }
        }
        _ => {
            map.insert(prefix.to_string(), value.clone());
        }
    }
}

/// Detect which env vars override settings. Returns only keys that have active overrides.
#[allow(clippy::too_many_lines)]
pub fn detect_env_overrides(cli: &Cli) -> HashMap<String, ConfigOverride> {
    let mut overrides = HashMap::new();

    // Map of setting keys to their environment variable names
    let env_mappings: &[(&str, &str)] = &[
        ("listen_address", "ARCHIVIS_LISTEN_ADDRESS"),
        ("port", "ARCHIVIS_PORT"),
        ("data_dir", "ARCHIVIS_DATA_DIR"),
        ("book_storage_path", "ARCHIVIS_BOOK_STORAGE_PATH"),
        ("frontend_dir", "ARCHIVIS_FRONTEND_DIR"),
        ("log_level", "ARCHIVIS_LOG_LEVEL"),
        ("metadata.enabled", "ARCHIVIS_METADATA__ENABLED"),
        ("metadata.contact_email", "ARCHIVIS_METADATA__CONTACT_EMAIL"),
        (
            "metadata.auto_identify_threshold",
            "ARCHIVIS_METADATA__AUTO_IDENTIFY_THRESHOLD",
        ),
        (
            "metadata.max_concurrent_identifies",
            "ARCHIVIS_METADATA__MAX_CONCURRENT_IDENTIFIES",
        ),
        (
            "metadata.scoring_profile",
            "ARCHIVIS_METADATA__SCORING_PROFILE",
        ),
        (
            "metadata.open_library.enabled",
            "ARCHIVIS_METADATA__OPEN_LIBRARY__ENABLED",
        ),
        (
            "metadata.open_library.max_requests_per_minute",
            "ARCHIVIS_METADATA__OPEN_LIBRARY__MAX_REQUESTS_PER_MINUTE",
        ),
        (
            "metadata.hardcover.enabled",
            "ARCHIVIS_METADATA__HARDCOVER__ENABLED",
        ),
        (
            "metadata.hardcover.api_token",
            "ARCHIVIS_METADATA__HARDCOVER__API_TOKEN",
        ),
        (
            "metadata.hardcover.max_requests_per_minute",
            "ARCHIVIS_METADATA__HARDCOVER__MAX_REQUESTS_PER_MINUTE",
        ),
        (
            "isbn_scan.scan_on_import",
            "ARCHIVIS_ISBN_SCAN__SCAN_ON_IMPORT",
        ),
        ("isbn_scan.confidence", "ARCHIVIS_ISBN_SCAN__CONFIDENCE"),
        (
            "isbn_scan.skip_threshold",
            "ARCHIVIS_ISBN_SCAN__SKIP_THRESHOLD",
        ),
        (
            "isbn_scan.epub_spine_items",
            "ARCHIVIS_ISBN_SCAN__EPUB_SPINE_ITEMS",
        ),
        ("isbn_scan.pdf_pages", "ARCHIVIS_ISBN_SCAN__PDF_PAGES"),
        ("isbn_scan.fb2_sections", "ARCHIVIS_ISBN_SCAN__FB2_SECTIONS"),
        ("isbn_scan.txt_bytes", "ARCHIVIS_ISBN_SCAN__TXT_BYTES"),
        ("isbn_scan.mobi_bytes", "ARCHIVIS_ISBN_SCAN__MOBI_BYTES"),
        ("watcher.enabled", "ARCHIVIS_WATCHER__ENABLED"),
        (
            "import.auto_link_formats",
            "ARCHIVIS_IMPORT__AUTO_LINK_FORMATS",
        ),
        ("auth.proxy.enabled", "ARCHIVIS_AUTH__PROXY__ENABLED"),
        (
            "auth.proxy.trusted_proxies",
            "ARCHIVIS_AUTH__PROXY__TRUSTED_PROXIES",
        ),
        (
            "auth.proxy.user_header",
            "ARCHIVIS_AUTH__PROXY__USER_HEADER",
        ),
        (
            "auth.proxy.email_header",
            "ARCHIVIS_AUTH__PROXY__EMAIL_HEADER",
        ),
        (
            "auth.proxy.groups_header",
            "ARCHIVIS_AUTH__PROXY__GROUPS_HEADER",
        ),
    ];

    for &(key, env_var) in env_mappings {
        if std::env::var(env_var).is_ok() {
            overrides.insert(
                key.to_string(),
                ConfigOverride {
                    source: ConfigSource::Env,
                    env_var: Some(env_var.to_string()),
                },
            );
        }
    }

    // CLI overrides: detect by checking if the Option fields were provided
    let cli_mappings: &[(&str, bool)] = &[
        ("listen_address", cli.listen_address.is_some()),
        ("port", cli.port.is_some()),
        ("data_dir", cli.data_dir.is_some()),
        ("book_storage_path", cli.book_storage_path.is_some()),
        ("frontend_dir", cli.frontend_dir.is_some()),
        ("log_level", cli.log_level.is_some()),
    ];

    for &(key, present) in cli_mappings {
        if present {
            overrides.insert(
                key.to_string(),
                ConfigOverride {
                    source: ConfigSource::Cli,
                    env_var: None,
                },
            );
        }
    }

    overrides
}

/// Detect the configured source (default vs file vs database) for each setting.
///
/// The logic branches on `SettingScope`:
/// - **Bootstrap** keys: compare file values vs defaults → `File` if different, then
///   check DB → `Database`, else `Default`.
/// - **Runtime** keys: check DB → `Database`, else `Default` — never `File`.
pub fn detect_configured_sources(
    default_flat: &HashMap<String, serde_json::Value>,
    file_flat: &HashMap<String, serde_json::Value>,
    db_keys: &[String],
) -> HashMap<String, ConfigSource> {
    let mut sources = HashMap::new();

    for meta in archivis_api::settings::registry::all_settings() {
        let source = match meta.scope {
            SettingScope::Bootstrap => {
                if db_keys.contains(&meta.key.to_string()) {
                    ConfigSource::Database
                } else if file_flat.get(meta.key) != default_flat.get(meta.key) {
                    ConfigSource::File
                } else {
                    ConfigSource::Default
                }
            }
            SettingScope::Runtime => {
                if db_keys.contains(&meta.key.to_string()) {
                    ConfigSource::Database
                } else {
                    ConfigSource::Default
                }
            }
        };
        sources.insert(meta.key.to_string(), source);
    }

    sources
}

/// Apply DB settings as overrides to an `AppConfig` by updating fields directly.
pub fn apply_db_settings(config: &mut AppConfig, db_settings: &[(String, String)]) {
    for (key, json_value) in db_settings {
        let Ok(value) = serde_json::from_str(json_value) else {
            continue;
        };

        apply_setting_to_config(config, key, &value);
    }
    config.resolve_derived_defaults();
}

/// Apply a single setting value to the config struct.
#[allow(clippy::too_many_lines)]
fn apply_setting_to_config(config: &mut AppConfig, key: &str, value: &serde_json::Value) {
    match key {
        "listen_address" => {
            if let Some(s) = value.as_str() {
                config.listen_address = s.to_string();
            }
        }
        "port" => {
            if let Some(n) = value.as_u64() {
                if let Ok(p) = u16::try_from(n) {
                    config.port = p;
                }
            }
        }
        "data_dir" => {
            if let Some(s) = value.as_str() {
                config.data_dir = PathBuf::from(s);
            }
        }
        "book_storage_path" => {
            if let Some(s) = value.as_str() {
                config.book_storage_path = PathBuf::from(s);
            }
        }
        "frontend_dir" => match value {
            serde_json::Value::Null => config.frontend_dir = None,
            serde_json::Value::String(s) => config.frontend_dir = Some(PathBuf::from(s)),
            _ => {}
        },
        "log_level" => {
            if let Some(s) = value.as_str() {
                config.log_level = s.to_string();
            }
        }
        "metadata.enabled" => {
            if let Some(b) = value.as_bool() {
                config.metadata.enabled = b;
            }
        }
        "metadata.contact_email" => match value {
            serde_json::Value::Null => config.metadata.contact_email = None,
            serde_json::Value::String(s) => config.metadata.contact_email = Some(s.clone()),
            _ => {}
        },
        "metadata.auto_identify_threshold" => {
            if let Some(f) = value.as_f64() {
                #[allow(clippy::cast_possible_truncation)]
                {
                    config.metadata.auto_identify_threshold = f as f32;
                }
            }
        }
        "metadata.max_concurrent_identifies" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = usize::try_from(n) {
                    config.metadata.max_concurrent_identifies = v;
                }
            }
        }
        "metadata.scoring_profile" => {
            if let Some(s) = value.as_str() {
                if let Ok(profile) = s.parse::<ScoringProfile>() {
                    config.metadata.scoring_profile = profile;
                }
            }
        }
        "metadata.open_library.enabled" => {
            if let Some(b) = value.as_bool() {
                config.metadata.open_library.enabled = b;
            }
        }
        "metadata.open_library.max_requests_per_minute" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = u32::try_from(n) {
                    config.metadata.open_library.max_requests_per_minute = v;
                }
            }
        }
        "metadata.hardcover.enabled" => {
            if let Some(b) = value.as_bool() {
                config.metadata.hardcover.enabled = b;
            }
        }
        "metadata.hardcover.api_token" => match value {
            serde_json::Value::Null => config.metadata.hardcover.api_token = None,
            serde_json::Value::String(s) => {
                config.metadata.hardcover.api_token = Some(s.clone());
            }
            _ => {}
        },
        "metadata.hardcover.max_requests_per_minute" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = u32::try_from(n) {
                    config.metadata.hardcover.max_requests_per_minute = v;
                }
            }
        }
        "isbn_scan.scan_on_import" => {
            if let Some(b) = value.as_bool() {
                config.isbn_scan.scan_on_import = b;
            }
        }
        "isbn_scan.confidence" => {
            if let Some(f) = value.as_f64() {
                #[allow(clippy::cast_possible_truncation)]
                {
                    config.isbn_scan.confidence = f as f32;
                }
            }
        }
        "isbn_scan.skip_threshold" => {
            if let Some(f) = value.as_f64() {
                #[allow(clippy::cast_possible_truncation)]
                {
                    config.isbn_scan.skip_threshold = f as f32;
                }
            }
        }
        "isbn_scan.epub_spine_items" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = usize::try_from(n) {
                    config.isbn_scan.epub_spine_items = v;
                }
            }
        }
        "isbn_scan.pdf_pages" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = usize::try_from(n) {
                    config.isbn_scan.pdf_pages = v;
                }
            }
        }
        "isbn_scan.fb2_sections" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = usize::try_from(n) {
                    config.isbn_scan.fb2_sections = v;
                }
            }
        }
        "isbn_scan.txt_bytes" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = usize::try_from(n) {
                    config.isbn_scan.txt_bytes = v;
                }
            }
        }
        "isbn_scan.mobi_bytes" => {
            if let Some(n) = value.as_u64() {
                if let Ok(v) = usize::try_from(n) {
                    config.isbn_scan.mobi_bytes = v;
                }
            }
        }
        "watcher.enabled" => {
            if let Some(b) = value.as_bool() {
                config.watcher.enabled = b;
            }
        }
        "import.auto_link_formats" => {
            if let Some(b) = value.as_bool() {
                config.import.auto_link_formats = b;
            }
        }
        "auth.proxy.enabled" => {
            if let Some(b) = value.as_bool() {
                config.auth.proxy.enabled = b;
            }
        }
        "auth.proxy.trusted_proxies" => {
            // Stored as a JSON array of strings or a comma-separated string
            if let Some(arr) = value.as_array() {
                config.auth.proxy.trusted_proxies = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect();
            } else if let Some(s) = value.as_str() {
                config.auth.proxy.trusted_proxies = s
                    .split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect();
            }
        }
        "auth.proxy.user_header" => {
            if let Some(s) = value.as_str() {
                config.auth.proxy.user_header = s.to_string();
            }
        }
        "auth.proxy.email_header" => match value {
            serde_json::Value::Null => config.auth.proxy.email_header = None,
            serde_json::Value::String(s) => {
                config.auth.proxy.email_header = Some(s.clone());
            }
            _ => {}
        },
        "auth.proxy.groups_header" => match value {
            serde_json::Value::Null => config.auth.proxy.groups_header = None,
            serde_json::Value::String(s) => {
                config.auth.proxy.groups_header = Some(s.clone());
            }
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = AppConfig::default();
        assert_eq!(config.listen_address, "127.0.0.1");
        assert_eq!(config.port, 9514);
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert!(config.frontend_dir.is_none());
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn book_storage_defaults_to_data_dir_books() {
        let mut config = AppConfig::default();
        config.resolve_derived_defaults();
        assert_eq!(config.book_storage_path, PathBuf::from("data/books"));
    }

    #[test]
    fn explicit_book_storage_is_preserved() {
        let mut config = AppConfig {
            book_storage_path: PathBuf::from("/custom/books"),
            ..AppConfig::default()
        };
        config.resolve_derived_defaults();
        assert_eq!(config.book_storage_path, PathBuf::from("/custom/books"));
    }

    #[test]
    fn custom_data_dir_changes_default_book_storage() {
        let mut config = AppConfig {
            data_dir: PathBuf::from("/srv/archivis"),
            ..AppConfig::default()
        };
        config.resolve_derived_defaults();
        assert_eq!(
            config.book_storage_path,
            PathBuf::from("/srv/archivis/books")
        );
    }

    #[test]
    fn bind_address_format() {
        let config = AppConfig::default();
        assert_eq!(config.bind_address(), "127.0.0.1:9514");
    }

    #[test]
    fn load_from_defaults_only() {
        let cli = Cli::parse_from(["archivis", "--config", "/nonexistent/config.toml"]);
        let config = AppConfig::load(&cli).expect("should load from defaults");
        assert_eq!(config.listen_address, "127.0.0.1");
        assert_eq!(config.port, 9514);
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert_eq!(config.book_storage_path, PathBuf::from("data/books"));
        assert!(config.frontend_dir.is_none());
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn cli_flags_override_defaults() {
        let cli = Cli::parse_from([
            "archivis",
            "--config",
            "/nonexistent/config.toml",
            "--listen-address",
            "0.0.0.0",
            "--port",
            "3000",
            "--data-dir",
            "/tmp/archivis",
            "--book-storage-path",
            "/mnt/books",
            "--frontend-dir",
            "/srv/frontend/dist",
            "--log-level",
            "debug",
        ]);
        let config = AppConfig::load(&cli).expect("should load with CLI overrides");
        assert_eq!(config.listen_address, "0.0.0.0");
        assert_eq!(config.port, 3000);
        assert_eq!(config.data_dir, PathBuf::from("/tmp/archivis"));
        assert_eq!(config.book_storage_path, PathBuf::from("/mnt/books"));
        assert_eq!(
            config.frontend_dir,
            Some(PathBuf::from("/srv/frontend/dist"))
        );
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn toml_config_is_loaded() {
        let dir = std::env::temp_dir().join("archivis-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("test-config.toml");
        std::fs::write(
            &config_path,
            r#"
listen_address = "0.0.0.0"
port = 9090
log_level = "debug"
frontend_dir = "/opt/archivis/frontend"
"#,
        )
        .unwrap();

        let cli = Cli::parse_from(["archivis", "--config", config_path.to_str().unwrap()]);
        let config = AppConfig::load(&cli).expect("should load from TOML");
        assert_eq!(config.listen_address, "0.0.0.0");
        assert_eq!(config.port, 9090);
        assert_eq!(config.log_level, "debug");
        assert_eq!(
            config.frontend_dir,
            Some(PathBuf::from("/opt/archivis/frontend"))
        );
        // data_dir still default since not in TOML
        assert_eq!(config.data_dir, PathBuf::from("data"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_metadata_config_has_sane_defaults() {
        let config = MetadataConfig::default();
        assert!(config.enabled);
        assert!(config.contact_email.is_none());
        assert!(config.open_library.enabled);
        assert_eq!(config.open_library.max_requests_per_minute, 100);
        assert!(!config.hardcover.enabled);
        assert!(config.hardcover.api_token.is_none());
        assert_eq!(config.hardcover.max_requests_per_minute, 50);
        assert!((config.auto_identify_threshold - 0.85).abs() < f32::EPSILON);
        assert_eq!(config.max_concurrent_identifies, 2);
    }

    #[test]
    fn app_config_defaults_include_metadata() {
        let cli = Cli::parse_from(["archivis", "--config", "/nonexistent/config.toml"]);
        let config = AppConfig::load(&cli).expect("should load from defaults");
        assert!(config.metadata.enabled);
        assert!(config.metadata.open_library.enabled);
        assert!(config.metadata.hardcover.api_token.is_none());
    }

    #[test]
    fn detect_configured_sources_uses_scope() {
        use archivis_api::settings::service::ConfigSource;
        let default_flat = flatten_config(&AppConfig::default());
        // Simulate a TOML file that sets both a bootstrap key and a runtime key
        let mut file_flat = default_flat.clone();
        file_flat.insert("port".to_string(), serde_json::json!(9090));
        file_flat.insert("metadata.enabled".to_string(), serde_json::json!(false));

        let db_keys = vec!["isbn_scan.confidence".to_string()];

        let sources = detect_configured_sources(&default_flat, &file_flat, &db_keys);

        // Bootstrap key changed in file → File
        assert_eq!(sources["port"], ConfigSource::File);
        // Runtime key changed in file → still Default (file is ignored for runtime)
        assert_eq!(sources["metadata.enabled"], ConfigSource::Default);
        // Runtime key in DB → Database
        assert_eq!(sources["isbn_scan.confidence"], ConfigSource::Database);
        // Unchanged bootstrap key → Default
        assert_eq!(sources["listen_address"], ConfigSource::Default);
    }

    /// Every setting in the registry must be backed by an `AppConfig` field so
    /// that `flatten_config(AppConfig::default())` returns the correct default
    /// and the API can surface it. This test catches the class of bug where a
    /// setting is added to the registry without a corresponding `AppConfig` field.
    #[test]
    fn all_registry_settings_have_app_config_defaults() {
        use archivis_api::settings::registry::{self, SettingType};

        let flat = flatten_config(&AppConfig::default());
        for meta in registry::all_settings() {
            assert!(
                flat.contains_key(meta.key),
                "Registry setting '{}' has no corresponding field in AppConfig. \
                 Every registered setting must be backed by an AppConfig field \
                 so the API returns its default correctly.",
                meta.key
            );
            let val = &flat[meta.key];
            match meta.value_type {
                SettingType::Bool => assert!(
                    val.is_boolean(),
                    "'{}' default is not a boolean: {}",
                    meta.key,
                    val
                ),
                SettingType::Integer | SettingType::Float => assert!(
                    val.is_number(),
                    "'{}' default is not a number: {}",
                    meta.key,
                    val
                ),
                SettingType::String | SettingType::Select => assert!(
                    val.is_string(),
                    "'{}' default is not a string: {}",
                    meta.key,
                    val
                ),
                SettingType::OptionalString => {} // null or string, both fine
            }
        }
    }
}
