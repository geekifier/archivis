use std::path::Path;

use uuid::Uuid;

use super::types::ImportResult;

/// Why a file was skipped during bulk import.
#[derive(Debug, Clone)]
pub enum SkipReason {
    DuplicateHash {
        existing_book_id: Uuid,
    },
    DuplicateIsbn {
        existing_book_id: Uuid,
        isbn: String,
    },
    UnsupportedFormat,
}

/// Outcome of processing a single file during bulk import.
#[derive(Debug)]
pub enum FileOutcome {
    Imported(Uuid),
    Skipped(SkipReason),
    Failed(String),
}

/// A file that was skipped during bulk import.
#[derive(Debug)]
pub struct SkippedFile {
    pub path: std::path::PathBuf,
    pub reason: SkipReason,
}

/// A file that failed to import.
#[derive(Debug)]
pub struct FailedFile {
    pub path: std::path::PathBuf,
    pub error: String,
}

/// Categorized results of a bulk import operation.
#[derive(Debug)]
pub struct BulkImportResult {
    pub imported: Vec<ImportResult>,
    pub skipped: Vec<SkippedFile>,
    pub failed: Vec<FailedFile>,
}

/// Progress reporting for bulk import operations.
///
/// All methods have default no-op implementations so callers can override
/// only the events they care about.
pub trait ImportProgress: Send + Sync {
    fn on_scan_progress(&self, _files_found: usize) {}
    fn on_import_start(&self, _total_files: usize) {}
    fn on_file_start(&self, _index: usize, _path: &Path) {}
    fn on_file_complete(&self, _index: usize, _path: &Path, _outcome: &FileOutcome) {}
    fn on_import_complete(&self, _result: &BulkImportResult) {}
}

/// No-op progress reporter.
pub struct NoopProgress;

impl ImportProgress for NoopProgress {}
