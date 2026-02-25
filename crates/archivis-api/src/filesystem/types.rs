use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Query parameters for browsing a directory.
#[derive(Debug, Deserialize, IntoParams)]
pub struct BrowseParams {
    /// Absolute path to browse. Defaults to the server's data directory if omitted.
    pub path: Option<String>,
    /// When true, only return directory entries.
    #[serde(default)]
    pub dirs_only: bool,
}

/// A single filesystem entry (file or directory).
#[derive(Debug, Serialize, ToSchema)]
pub struct FsEntry {
    /// Entry name (not full path).
    pub name: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// File size in bytes (0 for directories).
    pub size: u64,
}

/// Response from browsing a directory.
#[derive(Debug, Serialize, ToSchema)]
pub struct BrowseResponse {
    /// Canonicalized absolute path of the browsed directory.
    pub path: String,
    /// Parent directory path, if one exists.
    pub parent: Option<String>,
    /// Directory entries sorted: directories first, then alphabetical.
    pub entries: Vec<FsEntry>,
}
