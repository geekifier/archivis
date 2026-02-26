use std::io::Write;
use std::path::Path;

use archivis_db::{BookFileRepository, BookRepository};
use archivis_storage::local::LocalStorage;
use archivis_tasks::import::{ImportConfig, ImportError, ImportService, ThumbnailSizes};
use tempfile::TempDir;

/// Create a test EPUB with an SVG cover image (EPUB 3 properties="cover-image").
fn create_test_epub_with_svg_cover(title: &str, author: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);

        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="epub/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )
        .unwrap();

        zip.start_file("epub/content.opf", deflated).unwrap();
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:identifier id="uid">urn:uuid:aaaabbbb-cccc-dddd-eeee-ffffffffffff</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="cover" href="images/cover.svg" media-type="image/svg+xml" properties="cover-image"/>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="content"/>
  </spine>
</package>"#
        );
        zip.write_all(opf.as_bytes()).unwrap();

        zip.start_file("epub/images/cover.svg", deflated).unwrap();
        zip.write_all(
            br##"<?xml version="1.0" encoding="utf-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 300">
  <rect width="200" height="300" fill="#336699"/>
  <text x="100" y="150" text-anchor="middle" fill="white" font-size="20">Cover</text>
</svg>"##,
        )
        .unwrap();

        zip.start_file("epub/content.xhtml", deflated).unwrap();
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

/// Verify the actual Frankenstein advanced EPUB (with SVG cover) can be imported
/// with thumbnails. Skipped if the test file is not present.
#[tokio::test]
async fn import_real_epub_with_svg_cover() {
    let epub_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/test-files/mary-shelley_frankenstein_advanced.epub");
    if !epub_path.exists() {
        eprintln!("skipping: test file not found at {}", epub_path.display());
        return;
    }

    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let result = service.import_file(&epub_path).await.unwrap();

    assert!(result.cover_extracted, "cover_extracted should be true");

    let pool = get_pool(&tmp).await;
    let book = BookRepository::get_by_id(&pool, result.book_id)
        .await
        .unwrap();
    assert!(
        book.title.starts_with("Frankenstein"),
        "title should start with 'Frankenstein', got: {}",
        book.title,
    );
    assert!(book.cover_path.is_some(), "cover_path should be set");

    // Verify thumbnails were generated (the whole point of this fix)
    let sizes = ThumbnailSizes::default();
    let covers_dir = tmp
        .path()
        .join("data")
        .join("covers")
        .join(result.book_id.to_string());
    assert!(covers_dir.join("sm.webp").exists(), "sm thumbnail missing");
    assert!(covers_dir.join("md.webp").exists(), "md thumbnail missing");

    let sm_img = image::open(covers_dir.join("sm.webp")).unwrap();
    assert_eq!(sm_img.height(), sizes.sm_height);

    pool.close().await;
}

#[tokio::test]
async fn import_epub_with_svg_cover_generates_thumbnails() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Write test EPUB with SVG cover to a file
    let epub_bytes = create_test_epub_with_svg_cover("Frankenstein", "Mary Shelley");
    let epub_path = tmp.path().join("frankenstein.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();

    let result = service.import_file(&epub_path).await.unwrap();

    assert!(result.duplicate.is_none());
    assert!(result.cover_extracted, "cover_extracted should be true");

    // Verify cover was stored
    let pool = get_pool(&tmp).await;
    let book = BookRepository::get_by_id(&pool, result.book_id)
        .await
        .unwrap();
    assert!(
        book.cover_path.is_some(),
        "book should have a cover_path in DB"
    );
    let cover_path = book.cover_path.unwrap();
    assert!(
        std::path::Path::new(&cover_path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg")),
        "cover should be stored as SVG"
    );

    // Verify thumbnails were generated
    let sizes = ThumbnailSizes::default();
    let covers_dir = tmp
        .path()
        .join("data")
        .join("covers")
        .join(result.book_id.to_string());
    let sm_path = covers_dir.join("sm.webp");
    let md_path = covers_dir.join("md.webp");
    assert!(sm_path.exists(), "sm.webp thumbnail should exist");
    assert!(md_path.exists(), "md.webp thumbnail should exist");

    // Validate the thumbnails are valid WebP images with correct dimensions
    let sm_img = image::open(&sm_path).unwrap();
    assert_eq!(sm_img.height(), sizes.sm_height);
    let md_img = image::open(&md_path).unwrap();
    assert_eq!(md_img.height(), sizes.md_height);

    pool.close().await;
}
