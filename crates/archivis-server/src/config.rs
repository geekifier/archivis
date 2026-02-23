use std::path::PathBuf;

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
    version
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
/// Loaded from (in order of increasing priority):
/// 1. Compiled defaults
/// 2. TOML config file
/// 3. Environment variables (`ARCHIVIS_` prefix)
/// 4. CLI flags
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
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            contact_email: None,
            open_library: OpenLibraryConfig::default(),
            hardcover: HardcoverConfig::default(),
            auto_identify_threshold: 0.6,
            max_concurrent_identifies: 2,
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
            enabled: true,
            api_token: None,
            max_requests_per_minute: 50,
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
}

impl Default for IsbnScanConfig {
    fn default() -> Self {
        Self {
            scan_on_import: false,
            confidence: 0.85,
            skip_threshold: 0.95,
            epub_spine_items: 3,
            pdf_pages: 5,
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
    fn resolve_derived_defaults(&mut self) {
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
        assert!(config.hardcover.enabled);
        assert!(config.hardcover.api_token.is_none());
        assert_eq!(config.hardcover.max_requests_per_minute, 50);
        assert!((config.auto_identify_threshold - 0.6).abs() < f32::EPSILON);
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
    fn toml_metadata_config_is_loaded() {
        let dir = std::env::temp_dir().join("archivis-test-metadata-config");
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("test-metadata.toml");
        std::fs::write(
            &config_path,
            r#"
[metadata]
enabled = true
contact_email = "test@example.com"
auto_identify_threshold = 0.8

[metadata.open_library]
enabled = false
max_requests_per_minute = 50

[metadata.hardcover]
enabled = true
api_token = "test-token-123"
max_requests_per_minute = 30
"#,
        )
        .unwrap();

        let cli = Cli::parse_from(["archivis", "--config", config_path.to_str().unwrap()]);
        let config = AppConfig::load(&cli).expect("should load metadata config from TOML");
        assert!(config.metadata.enabled);
        assert_eq!(
            config.metadata.contact_email.as_deref(),
            Some("test@example.com")
        );
        assert!((config.metadata.auto_identify_threshold - 0.8).abs() < f32::EPSILON);
        assert!(!config.metadata.open_library.enabled);
        assert_eq!(config.metadata.open_library.max_requests_per_minute, 50);
        assert!(config.metadata.hardcover.enabled);
        assert_eq!(
            config.metadata.hardcover.api_token.as_deref(),
            Some("test-token-123")
        );
        assert_eq!(config.metadata.hardcover.max_requests_per_minute, 30);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
