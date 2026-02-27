pub mod fs_detect;
pub mod service;

use std::path::PathBuf;

pub use fs_detect::{detect_fs_type, FsDetectionResult, NativeSupport};
pub use service::WatcherService;

/// Events emitted by the watcher service after filtering and normalization.
///
/// Shared between the watcher service (producer) and the event processor (consumer).
#[derive(Debug, Clone)]
pub enum WatcherEvent {
    /// A new file was created or an existing file was modified.
    /// (Create and Modify are merged — both trigger an import attempt.
    /// The import pipeline handles dedup via hash.)
    FileChanged { path: PathBuf },
    /// A file was removed.
    FileRemoved { path: PathBuf },
    /// A watcher error occurred (e.g., permission denied, path disappeared).
    Error {
        error: String,
        path: Option<PathBuf>,
    },
}
