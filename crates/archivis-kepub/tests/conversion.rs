//! End-to-end conversion tests.
//!
//! Builds synthetic EPUBs in-memory rather than relying on a third-party
//! corpus so the test suite stays self-contained. The fixture builder
//! stitches together the minimum entries required for `archivis-kepub` to
//! exercise every code path: `mimetype`, `META-INF/container.xml`, an
//! OPF file, and one or more spine documents.

use std::io::{Cursor, Read, Write};

use archivis_formats::transform::FormatTransformer;
use archivis_kepub::KepubTransformer;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

fn opf(rendition_layout: Option<&str>, spine_items: &[&str]) -> String {
    use std::fmt::Write;
    let layout = rendition_layout
        .map(|l| format!(r#"<meta property="rendition:layout">{l}</meta>"#))
        .unwrap_or_default();
    let mut manifest_items = String::new();
    for p in spine_items {
        write!(
            manifest_items,
            r#"<item id="{p}" href="{p}" media-type="application/xhtml+xml"/>"#
        )
        .unwrap();
    }
    let mut spine_refs = String::new();
    for p in spine_items {
        write!(spine_refs, r#"<itemref idref="{p}"/>"#).unwrap();
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fixture</dc:title>
    <dc:identifier id="uid">urn:uuid:fixture</dc:identifier>
    <dc:language>en</dc:language>
    {layout}
  </metadata>
  <manifest>
    {manifest_items}
  </manifest>
  <spine>
    {spine_refs}
  </spine>
</package>"#
    )
}

fn xhtml_body(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="en">
<head><title>t</title></head>
<body>{body}</body>
</html>"#
    )
}

fn build_epub(spine: &[(&str, &str)], rendition_layout: Option<&str>) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let cursor = Cursor::new(&mut out);
        let mut zip = zip::ZipWriter::new(cursor);

        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(CONTAINER_XML.as_bytes()).unwrap();

        let spine_paths: Vec<&str> = spine.iter().map(|(p, _)| *p).collect();
        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(opf(rendition_layout, &spine_paths).as_bytes())
            .unwrap();

        for (path, body) in spine {
            zip.start_file(format!("OEBPS/{path}"), deflated).unwrap();
            zip.write_all(xhtml_body(body).as_bytes()).unwrap();
        }

        zip.finish().unwrap();
    }
    out
}

fn read_zip_entry(bytes: &[u8], path: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
    let mut f = archive.by_name(path).expect("entry exists");
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

fn list_zip_entries(bytes: &[u8]) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

fn convert(epub: &[u8]) -> Vec<u8> {
    KepubTransformer.transform(epub).expect("conversion ok")
}

#[test]
fn mimetype_first_and_stored() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Hello world.</p>")], None);
    let kepub = convert(&epub);

    let mut archive = zip::ZipArchive::new(Cursor::new(&kepub)).expect("open kepub");
    let first = archive.by_index(0).expect("first entry");
    assert_eq!(first.name(), "mimetype");
    assert_eq!(first.compression(), CompressionMethod::Stored);
    drop(first);

    let mt_bytes = read_zip_entry(&kepub, "mimetype");
    assert_eq!(mt_bytes, b"application/kepub+zip");
}

#[test]
fn deterministic_output() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Hello world.</p>")], None);
    let a = convert(&epub);
    let b = convert(&epub);
    assert_eq!(a, b, "kepub output must be deterministic");
}

#[test]
fn lexicographic_entry_order() {
    let epub = build_epub(
        &[("ch2.xhtml", "<p>Two.</p>"), ("ch1.xhtml", "<p>One.</p>")],
        None,
    );
    let kepub = convert(&epub);
    let entries = list_zip_entries(&kepub);
    assert_eq!(entries.first().map(String::as_str), Some("mimetype"));
    let after_mt: Vec<String> = entries.iter().skip(1).cloned().collect();
    let mut sorted = after_mt.clone();
    sorted.sort();
    assert_eq!(after_mt, sorted, "entries after mimetype must be sorted");
}

#[test]
fn xml_declaration_preserved_in_xhtml() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Hello.</p>")], None);
    let kepub = convert(&epub);
    let xhtml = read_zip_entry(&kepub, "OEBPS/ch1.xhtml");
    let s = String::from_utf8(xhtml).unwrap();
    assert!(
        s.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#)
            || s.starts_with(r"<?xml version='1.0' encoding='UTF-8'?>"),
        "XML declaration not preserved: {}",
        &s[..s.len().min(120)]
    );
}

#[test]
fn namespaces_preserved() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Hi.</p>")], None);
    let kepub = convert(&epub);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "OEBPS/ch1.xhtml")).unwrap();
    assert!(xhtml.contains(r#"xmlns="http://www.w3.org/1999/xhtml""#));
    assert!(xhtml.contains(r#"xmlns:epub="http://www.idpf.org/2007/ops""#));
    assert!(xhtml.contains(r#"xml:lang="en""#));
}

#[test]
fn empty_body_roundtrip() {
    let epub = build_epub(&[("ch1.xhtml", "")], None);
    let kepub = convert(&epub);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "OEBPS/ch1.xhtml")).unwrap();
    assert!(
        !xhtml.contains("koboSpan"),
        "empty body should not gain spans"
    );
    assert!(
        xhtml.contains("kobo.js"),
        "kobo.js script should still be injected"
    );
}

#[test]
fn kobospan_ids_well_formed() {
    let epub = build_epub(&[("ch1.xhtml", "<p>One. Two. Three.</p>")], None);
    let kepub = convert(&epub);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "OEBPS/ch1.xhtml")).unwrap();

    // Find all id="kobo.X.Y" matches; ensure pattern and uniqueness.
    let mut ids = Vec::new();
    for window in xhtml.split("id=\"kobo.").skip(1) {
        if let Some(end) = window.find('"') {
            ids.push(format!("kobo.{}", &window[..end]));
        }
    }
    assert_eq!(ids.len(), 3, "expected 3 koboSpan ids, found {ids:?}");
    let mut deduped = ids.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(ids.len(), deduped.len(), "ids must be unique");
    for id in &ids {
        let parts: Vec<&str> = id.splitn(3, '.').collect();
        assert_eq!(parts.len(), 3, "id should be kobo.<para>.<seg>: {id}");
        assert!(parts[1].parse::<usize>().is_ok());
        assert!(parts[2].parse::<usize>().is_ok());
    }
}

#[test]
fn idempotent_on_kepub_input() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Hello.</p>")], None);
    let kepub_once = convert(&epub);
    let kepub_twice = convert(&kepub_once);
    let xhtml = String::from_utf8(read_zip_entry(&kepub_twice, "OEBPS/ch1.xhtml")).unwrap();
    let span_count = xhtml.matches("class=\"koboSpan\"").count();
    assert_eq!(
        span_count, 1,
        "double conversion should not duplicate spans"
    );
}

#[test]
fn fixed_layout_passthrough() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Page text.</p>")], Some("pre-paginated"));
    let kepub = convert(&epub);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "OEBPS/ch1.xhtml")).unwrap();
    assert!(
        !xhtml.contains("koboSpan"),
        "fixed-layout: no spans expected"
    );
    // Output is still a valid kepub-MIME deterministic ZIP.
    assert_eq!(read_zip_entry(&kepub, "mimetype"), b"application/kepub+zip");
}

#[test]
fn malformed_xhtml_per_doc_fallback() {
    // Two spine docs: one well-formed, one with a hand-crafted bad tag.
    let mut bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut bytes);
        let mut zip = zip::ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(CONTAINER_XML.as_bytes()).unwrap();

        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(opf(None, &["good.xhtml", "bad.xhtml"]).as_bytes())
            .unwrap();

        zip.start_file("OEBPS/good.xhtml", deflated).unwrap();
        zip.write_all(xhtml_body("<p>Good.</p>").as_bytes())
            .unwrap();

        // Mismatched tag — quick-xml strict parse will reject this.
        zip.start_file("OEBPS/bad.xhtml", deflated).unwrap();
        let bad = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body><p>Oops</body></html>"#;
        zip.write_all(bad.as_bytes()).unwrap();

        zip.finish().unwrap();
    }
    let kepub = convert(&bytes);
    let good = String::from_utf8(read_zip_entry(&kepub, "OEBPS/good.xhtml")).unwrap();
    assert!(good.contains("koboSpan"), "good doc should be wrapped");
    let bad = String::from_utf8(read_zip_entry(&kepub, "OEBPS/bad.xhtml")).unwrap();
    assert!(
        !bad.contains("koboSpan"),
        "bad doc should pass through untouched"
    );
}

#[test]
fn corrupt_zip_returns_error() {
    // A non-ZIP byte stream must produce a transform error.
    let result = KepubTransformer.transform(b"not a zip at all");
    assert!(result.is_err(), "non-zip must error: {result:?}");
}

#[test]
fn container_xml_with_whitespace_attrs_converts() {
    // Some EPUB writers emit `full-path = "..."` with surrounding whitespace
    // and/or single quotes — both are well-formed XML. Make sure we don't
    // miss the rootfile.
    let container = r#"<?xml version="1.0"?>
<container version='1.0' xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path = "OEBPS/content.opf"   media-type = 'application/oebps-package+xml'/>
  </rootfiles>
</container>"#;

    let mut bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut bytes);
        let mut zip = zip::ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(container.as_bytes()).unwrap();

        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(opf(None, &["ch1.xhtml"]).as_bytes()).unwrap();

        zip.start_file("OEBPS/ch1.xhtml", deflated).unwrap();
        zip.write_all(xhtml_body("<p>One.</p>").as_bytes()).unwrap();

        zip.finish().unwrap();
    }

    let kepub = convert(&bytes);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "OEBPS/ch1.xhtml")).unwrap();
    assert!(
        xhtml.contains("koboSpan"),
        "rootfile with whitespace/single-quoted attrs must still be located"
    );
}

#[test]
fn spine_href_with_parent_segment_is_rewritten() {
    // OPF lives at `OEBPS/sub/content.opf`; manifest item href is
    // `../Text/ch1.xhtml`, which must normalize to `OEBPS/Text/ch1.xhtml`.
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/sub/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Parent Href</dc:title>
    <dc:identifier id="uid">urn:uuid:fixture</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="../Text/ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;

    let mut bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut bytes);
        let mut zip = zip::ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(container.as_bytes()).unwrap();

        zip.start_file("OEBPS/sub/content.opf", deflated).unwrap();
        zip.write_all(opf.as_bytes()).unwrap();

        zip.start_file("OEBPS/Text/ch1.xhtml", deflated).unwrap();
        zip.write_all(xhtml_body("<p>Parent-segment reach.</p>").as_bytes())
            .unwrap();

        zip.finish().unwrap();
    }

    let kepub = convert(&bytes);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "OEBPS/Text/ch1.xhtml")).unwrap();
    assert!(
        xhtml.contains("koboSpan"),
        "spine href with `..` must resolve to the actual ZIP entry and be rewritten"
    );
    // The injected kobo.js href is relative to the chapter's *normalized*
    // directory (`OEBPS/Text`), so the script tag should walk up two levels.
    assert!(
        xhtml.contains(r#"src="../../kobo.js""#),
        "kobo.js script href should use normalized parent depth: {xhtml}"
    );
}

#[test]
fn spine_href_with_leading_slash_resolves_from_archive_root() {
    // OPF lives at `OEBPS/sub/content.opf`; manifest item href is
    // `/Text/ch1.xhtml`. The leading `/` means archive root, so the file
    // lives at `Text/ch1.xhtml` (NOT `OEBPS/sub/Text/ch1.xhtml`).
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/sub/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Root Href</dc:title>
    <dc:identifier id="uid">urn:uuid:fixture</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="/Text/ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;

    let mut bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut bytes);
        let mut zip = zip::ZipWriter::new(cursor);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(container.as_bytes()).unwrap();

        zip.start_file("OEBPS/sub/content.opf", deflated).unwrap();
        zip.write_all(opf.as_bytes()).unwrap();

        zip.start_file("Text/ch1.xhtml", deflated).unwrap();
        zip.write_all(xhtml_body("<p>Root-anchored.</p>").as_bytes())
            .unwrap();

        zip.finish().unwrap();
    }

    let kepub = convert(&bytes);
    let xhtml = String::from_utf8(read_zip_entry(&kepub, "Text/ch1.xhtml")).unwrap();
    assert!(
        xhtml.contains("koboSpan"),
        "leading-slash spine href must resolve from archive root and be rewritten"
    );
    // Chapter normalized location is `Text/`, depth 1 → script walks up once.
    assert!(
        xhtml.contains(r#"src="../kobo.js""#),
        "kobo.js href should reflect chapter's archive-root depth: {xhtml}"
    );
}

#[test]
fn opf_gets_kobo_manifest_entry() {
    let epub = build_epub(&[("ch1.xhtml", "<p>Hi.</p>")], None);
    let kepub = convert(&epub);
    let opf_bytes = read_zip_entry(&kepub, "OEBPS/content.opf");
    let opf_str = String::from_utf8(opf_bytes).unwrap();
    assert!(
        opf_str.contains(r#"id="kobo-js""#),
        "kobo.js manifest entry missing: {opf_str}"
    );
}
