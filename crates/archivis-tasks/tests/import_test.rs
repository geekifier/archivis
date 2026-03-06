use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use archivis_core::models::{
    Identifier, IdentifierType, MetadataSource, ResolutionState, Task, TaskType,
};
use archivis_db::{
    BookFileRepository, BookRepository, DuplicateRepository, IdentifierRepository,
    SettingRepository, TaskRepository,
};
use archivis_storage::local::LocalStorage;
use archivis_tasks::import::{ImportConfig, ImportError, ImportService, ThumbnailSizes};
use archivis_tasks::isbn_scan::{IsbnScanConfig, IsbnScanService};
use archivis_tasks::queue::{ProgressSender, TaskQueue, Worker};
use archivis_tasks::workers::{ImportFileWorker, IsbnScanWorker};
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

/// Create a minimal valid PDF file as bytes with title and author metadata.
fn create_test_pdf(title: &str, author: &str) -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.4");

    // Create a minimal page with a content stream
    let content = Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf 100 700 Td (Hello) Tj ET".to_vec(),
    );
    let content_id = doc.add_object(content);

    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    };
    let font_id = doc.add_object(font);

    let resources = dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
    };

    let page = dictionary! {
        "Type" => "Page",
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => content_id,
        "Resources" => resources,
    };
    let page_id = doc.add_object(page);

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    let page_tree_id = doc.add_object(pages);

    // Update the page's Parent reference
    if let Ok(Object::Dictionary(ref mut dict)) = doc.get_object_mut(page_id) {
        dict.set("Parent", page_tree_id);
    }

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => page_tree_id,
    });
    doc.trailer.set("Root", catalog_id);

    // Set metadata via Info dictionary
    let info = dictionary! {
        "Title" => Object::string_literal(title),
        "Author" => Object::string_literal(author),
    };
    let info_id = doc.add_object(info);
    doc.trailer.set("Info", info_id);

    let mut buf = Vec::new();
    doc.save_to(&mut buf).unwrap();
    buf
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

async fn progress_sender_for_task(
    queue: &TaskQueue,
    pool: &archivis_db::DbPool,
    task_type: TaskType,
    payload: serde_json::Value,
) -> ProgressSender {
    let task = Task::new(task_type, payload);
    let task_id = task.id;

    TaskRepository::create(pool, &task).await.unwrap();

    queue.progress_sender().for_task(task_id)
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
async fn import_sets_pending_resolution_and_protected_provenance() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub_bytes = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();

    let result = service.import_file(&epub_path).await.unwrap();
    let pool = get_pool(&tmp).await;
    let book = BookRepository::get_by_id(&pool, result.book_id)
        .await
        .unwrap();

    assert_eq!(book.resolution_state, ResolutionState::Pending);
    assert_eq!(book.resolution_requested_reason.as_deref(), Some("import"));
    assert!(!book.metadata_locked);

    let title = book.metadata_provenance.title.as_ref().unwrap();
    assert_eq!(title.origin, MetadataSource::Embedded);
    assert!(title.protected);

    let authors = book.metadata_provenance.authors.as_ref().unwrap();
    assert_eq!(authors.origin, MetadataSource::Embedded);
    assert!(authors.protected);

    let language = book.metadata_provenance.language.as_ref().unwrap();
    assert_eq!(language.origin, MetadataSource::Embedded);
    assert!(language.protected);

    pool.close().await;
}

#[tokio::test]
async fn import_worker_with_scan_on_import_enqueues_only_scan_child() {
    let tmp = TempDir::new().unwrap();
    let import_service = Arc::new(setup_test_env(&tmp).await);
    let worker = ImportFileWorker::new(Arc::clone(&import_service));
    let pool = get_pool(&tmp).await;
    let (queue_inner, rx) = TaskQueue::new(pool.clone());
    let queue = Arc::new(queue_inner);
    let worker = worker.with_isbn_scan(Arc::clone(&queue), true);

    let epub_bytes = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();

    let payload = serde_json::json!({
        "file_path": epub_path.to_string_lossy(),
    });
    let progress =
        progress_sender_for_task(&queue, &pool, TaskType::ImportFile, payload.clone()).await;

    worker.execute(payload, progress.clone()).await.unwrap();

    let children = TaskRepository::list_children(&pool, progress.task_id())
        .await
        .unwrap();
    assert_eq!(
        children.len(),
        1,
        "import should enqueue only the scan child"
    );
    assert_eq!(children[0].task_type, TaskType::ScanIsbn);
    assert_eq!(children[0].payload["resolve_after_scan"], true);

    drop(rx);
    pool.close().await;
}

#[tokio::test]
async fn manual_isbn_scan_noop_does_not_enqueue_resolution_child() {
    let tmp = TempDir::new().unwrap();
    let import_service = setup_test_env(&tmp).await;

    let epub_bytes = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();
    let import_result = import_service.import_file(&epub_path).await.unwrap();

    let pool = get_pool(&tmp).await;
    let existing = Identifier::new(
        import_result.book_id,
        IdentifierType::Isbn13,
        "9780441172719",
        MetadataSource::Embedded,
        1.0,
    );
    IdentifierRepository::create(&pool, &existing)
        .await
        .unwrap();

    let storage = LocalStorage::new(&tmp.path().join("storage"))
        .await
        .unwrap();
    let scan_service = Arc::new(IsbnScanService::new(
        pool.clone(),
        storage,
        IsbnScanConfig::default(),
    ));
    let (queue_inner, rx) = TaskQueue::new(pool.clone());
    let queue = Arc::new(queue_inner);
    let worker = IsbnScanWorker::new(scan_service).with_resolution_queue(Arc::clone(&queue));

    let payload = serde_json::json!({
        "book_id": import_result.book_id.to_string(),
    });
    let progress =
        progress_sender_for_task(&queue, &pool, TaskType::ScanIsbn, payload.clone()).await;

    worker.execute(payload, progress.clone()).await.unwrap();

    let children = TaskRepository::list_children(&pool, progress.task_id())
        .await
        .unwrap();
    assert!(
        children.is_empty(),
        "scan no-op should not enqueue a resolve child"
    );

    drop(rx);
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

// ── Auto-link format tests ──────────────────────────────────────────

#[tokio::test]
async fn import_epub_then_pdf_links_to_same_book() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Import EPUB first
    let epub_bytes = create_test_epub("Starship Troopers", "Robert A. Heinlein");
    let epub_path = tmp.path().join("starship_troopers.epub");
    std::fs::write(&epub_path, &epub_bytes).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    // Import PDF with identical metadata — should auto-link
    let pdf_bytes = create_test_pdf("Starship Troopers", "Robert A. Heinlein");
    let pdf_path = tmp.path().join("starship_troopers.pdf");
    std::fs::write(&pdf_path, &pdf_bytes).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_eq!(
        r1.book_id, r2.book_id,
        "both formats should share the same book"
    );
    assert!(
        r2.duplicate.is_none(),
        "auto-linked file should not be flagged as duplicate"
    );

    let pool = get_pool(&tmp).await;
    let files = BookFileRepository::get_by_book_id(&pool, r1.book_id)
        .await
        .unwrap();
    assert_eq!(files.len(), 2, "book should have 2 format files");

    let formats: Vec<_> = files.iter().map(|f| f.format).collect();
    assert!(formats.contains(&archivis_core::models::BookFormat::Epub));
    assert!(formats.contains(&archivis_core::models::BookFormat::Pdf));

    pool.close().await;
}

#[tokio::test]
async fn import_same_format_different_content_creates_new_book() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Import first EPUB
    let epub1 = create_test_epub("Dune", "Frank Herbert");
    let epub1_path = tmp.path().join("dune_v1.epub");
    std::fs::write(&epub1_path, &epub1).unwrap();
    let r1 = service.import_file(&epub1_path).await.unwrap();

    // Import second EPUB with same metadata but different content (different hash)
    // The create_test_epub in import_test uses a fixed UUID, so we need a slightly
    // different approach — use the bulk_import's version that generates random UUIDs
    let epub2 = {
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
    <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
            )
            .unwrap();

            zip.start_file("content.opf", deflated).unwrap();
            let opf = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dune</dc:title>
    <dc:creator>Frank Herbert</dc:creator>
    <dc:identifier id="uid">urn:uuid:{}</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="content"/>
  </spine>
</package>"#,
                uuid::Uuid::new_v4()
            );
            zip.write_all(opf.as_bytes()).unwrap();

            zip.start_file("content.xhtml", deflated).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body><p>Different content for different hash</p></body>
</html>"#,
            )
            .unwrap();

            zip.finish().unwrap();
        }
        buf.into_inner()
    };
    let epub2_path = tmp.path().join("dune_v2.epub");
    std::fs::write(&epub2_path, &epub2).unwrap();
    let r2 = service.import_file(&epub2_path).await.unwrap();

    // Same format guard should prevent auto-linking
    assert_ne!(
        r1.book_id, r2.book_id,
        "same format should create separate books"
    );

    pool_close(&tmp).await;
}

#[tokio::test]
async fn import_low_similarity_does_not_auto_link() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // "Dune" and "Dune Messiah" are different books
    let epub = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    let pdf = create_test_pdf("Dune Messiah", "Frank Herbert");
    let pdf_path = tmp.path().join("dune_messiah.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_ne!(
        r1.book_id, r2.book_id,
        "different titles should not auto-link"
    );

    pool_close(&tmp).await;
}

#[tokio::test]
async fn import_different_authors_not_auto_linked() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub = create_test_epub("Algorithms", "Robert Sedgewick");
    let epub_path = tmp.path().join("algorithms_sedgewick.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    let pdf = create_test_pdf("Algorithms", "Thomas H. Cormen");
    let pdf_path = tmp.path().join("algorithms_cormen.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_ne!(
        r1.book_id, r2.book_id,
        "same title different authors should not auto-link"
    );

    pool_close(&tmp).await;
}

#[tokio::test]
async fn self_duplicate_link_not_created() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub = create_test_epub("Dune", "Frank Herbert");
    let epub_path = tmp.path().join("dune.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r = service.import_file(&epub_path).await.unwrap();

    let pool = get_pool(&tmp).await;
    let links = DuplicateRepository::find_for_book(&pool, r.book_id)
        .await
        .unwrap();
    for link in &links {
        assert_ne!(
            link.book_id_a, link.book_id_b,
            "self-referential duplicate link should never exist"
        );
    }

    pool.close().await;
}

#[tokio::test]
async fn auto_link_disabled_creates_separate_books() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Disable auto-linking via setting
    let pool = get_pool(&tmp).await;
    SettingRepository::set(&pool, "import.auto_link_formats", "false")
        .await
        .unwrap();
    pool.close().await;

    // Import EPUB
    let epub = create_test_epub("Permanent Record", "Edward Snowden");
    let epub_path = tmp.path().join("permanent_record.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    // Import PDF with identical metadata — should NOT auto-link because setting is off
    let pdf = create_test_pdf("Permanent Record", "Edward Snowden");
    let pdf_path = tmp.path().join("permanent_record.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_ne!(
        r1.book_id, r2.book_id,
        "with auto-link disabled, should create separate books"
    );
    assert!(
        r2.duplicate.is_some(),
        "should report fuzzy duplicate when not auto-linked"
    );

    pool_close(&tmp).await;
}

/// Regression: titles starting with articles ("The", "A", "An") must still
/// auto-link across formats. The prefix was previously computed from the raw
/// title, not the article-stripped `sort_title`.
#[tokio::test]
async fn import_article_title_links_to_same_book() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Import EPUB with a "The …" title
    let epub = create_test_epub("The Ballad of Songbirds and Snakes", "Suzanne Collins");
    let epub_path = tmp.path().join("ballad.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    // Import PDF with identical metadata — should auto-link
    let pdf = create_test_pdf("The Ballad of Songbirds and Snakes", "Suzanne Collins");
    let pdf_path = tmp.path().join("ballad.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_eq!(
        r1.book_id, r2.book_id,
        "article-prefixed title should auto-link to the same book"
    );
    assert!(
        r2.duplicate.is_none(),
        "auto-linked file should not be flagged as duplicate"
    );

    let pool = get_pool(&tmp).await;
    let files = BookFileRepository::get_by_book_id(&pool, r1.book_id)
        .await
        .unwrap();
    assert_eq!(files.len(), 2, "book should have 2 format files");
    pool.close().await;
}

/// Verify that semicolon-separated authors in EPUB metadata are split into
/// separate `Author` records during import.
#[tokio::test]
async fn import_epub_normalizes_semicolon_authors() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub = create_test_epub("Test Book", "Author1;Author2;");
    let path = tmp.path().join("multi_author.epub");
    std::fs::write(&path, &epub).unwrap();

    let result = service.import_file(&path).await.unwrap();

    let pool = get_pool(&tmp).await;
    let book = BookRepository::get_with_relations(&pool, result.book_id)
        .await
        .unwrap();

    let names: Vec<&str> = book
        .authors
        .iter()
        .map(|a| a.author.name.as_str())
        .collect();
    assert_eq!(names.len(), 2, "expected 2 authors, got: {names:?}");
    assert!(names.contains(&"Author1"), "missing Author1: {names:?}");
    assert!(names.contains(&"Author2"), "missing Author2: {names:?}");

    pool.close().await;
}

/// Helper to close the pool from a temp dir.
async fn pool_close(tmp: &TempDir) {
    let pool = get_pool(tmp).await;
    pool.close().await;
}

// ── Duplicate detection regression tests ────────────────────────────

/// Regression: titles whose first word is < 3 chars (e.g. "In Plain Sight")
/// must still be detected as duplicates across formats.
#[tokio::test]
async fn import_short_word_title_links_to_same_book() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub = create_test_epub("In Plain Sight", "Ross Coulthart");
    let epub_path = tmp.path().join("in_plain_sight.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    let pdf = create_test_pdf("In Plain Sight", "Ross Coulthart");
    let pdf_path = tmp.path().join("in_plain_sight.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_eq!(
        r1.book_id, r2.book_id,
        "identical short-word title + author should auto-link to the same book"
    );
    assert!(
        r2.duplicate.is_none(),
        "auto-linked file should not be flagged as duplicate"
    );

    let pool = get_pool(&tmp).await;
    let files = BookFileRepository::get_by_book_id(&pool, r1.book_id)
        .await
        .unwrap();
    assert_eq!(files.len(), 2, "book should have 2 format files");
    pool.close().await;
}

/// Same title, different authors → separate books + `DuplicateLink` for review.
#[tokio::test]
async fn import_mismatched_authors_flags_duplicate() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub = create_test_epub("Welcome to MyAnonamouse", "Author Alpha");
    let epub_path = tmp.path().join("mam_alpha.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    let pdf = create_test_pdf("Welcome to MyAnonamouse", "Author Beta");
    let pdf_path = tmp.path().join("mam_beta.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_ne!(
        r1.book_id, r2.book_id,
        "different authors should create separate books"
    );
    assert!(
        r2.duplicate.is_some(),
        "title-only match should flag as duplicate"
    );

    let pool = get_pool(&tmp).await;
    let links = DuplicateRepository::find_for_book(&pool, r2.book_id)
        .await
        .unwrap();
    assert!(
        !links.is_empty(),
        "DuplicateLink should exist between the two books"
    );
    pool.close().await;
}

/// One book has author, the other has no embedded author → separate books + `DuplicateLink`.
#[tokio::test]
async fn import_one_empty_author_flags_duplicate() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    let epub = create_test_epub("Welcome to MyAnonamouse", "Some Author");
    let epub_path = tmp.path().join("mam_with_author.epub");
    std::fs::write(&epub_path, &epub).unwrap();
    let r1 = service.import_file(&epub_path).await.unwrap();

    // PDF with empty author — the filename-based fallback will yield "Unknown Author"
    let pdf = create_test_pdf("Welcome to MyAnonamouse", "");
    let pdf_path = tmp.path().join("mam_no_author.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r2 = service.import_file(&pdf_path).await.unwrap();

    assert_ne!(
        r1.book_id, r2.book_id,
        "one-sided author should create separate books"
    );
    assert!(
        r2.duplicate.is_some(),
        "title-only match should flag as duplicate"
    );

    let pool = get_pool(&tmp).await;
    let links = DuplicateRepository::find_for_book(&pool, r2.book_id)
        .await
        .unwrap();
    assert!(
        !links.is_empty(),
        "DuplicateLink should exist between the two books"
    );
    pool.close().await;
}

/// When multiple candidates exist, the best title+author match should be preferred.
#[tokio::test]
async fn import_best_candidate_preferred() {
    let tmp = TempDir::new().unwrap();
    let service = setup_test_env(&tmp).await;

    // Create two existing books with the same title but different authors
    let epub1 = create_test_epub("Algorithms Unlocked", "Author X");
    let epub1_path = tmp.path().join("algo_x.epub");
    std::fs::write(&epub1_path, &epub1).unwrap();
    let r1 = service.import_file(&epub1_path).await.unwrap();

    // Use a variant EPUB (different UUID → different hash) for the second book
    let epub2 = {
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
    <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
            )
            .unwrap();

            zip.start_file("content.opf", deflated).unwrap();
            let opf = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Algorithms Unlocked</dc:title>
    <dc:creator>Author Y</dc:creator>
    <dc:identifier id="uid">urn:uuid:{}</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="content" href="content.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="content"/>
  </spine>
</package>"#,
                uuid::Uuid::new_v4()
            );
            zip.write_all(opf.as_bytes()).unwrap();

            zip.start_file("content.xhtml", deflated).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body><p>Different content Y</p></body>
</html>"#,
            )
            .unwrap();

            zip.finish().unwrap();
        }
        buf.into_inner()
    };
    let epub2_path = tmp.path().join("algo_y.epub");
    std::fs::write(&epub2_path, &epub2).unwrap();
    let r2 = service.import_file(&epub2_path).await.unwrap();

    assert_ne!(
        r1.book_id, r2.book_id,
        "different authors should create separate books"
    );

    // Now import a PDF with Author Y — should auto-link to book r2, not r1
    let pdf = create_test_pdf("Algorithms Unlocked", "Author Y");
    let pdf_path = tmp.path().join("algo_y.pdf");
    std::fs::write(&pdf_path, &pdf).unwrap();
    let r3 = service.import_file(&pdf_path).await.unwrap();

    assert_eq!(
        r3.book_id, r2.book_id,
        "PDF should auto-link to the book with matching author (Author Y)"
    );
    assert!(
        r3.duplicate.is_none(),
        "auto-linked file should not be flagged as duplicate"
    );

    pool_close(&tmp).await;
}
