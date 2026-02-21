use std::io::Write;
use std::path::Path;

use archivis_db::{BookFileRepository, BookRepository};
use archivis_storage::local::LocalStorage;
use archivis_tasks::import::{ImportConfig, ImportError, ImportService};
use tempfile::TempDir;

/// Create a minimal valid EPUB file as bytes.
///
/// The EPUB spec requires a ZIP archive containing:
/// - `mimetype` entry with content `application/epub+zip` (stored, not compressed)
/// - `META-INF/container.xml` pointing to the OPF
/// - A minimal OPF file with title and author metadata
fn create_test_epub(title: &str, author: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);

        // mimetype must be first entry, stored (not compressed)
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", options).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        // META-INF/container.xml
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

        // content.opf with title and author
        zip.start_file("content.opf", options).unwrap();
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:identifier id="uid">urn:uuid:12345678-1234-1234-1234-123456789abc</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="content"/>
  </spine>
</package>"#
        );
        zip.write_all(opf.as_bytes()).unwrap();

        // Minimal content file
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

/// Set up a test environment with a temporary DB, storage, and import service.
async fn setup_test_env(tmp: &TempDir) -> ImportService<LocalStorage> {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();

    let config = ImportConfig {
        data_dir: tmp.path().join("data"),
        ..ImportConfig::default()
    };

    ImportService::new(pool, storage, config)
}

/// Helper to get a fresh DB pool from the same temp dir.
async fn get_pool(tmp: &TempDir) -> archivis_db::DbPool {
    let db_path = tmp.path().join("test.db");
    archivis_db::create_pool(&db_path).await.unwrap()
}

#[tokio::test]
async fn import_valid_epub() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Write test EPUB to a file
    let epub_bytes = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();

    let result = service.import_file(&epub_path).await.unwrap();

    assert!(result.duplicate.is_none());
    assert_eq!(
        result.status,
        archivis_core::models::MetadataStatus::NeedsReview
    );
    assert!(result.confidence > 0.0);

    // Verify DB records exist
    let pool = get_pool(&tmp).await;
    let book = BookRepository::get_by_id(&pool, result.book_id)
        .await
        .unwrap();
    assert_eq!(book.title, "Dune");

    let files = BookFileRepository::get_by_book_id(&pool, result.book_id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].format, archivis_core::models::BookFormat::Epub);

    // Verify the file exists in storage
    let storage_path = &files[0].storage_path;
    let storage_dir = tmp.path().join("storage");
    assert!(storage_dir.join(storage_path).exists());

    pool.close().await;
}

#[tokio::test]
async fn import_duplicate_hash() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub_bytes = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();

    // First import succeeds
    let result = service.import_file(&epub_path).await.unwrap();
    assert!(result.duplicate.is_none());

    // Second import of same file should fail with DuplicateFile
    let err = service.import_file(&epub_path).await.unwrap_err();
    assert!(
        matches!(err, ImportError::DuplicateFile { .. }),
        "expected DuplicateFile error, got: {err:?}"
    );
}

#[tokio::test]
async fn import_unknown_format() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Create a file with unrecognized binary content
    let junk: Vec<u8> = (0..=255).collect();
    let junk_path = tmp.path().join("mystery.dat");
    std::fs::write(&junk_path, &junk).unwrap();

    let err = service.import_file(&junk_path).await.unwrap_err();
    assert!(
        matches!(err, ImportError::InvalidFile(_)),
        "expected InvalidFile error, got: {err:?}"
    );
}

#[tokio::test]
async fn import_nonexistent_file() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let result = service
        .import_file(Path::new("/nonexistent/file.epub"))
        .await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ImportError::Io(_)));
}
