use std::collections::HashMap;
use std::path::{Path, PathBuf};

use archivis_core::models::BookFormat;
use archivis_storage::StorageBackend;
use tracing::{debug, info, warn};

use super::progress::{
    BulkImportResult, FailedFile, FileOutcome, ImportProgress, SkipReason, SkippedFile,
};
use super::service::ImportService;
use super::types::{DuplicateInfo, ImportError};

/// Supported file extensions for ebook import.
const SUPPORTED_EXTENSIONS: &[&str] = &["epub", "pdf", "mobi", "azw3", "cbz", "fb2", "txt", "djvu"];

/// Manifest returned from scanning a directory (fast, no heavy processing).
#[derive(Debug)]
pub struct ImportManifest {
    pub total_files: usize,
    pub total_size: u64,
    pub by_format: HashMap<BookFormat, FormatCount>,
    pub files: Vec<ManifestEntry>,
}

/// Count and total size for a single format.
#[derive(Debug)]
pub struct FormatCount {
    pub count: usize,
    pub total_size: u64,
}

/// A single file entry in an import manifest.
#[derive(Debug)]
pub struct ManifestEntry {
    pub path: PathBuf,
    pub format: BookFormat,
    pub size: u64,
}

/// Service for scanning directories and bulk-importing ebook files.
pub struct BulkImportService<S: StorageBackend> {
    import_service: ImportService<S>,
}

impl<S: StorageBackend> BulkImportService<S> {
    pub fn new(import_service: ImportService<S>) -> Self {
        Self { import_service }
    }

    /// Scan a directory tree and build a manifest of importable files.
    ///
    /// This is a fast operation that detects formats via magic bytes without
    /// performing any heavy processing like metadata extraction or hashing.
    pub async fn scan_directory(&self, path: &Path) -> Result<ImportManifest, ImportError> {
        if !path.is_dir() {
            return Err(ImportError::InvalidFile(format!(
                "path is not a directory: {}",
                path.display()
            )));
        }

        let mut entries = Vec::new();
        let mut dirs = vec![path.to_path_buf()];

        while let Some(dir) = dirs.pop() {
            let mut read_dir = tokio::fs::read_dir(&dir).await?;

            while let Some(entry) = read_dir.next_entry().await? {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();

                // Skip hidden files and directories.
                if name.starts_with('.') {
                    continue;
                }

                let file_type = entry.file_type().await?;

                // Recurse into subdirectories but never follow symlinks to dirs.
                if file_type.is_dir() {
                    dirs.push(entry.path());
                    continue;
                }

                // Only process regular files (skip symlinks).
                if !file_type.is_file() {
                    continue;
                }

                let entry_path = entry.path();

                // Fast pre-filter by extension.
                let ext = entry_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_lowercase);

                let Some(ext) = ext else { continue };
                if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
                    continue;
                }

                // Read first 64 bytes for magic-byte detection.
                let header = match read_file_header(&entry_path, 8192).await {
                    Ok(h) => h,
                    Err(e) => {
                        warn!(path = %entry_path.display(), "skipping file, cannot read header: {e}");
                        continue;
                    }
                };

                let format = match archivis_formats::detect::detect(&header) {
                    Ok(f) if f != BookFormat::Unknown => f,
                    Ok(_) => {
                        debug!(path = %entry_path.display(), "skipping file: format not recognised");
                        continue;
                    }
                    Err(e) => {
                        warn!(path = %entry_path.display(), "format detection failed: {e}");
                        continue;
                    }
                };

                let metadata = entry.metadata().await?;

                entries.push(ManifestEntry {
                    path: entry_path,
                    format,
                    size: metadata.len(),
                });
            }
        }

        // Sort by path for deterministic ordering.
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        // Build format summary.
        let mut by_format: HashMap<BookFormat, FormatCount> = HashMap::new();
        let mut total_size = 0u64;
        for entry in &entries {
            total_size = total_size.saturating_add(entry.size);
            let fc = by_format.entry(entry.format).or_insert(FormatCount {
                count: 0,
                total_size: 0,
            });
            fc.count += 1;
            fc.total_size = fc.total_size.saturating_add(entry.size);
        }

        let total_files = entries.len();

        info!(total_files, total_size, "directory scan complete");

        Ok(ImportManifest {
            total_files,
            total_size,
            by_format,
            files: entries,
        })
    }

    /// Import all supported files from a directory tree.
    ///
    /// Reports progress via the callback trait. Failures on individual files
    /// are recorded in the result but never abort the entire operation.
    pub async fn import_directory(
        &self,
        path: &Path,
        progress: &dyn ImportProgress,
    ) -> Result<BulkImportResult, ImportError> {
        let manifest = self.scan_directory(path).await?;

        progress.on_import_start(manifest.total_files);

        let mut imported = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();

        for (index, entry) in manifest.files.iter().enumerate() {
            // Check for cancellation before processing each file
            if progress.should_cancel() {
                info!(
                    imported = imported.len(),
                    skipped = skipped.len(),
                    failed = failed.len(),
                    remaining = manifest.files.len() - index,
                    "bulk import cancelled"
                );
                break;
            }

            progress.on_file_start(index, &entry.path);

            let outcome = match self.import_service.import_file(&entry.path).await {
                Ok(result) => match result.duplicate {
                    Some(DuplicateInfo::ExactHash { existing_book_id }) => {
                        let reason = SkipReason::DuplicateHash { existing_book_id };
                        let outcome = FileOutcome::Skipped(reason.clone());
                        skipped.push(SkippedFile {
                            path: entry.path.clone(),
                            reason,
                        });
                        outcome
                    }
                    Some(DuplicateInfo::SameIsbn {
                        existing_book_id,
                        isbn,
                    }) => {
                        let reason = SkipReason::DuplicateIsbn {
                            existing_book_id,
                            isbn,
                        };
                        let outcome = FileOutcome::Skipped(reason.clone());
                        skipped.push(SkippedFile {
                            path: entry.path.clone(),
                            reason,
                        });
                        outcome
                    }
                    Some(DuplicateInfo::FuzzyMatch { .. }) | None => {
                        // FuzzyMatch is a soft duplicate — the book is still
                        // imported. The duplicate info is preserved in the
                        // ImportResult for downstream consumers.
                        let outcome = FileOutcome::Imported(result.book_id);
                        imported.push(result);
                        outcome
                    }
                },
                Err(ImportError::DuplicateFile {
                    existing_book_id, ..
                }) => {
                    let reason = SkipReason::DuplicateHash { existing_book_id };
                    let outcome = FileOutcome::Skipped(reason.clone());
                    skipped.push(SkippedFile {
                        path: entry.path.clone(),
                        reason,
                    });
                    outcome
                }
                Err(ImportError::InvalidFile(_)) => {
                    let reason = SkipReason::UnsupportedFormat;
                    let outcome = FileOutcome::Skipped(reason.clone());
                    skipped.push(SkippedFile {
                        path: entry.path.clone(),
                        reason,
                    });
                    outcome
                }
                Err(other) => {
                    let error_msg = other.to_string();
                    warn!(path = %entry.path.display(), error = %error_msg, "file import failed");
                    let outcome = FileOutcome::Failed(error_msg.clone());
                    failed.push(FailedFile {
                        path: entry.path.clone(),
                        error: error_msg,
                    });
                    outcome
                }
            };

            progress.on_file_complete(index, &entry.path, &outcome);
        }

        let result = BulkImportResult {
            imported,
            skipped,
            failed,
        };

        progress.on_import_complete(&result);

        info!(
            imported = result.imported.len(),
            skipped = result.skipped.len(),
            failed = result.failed.len(),
            "bulk import complete"
        );

        Ok(result)
    }
}

/// Read up to `max_bytes` from the beginning of a file.
async fn read_file_header(path: &Path, max_bytes: usize) -> Result<Vec<u8>, std::io::Error> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; max_bytes];
    let n = file.read(&mut buf).await?;
    buf.truncate(n);
    Ok(buf)
}
