use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use archivis_storage::local::LocalStorage;
use archivis_tasks::import::{
    BulkImportResult, BulkImportService, FileOutcome, ImportConfig, ImportProgress, ImportService,
    NoopProgress, SkipReason,
};
use tempfile::TempDir;

/// Create a minimal valid EPUB file as bytes (same as `import_test.rs`).
fn create_test_epub(title: &str, author: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", options).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("META-INF/container.xml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )
        .unwrap();

        zip.start_file("content.opf", options).unwrap();
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:identifier id="uid">urn:uuid:{id}</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="content"/>
  </spine>
</package>"#,
            id = uuid::Uuid::new_v4()
        );
        zip.write_all(opf.as_bytes()).unwrap();

        zip.start_file("content.xhtml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body><p>Hello, world!</p></body>
</html>"#,
        )
        .unwrap();

        zip.finish().unwrap();
    }
    buf.into_inner()
}

/// Set up a test environment with a temporary DB, storage, and bulk import service.
async fn setup_bulk_env(tmp: &TempDir) -> BulkImportService<LocalStorage> {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();

    let config = ImportConfig {
        data_dir: tmp.path().join("data"),
        ..ImportConfig::default()
    };

    let import_service = ImportService::new(pool, storage, config);
    BulkImportService::new(import_service)
}

#[tokio::test]
async fn scan_directory_finds_supported_files() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    // Create a directory with books to scan.
    let books_dir = tmp.path().join("books");
    std::fs::create_dir_all(&books_dir).unwrap();

    // Valid EPUB
    let epub = create_test_epub("Dune", "Frank Herbert");
    std::fs::write(books_dir.join("dune.epub"), &epub).unwrap();

    // Binary junk with .dat extension (should be skipped — unsupported extension)
    let junk: Vec<u8> = (0..=255).collect();
    std::fs::write(books_dir.join("random.dat"), &junk).unwrap();

    // Binary junk with .epub extension (should be skipped — invalid magic bytes)
    std::fs::write(books_dir.join("fake.epub"), &junk).unwrap();

    let manifest = service.scan_directory(&books_dir).await.unwrap();

    assert_eq!(manifest.total_files, 1);
    assert_eq!(manifest.files.len(), 1);
    assert_eq!(
        manifest.files[0].format,
        archivis_core::models::BookFormat::Epub
    );
    assert!(manifest.total_size > 0);

    // Format summary should have exactly 1 EPUB.
    let epub_count = manifest
        .by_format
        .get(&archivis_core::models::BookFormat::Epub)
        .unwrap();
    assert_eq!(epub_count.count, 1);
}

#[tokio::test]
async fn scan_directory_skips_hidden_files() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let books_dir = tmp.path().join("books");
    std::fs::create_dir_all(&books_dir).unwrap();

    // Visible EPUB
    let epub = create_test_epub("Visible", "Author");
    std::fs::write(books_dir.join("visible.epub"), &epub).unwrap();

    // Hidden EPUB (should be skipped)
    std::fs::write(books_dir.join(".hidden.epub"), &epub).unwrap();

    // Hidden directory with EPUB inside (should be skipped)
    let hidden_dir = books_dir.join(".hidden_dir");
    std::fs::create_dir_all(&hidden_dir).unwrap();
    std::fs::write(hidden_dir.join("book.epub"), &epub).unwrap();

    let manifest = service.scan_directory(&books_dir).await.unwrap();

    assert_eq!(manifest.total_files, 1);
    assert!(manifest.files[0].path.ends_with("visible.epub"));
}

#[tokio::test]
async fn scan_directory_recurses_subdirectories() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let books_dir = tmp.path().join("books");
    let sub_dir = books_dir.join("scifi");
    std::fs::create_dir_all(&sub_dir).unwrap();

    let epub1 = create_test_epub("Dune", "Frank Herbert");
    std::fs::write(books_dir.join("dune.epub"), &epub1).unwrap();

    let epub2 = create_test_epub("Foundation", "Isaac Asimov");
    std::fs::write(sub_dir.join("foundation.epub"), &epub2).unwrap();

    let manifest = service.scan_directory(&books_dir).await.unwrap();

    assert_eq!(manifest.total_files, 2);
    // Files should be sorted by path.
    assert!(manifest.files[0].path < manifest.files[1].path);
}

#[tokio::test]
async fn scan_directory_rejects_non_directory() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let file_path = tmp.path().join("not_a_dir.txt");
    std::fs::write(&file_path, b"hello").unwrap();

    let result = service.scan_directory(&file_path).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn import_directory_imports_files() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let books_dir = tmp.path().join("books");
    std::fs::create_dir_all(&books_dir).unwrap();

    let epub1 = create_test_epub("Dune", "Frank Herbert");
    std::fs::write(books_dir.join("dune.epub"), &epub1).unwrap();

    let epub2 = create_test_epub("Foundation", "Isaac Asimov");
    std::fs::write(books_dir.join("foundation.epub"), &epub2).unwrap();

    let result = service
        .import_directory(&books_dir, &NoopProgress)
        .await
        .unwrap();

    assert_eq!(result.imported.len(), 2);
    assert!(result.skipped.is_empty());
    assert!(result.failed.is_empty());
}

#[tokio::test]
async fn import_directory_detects_duplicates() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let books_dir = tmp.path().join("books");
    std::fs::create_dir_all(&books_dir).unwrap();

    let epub = create_test_epub("Dune", "Frank Herbert");
    std::fs::write(books_dir.join("dune.epub"), &epub).unwrap();

    // First import: everything should succeed.
    let result1 = service
        .import_directory(&books_dir, &NoopProgress)
        .await
        .unwrap();
    assert_eq!(result1.imported.len(), 1);
    assert!(result1.skipped.is_empty());

    // Second import: same file should be skipped as duplicate hash.
    let result2 = service
        .import_directory(&books_dir, &NoopProgress)
        .await
        .unwrap();
    assert!(result2.imported.is_empty());
    assert_eq!(result2.skipped.len(), 1);
    assert!(
        matches!(&result2.skipped[0].reason, SkipReason::DuplicateHash { .. }),
        "expected DuplicateHash, got: {:?}",
        result2.skipped[0].reason
    );
}

#[tokio::test]
async fn import_directory_continues_on_failure() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let books_dir = tmp.path().join("books");
    std::fs::create_dir_all(&books_dir).unwrap();

    // Valid EPUB
    let epub = create_test_epub("Dune", "Frank Herbert");
    std::fs::write(books_dir.join("a_dune.epub"), &epub).unwrap();

    // A "valid enough" EPUB that passes magic-byte detection but has truncated content.
    // The import service will read the entire file, so this tests error resilience.
    let mut bad_epub = create_test_epub("Bad", "Author");
    bad_epub.truncate(bad_epub.len() / 4); // Corrupt it

    // Only write this if it still passes the initial ZIP magic bytes check.
    // ZIP magic is PK\x03\x04 which is the first 4 bytes — truncating to 1/4
    // should still keep that. But the detect function reads the full ZIP, so
    // this corrupted file may fail detection. Let's write it and see.
    std::fs::write(books_dir.join("b_bad.epub"), &bad_epub).unwrap();

    let result = service
        .import_directory(&books_dir, &NoopProgress)
        .await
        .unwrap();

    // At minimum, the valid EPUB should import. The bad one is either skipped
    // (if detection filters it out) or failed. Either way the import should not abort.
    assert!(
        !result.imported.is_empty(),
        "at least one file should have imported successfully"
    );
}

/// A progress reporter that records callback invocations.
struct RecordingProgress {
    scan_calls: AtomicUsize,
    import_start_total: AtomicUsize,
    file_starts: Mutex<Vec<(usize, String)>>,
    file_completes: Mutex<Vec<(usize, String)>>,
    import_complete_called: AtomicUsize,
}

impl RecordingProgress {
    fn new() -> Self {
        Self {
            scan_calls: AtomicUsize::new(0),
            import_start_total: AtomicUsize::new(0),
            file_starts: Mutex::new(Vec::new()),
            file_completes: Mutex::new(Vec::new()),
            import_complete_called: AtomicUsize::new(0),
        }
    }
}

impl ImportProgress for RecordingProgress {
    fn on_scan_progress(&self, _files_found: usize) {
        self.scan_calls.fetch_add(1, Ordering::Relaxed);
    }

    fn on_import_start(&self, total_files: usize) {
        self.import_start_total
            .store(total_files, Ordering::Relaxed);
    }

    fn on_file_start(&self, index: usize, path: &Path) {
        self.file_starts
            .lock()
            .unwrap()
            .push((index, path.display().to_string()));
    }

    fn on_file_complete(&self, index: usize, path: &Path, _outcome: &FileOutcome) {
        self.file_completes
            .lock()
            .unwrap()
            .push((index, path.display().to_string()));
    }

    fn on_import_complete(&self, _result: &BulkImportResult) {
        self.import_complete_called.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn progress_callbacks_are_invoked() {
    let tmp = TempDir::new().unwrap();
    let service = setup_bulk_env(&tmp).await;

    let books_dir = tmp.path().join("books");
    std::fs::create_dir_all(&books_dir).unwrap();

    let epub1 = create_test_epub("Dune", "Frank Herbert");
    std::fs::write(books_dir.join("dune.epub"), &epub1).unwrap();

    let epub2 = create_test_epub("Foundation", "Isaac Asimov");
    std::fs::write(books_dir.join("foundation.epub"), &epub2).unwrap();

    let progress = RecordingProgress::new();
    let result = service
        .import_directory(&books_dir, &progress)
        .await
        .unwrap();

    assert_eq!(result.imported.len(), 2);

    // on_import_start should have been called with 2.
    assert_eq!(progress.import_start_total.load(Ordering::Relaxed), 2);

    // on_file_start and on_file_complete should each have 2 entries.
    let starts: Vec<_> = progress.file_starts.lock().unwrap().clone();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0].0, 0);
    assert_eq!(starts[1].0, 1);

    let completes: Vec<_> = progress.file_completes.lock().unwrap().clone();
    assert_eq!(completes.len(), 2);
    assert_eq!(completes[0].0, 0);
    assert_eq!(completes[1].0, 1);

    // on_import_complete should have been called exactly once.
    assert_eq!(progress.import_complete_called.load(Ordering::Relaxed), 1);
}
