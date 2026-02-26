use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    String,
    OptionalString,
    Bool,
    Integer,
    Float,
}

pub struct SettingMeta {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub section: &'static str,
    pub value_type: SettingType,
    pub requires_restart: bool,
    pub sensitive: bool,
}

/// Get the static registry of all known settings.
pub fn all_settings() -> &'static [SettingMeta] {
    SETTINGS
}

/// Look up a setting by key.
pub fn get_setting_meta(key: &str) -> Option<&'static SettingMeta> {
    SETTINGS.iter().find(|s| s.key == key)
}

static SETTINGS: &[SettingMeta] = &[
    // Server settings
    SettingMeta {
        key: "listen_address",
        label: "Listen Address",
        description: "Address to bind the HTTP server to",
        section: "server",
        value_type: SettingType::String,
        requires_restart: true,
        sensitive: false,
    },
    SettingMeta {
        key: "port",
        label: "Port",
        description: "Port to bind the HTTP server to",
        section: "server",
        value_type: SettingType::Integer,
        requires_restart: true,
        sensitive: false,
    },
    SettingMeta {
        key: "data_dir",
        label: "Data Directory",
        description: "Root directory for application data (database, cache, etc.)",
        section: "server",
        value_type: SettingType::String,
        requires_restart: true,
        sensitive: false,
    },
    SettingMeta {
        key: "book_storage_path",
        label: "Book Storage Path",
        description: "Root directory for book file storage",
        section: "server",
        value_type: SettingType::String,
        requires_restart: true,
        sensitive: false,
    },
    SettingMeta {
        key: "frontend_dir",
        label: "Frontend Directory",
        description: "Directory containing the built frontend assets to serve",
        section: "server",
        value_type: SettingType::OptionalString,
        requires_restart: true,
        sensitive: false,
    },
    SettingMeta {
        key: "log_level",
        label: "Log Level",
        description: "Log level filter (trace, debug, info, warn, error)",
        section: "server",
        value_type: SettingType::String,
        requires_restart: true,
        sensitive: false,
    },
    // Metadata settings
    SettingMeta {
        key: "metadata.enabled",
        label: "Metadata Lookups",
        description: "Enable metadata provider lookups",
        section: "metadata",
        value_type: SettingType::Bool,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "metadata.contact_email",
        label: "Contact Email",
        description: "Contact email included in User-Agent for API identification",
        section: "metadata",
        value_type: SettingType::OptionalString,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "metadata.auto_identify_threshold",
        label: "Auto-Identify Threshold",
        description: "Auto-identify books after import when confidence is below this threshold",
        section: "metadata",
        value_type: SettingType::Float,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "metadata.max_concurrent_identifies",
        label: "Max Concurrent Identifies",
        description: "Maximum concurrent identification tasks",
        section: "metadata",
        value_type: SettingType::Integer,
        requires_restart: false,
        sensitive: false,
    },
    // Open Library
    SettingMeta {
        key: "metadata.open_library.enabled",
        label: "Enabled",
        description: "Whether Open Library lookups are enabled",
        section: "metadata.open_library",
        value_type: SettingType::Bool,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "metadata.open_library.max_requests_per_minute",
        label: "Rate Limit",
        description: "Maximum requests per minute to Open Library",
        section: "metadata.open_library",
        value_type: SettingType::Integer,
        requires_restart: false,
        sensitive: false,
    },
    // Hardcover
    SettingMeta {
        key: "metadata.hardcover.enabled",
        label: "Enabled",
        description: "Whether Hardcover lookups are enabled",
        section: "metadata.hardcover",
        value_type: SettingType::Bool,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "metadata.hardcover.api_token",
        label: "API Token",
        description: "Bearer token for the Hardcover GraphQL API",
        section: "metadata.hardcover",
        value_type: SettingType::OptionalString,
        requires_restart: false,
        sensitive: true,
    },
    SettingMeta {
        key: "metadata.hardcover.max_requests_per_minute",
        label: "Rate Limit",
        description: "Maximum requests per minute to Hardcover",
        section: "metadata.hardcover",
        value_type: SettingType::Integer,
        requires_restart: false,
        sensitive: false,
    },
    // ISBN Scan settings
    SettingMeta {
        key: "isbn_scan.scan_on_import",
        label: "Scan on Import",
        description: "Automatically scan imported books for ISBNs in their content",
        section: "isbn_scan",
        value_type: SettingType::Bool,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "isbn_scan.confidence",
        label: "Confidence",
        description: "Confidence value assigned to ISBNs found via content scanning (0.0-1.0)",
        section: "isbn_scan",
        value_type: SettingType::Float,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "isbn_scan.skip_threshold",
        label: "Skip Threshold",
        description: "Skip scanning if any existing ISBN has confidence >= this threshold",
        section: "isbn_scan",
        value_type: SettingType::Float,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "isbn_scan.epub_spine_items",
        label: "EPUB Spine Items",
        description: "Number of EPUB spine items to read from front and back",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        requires_restart: false,
        sensitive: false,
    },
    SettingMeta {
        key: "isbn_scan.pdf_pages",
        label: "PDF Pages",
        description: "Number of PDF pages to read from front and back",
        section: "isbn_scan",
        value_type: SettingType::Integer,
        requires_restart: false,
        sensitive: false,
    },
];

/// Validate a value against the setting's type constraints.
pub fn validate_setting_value(meta: &SettingMeta, value: &serde_json::Value) -> Result<(), String> {
    match meta.value_type {
        SettingType::String => {
            if !value.is_string() {
                return Err(format!("{}: expected a string", meta.key));
            }
        }
        SettingType::OptionalString => {
            // Can be string or null
            if !value.is_string() && !value.is_null() {
                return Err(format!("{}: expected a string or null", meta.key));
            }
        }
        SettingType::Bool => {
            if !value.is_boolean() {
                return Err(format!("{}: expected a boolean", meta.key));
            }
        }
        SettingType::Integer => {
            let Some(n) = value.as_i64() else {
                return Err(format!("{}: expected an integer", meta.key));
            };
            // Key-specific range validation
            match meta.key {
                "port" => {
                    if !(1..=65535).contains(&n) {
                        return Err(format!("{}: must be between 1 and 65535", meta.key));
                    }
                }
                k if k.contains("max_requests_per_minute") => {
                    if !(1..=10000).contains(&n) {
                        return Err(format!("{}: must be between 1 and 10000", meta.key));
                    }
                }
                k if k.contains("max_concurrent") => {
                    if !(1..=100).contains(&n) {
                        return Err(format!("{}: must be between 1 and 100", meta.key));
                    }
                }
                "isbn_scan.epub_spine_items" | "isbn_scan.pdf_pages" => {
                    if !(1..=100).contains(&n) {
                        return Err(format!("{}: must be between 1 and 100", meta.key));
                    }
                }
                _ => {}
            }
        }
        SettingType::Float => {
            let Some(f) = value.as_f64() else {
                return Err(format!("{}: expected a number", meta.key));
            };
            // Key-specific range validation
            match meta.key {
                k if k.contains("threshold") || k == "isbn_scan.confidence" => {
                    if !(0.0..=1.0).contains(&f) {
                        return Err(format!("{}: must be between 0.0 and 1.0", meta.key));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
