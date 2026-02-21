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
    /// Log level filter string (supports `tracing` directives).
    pub log_level: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_owned(),
            port: 9514,
            data_dir: PathBuf::from("data"),
            // Empty sentinel — resolved to {data_dir}/books in resolve_derived_defaults
            book_storage_path: PathBuf::new(),
            log_level: "info".to_owned(),
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
            log_level: cli.log_level.clone(),
        };

        figment = figment
            .merge(Env::prefixed("ARCHIVIS_"))
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
            "--log-level",
            "debug",
        ]);
        let config = AppConfig::load(&cli).expect("should load with CLI overrides");
        assert_eq!(config.listen_address, "0.0.0.0");
        assert_eq!(config.port, 3000);
        assert_eq!(config.data_dir, PathBuf::from("/tmp/archivis"));
        assert_eq!(config.book_storage_path, PathBuf::from("/mnt/books"));
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
"#,
        )
        .unwrap();

        let cli = Cli::parse_from(["archivis", "--config", config_path.to_str().unwrap()]);
        let config = AppConfig::load(&cli).expect("should load from TOML");
        assert_eq!(config.listen_address, "0.0.0.0");
        assert_eq!(config.port, 9090);
        assert_eq!(config.log_level, "debug");
        // data_dir still default since not in TOML
        assert_eq!(config.data_dir, PathBuf::from("data"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
