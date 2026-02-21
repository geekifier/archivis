use std::io::Write;

use archivis_core::models::BookFormat;
use archivis_formats::detect;

/// Build a minimal valid EPUB as raw bytes.
fn build_epub() -> Vec<u8> {
    let buf = Vec::new();
    let cursor = std::io::Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);

    // The mimetype entry must be stored (not compressed) and be the first entry.
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", options).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();

    // Minimal META-INF/container.xml
    let deflate = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("META-INF/container.xml", deflate).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
    )
    .unwrap();

    // Minimal content.opf
    zip.start_file("content.opf", deflate).unwrap();
    zip.write_all(
        br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test</dc:title>
  </metadata>
  <manifest/>
  <spine/>
</package>"#,
    )
    .unwrap();

    zip.finish().unwrap().into_inner()
}

/// Build a minimal CBZ (ZIP with image files).
fn build_cbz() -> Vec<u8> {
    let buf = Vec::new();
    let cursor = std::io::Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);

    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    // A tiny 1x1 PNG (valid header, not a real image but has the extension)
    zip.start_file("page001.png", options).unwrap();
    // Minimal PNG: 8-byte signature + IHDR + IEND
    let png_sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    zip.write_all(&png_sig).unwrap();
    zip.write_all(&[0u8; 64]).unwrap(); // padding for a fake chunk

    zip.start_file("page002.jpg", options).unwrap();
    zip.write_all(b"fake jpg data").unwrap();

    zip.finish().unwrap().into_inner()
}

/// Build a minimal PDF.
fn build_pdf() -> Vec<u8> {
    b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
      2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\n\
      xref\n0 3\ntrailer<</Size 3/Root 1 0 R>>\nstartxref\n9\n%%EOF"
        .to_vec()
}

/// Build a minimal MOBI (PDB with BOOKMOBI at offset 60).
fn build_mobi() -> Vec<u8> {
    let mut data = vec![0u8; 60];
    // PDB name (first 32 bytes)
    data[..9].copy_from_slice(b"Test Book");
    // Type/creator at offset 60
    data.extend_from_slice(b"BOOKMOBI");
    // Pad to reasonable size
    data.resize(512, 0);
    data
}

/// Build a minimal FB2 document.
fn build_fb2() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
  <description>
    <title-info>
      <author><first-name>Test</first-name><last-name>Author</last-name></author>
      <book-title>Test Book</book-title>
      <lang>en</lang>
    </title-info>
  </description>
  <body>
    <section><p>Hello, world!</p></section>
  </body>
</FictionBook>"#
        .to_vec()
}

/// Build a minimal DJVU file (just the magic header).
fn build_djvu() -> Vec<u8> {
    let mut data = b"AT&TFORM".to_vec();
    // Length (big-endian u32) + DJVU form type
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x20]);
    data.extend_from_slice(b"DJVU");
    data.resize(512, 0);
    data
}

/// Build a plain text file.
fn build_txt() -> Vec<u8> {
    b"The quick brown fox jumps over the lazy dog.\n\
      This is a plain text file used for format detection testing.\n\
      It contains only valid UTF-8 characters and no null bytes.\n"
        .to_vec()
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn detect_epub() {
    let data = build_epub();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Epub);
}

#[test]
fn detect_cbz() {
    let data = build_cbz();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Cbz);
}

#[test]
fn detect_pdf() {
    let data = build_pdf();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Pdf);
}

#[test]
fn detect_mobi() {
    let data = build_mobi();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Mobi);
}

#[test]
fn detect_fb2() {
    let data = build_fb2();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Fb2);
}

#[test]
fn detect_djvu() {
    let data = build_djvu();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Djvu);
}

#[test]
fn detect_txt() {
    let data = build_txt();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Txt);
}

#[test]
fn detect_unknown_for_random_binary() {
    // Random binary data that doesn't match any known format
    let data: Vec<u8> = (0u8..=255).collect();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Unknown);
}

#[test]
fn detect_error_for_empty_input() {
    assert!(detect::detect(&[]).is_err());
}

#[test]
fn detect_error_for_tiny_input() {
    assert!(detect::detect(&[0x50, 0x4B]).is_err());
}

#[test]
fn epub_vs_cbz_distinction() {
    // Both are ZIP archives, but detection must distinguish them
    let epub = build_epub();
    let cbz = build_cbz();
    assert_eq!(detect::detect(&epub).unwrap(), BookFormat::Epub);
    assert_eq!(detect::detect(&cbz).unwrap(), BookFormat::Cbz);
}

#[test]
fn fb2_with_xml_declaration() {
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
  <body><section><p>Test</p></section></body>
</FictionBook>"#;
    assert_eq!(detect::detect(xml).unwrap(), BookFormat::Fb2);
}

#[test]
fn fb2_with_bom() {
    let mut data = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
    data.extend_from_slice(b"<?xml version=\"1.0\"?>\n<FictionBook><body/></FictionBook>");
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Fb2);
}

#[test]
fn zip_without_epub_or_images_is_unknown() {
    let buf = Vec::new();
    let cursor = std::io::Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);

    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("readme.txt", options).unwrap();
    zip.write_all(b"just a text file in a zip").unwrap();

    let data = zip.finish().unwrap().into_inner();
    assert_eq!(detect::detect(&data).unwrap(), BookFormat::Unknown);
}
