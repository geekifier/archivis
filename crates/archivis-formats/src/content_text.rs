//! Per-format content text extraction.
//!
//! Extracts readable text from the *content* of ebook files (as opposed to
//! embedded metadata). Used by the content-scan ISBN pipeline to find ISBNs
//! printed on title pages, copyright pages, etc.
//!
//! Only a configurable subset of each book is read (front + back pages/sections)
//! to keep processing fast and memory-bounded.

use std::io::Cursor;

use archivis_core::errors::FormatError;
use archivis_core::models::BookFormat;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::epub::{find_attr, find_opf_path, local_name, opf_directory, read_zip_entry};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Controls how much content to read from each format.
#[derive(Debug, Clone)]
pub struct ContentScanConfig {
    /// Number of EPUB spine items to read from front and back (default: 3).
    pub epub_spine_items: usize,
    /// Number of PDF pages to read from front and back (default: 5).
    pub pdf_pages: usize,
    /// Number of FB2 `<section>` elements to read from front and back (default: 3).
    pub fb2_sections: usize,
    /// Bytes to read from front and back of TXT files (default: 4000).
    pub txt_bytes: usize,
    /// Bytes to read from front and back of MOBI/AZW3 text (default: 8000).
    /// Higher than TXT because MOBI text may contain HTML markup that inflates size.
    pub mobi_bytes: usize,
}

impl Default for ContentScanConfig {
    fn default() -> Self {
        Self {
            epub_spine_items: 3,
            pdf_pages: 5,
            fb2_sections: 3,
            txt_bytes: 4000,
            mobi_bytes: 8000,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract readable text content from an ebook file.
///
/// Reads a limited portion of the book (front and back) as configured.
/// Returns `Ok(None)` for formats that are not supported for content scanning
/// (CBZ, DJVU, Unknown).
///
/// # Errors
///
/// Returns `FormatError` if the file data is corrupt or cannot be parsed.
pub fn extract_content_text(
    data: &[u8],
    format: BookFormat,
    config: &ContentScanConfig,
) -> Result<Option<String>, FormatError> {
    match format {
        BookFormat::Epub => extract_epub_text(data, config).map(Some),
        BookFormat::Pdf => extract_pdf_text(data, config).map(Some),
        BookFormat::Fb2 => extract_fb2_text(data, config).map(Some),
        BookFormat::Txt => Ok(Some(extract_txt_text(data, config))),
        BookFormat::Mobi | BookFormat::Azw3 => {
            let raw = crate::mobi::extract_mobi_text(data, config.mobi_bytes)?;
            let mut text = String::new();
            strip_html_tags(&raw, &mut text);
            Ok(Some(text))
        }
        BookFormat::Cbz | BookFormat::Djvu | BookFormat::Unknown => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// EPUB content extraction
// ---------------------------------------------------------------------------

/// Extract text from EPUB spine items (front N + back N).
fn extract_epub_text(data: &[u8], config: &ContentScanConfig) -> Result<String, FormatError> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| FormatError::Parse {
        format: "EPUB".into(),
        message: format!("invalid ZIP archive: {e}"),
    })?;

    let opf_path = find_opf_path(&mut archive)?;
    let opf_content = read_zip_entry(&mut archive, &opf_path)?;
    let opf_dir = opf_directory(&opf_path);

    // Parse spine to get reading-order item references.
    let spine_hrefs = parse_spine_hrefs(&opf_content, &opf_dir)?;

    if spine_hrefs.is_empty() {
        return Ok(String::new());
    }

    // Select front N and back N spine items.
    let indices = front_back_indices(spine_hrefs.len(), config.epub_spine_items);

    let mut text = String::new();
    for idx in indices {
        let href = &spine_hrefs[idx];
        if let Ok(html) = read_zip_entry(&mut archive, href) {
            strip_html_tags(&html, &mut text);
            text.push('\n');
        }
        // Missing spine item — skip gracefully.
    }

    Ok(text)
}

/// Parse the OPF `<spine>` element to get an ordered list of content file paths.
///
/// Maps each `<itemref idref="...">` to its corresponding `<item href="...">`
/// in the `<manifest>`.
fn parse_spine_hrefs(opf_xml: &str, opf_dir: &str) -> Result<Vec<String>, FormatError> {
    let mut reader = Reader::from_str(opf_xml);

    // First pass: collect manifest items (id → href).
    let mut manifest: Vec<(String, String)> = Vec::new();
    let mut in_manifest = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if name == b"manifest" {
                    in_manifest = true;
                }
                if in_manifest && name == b"item" {
                    if let (Some(id), Some(href)) = (find_attr(e, b"id"), find_attr(e, b"href")) {
                        manifest.push((id, href));
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let qname = e.name();
                if local_name(qname.as_ref()) == b"manifest" {
                    in_manifest = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FormatError::Parse {
                    format: "EPUB".into(),
                    message: format!("error parsing OPF manifest: {e}"),
                });
            }
            _ => {}
        }
    }

    // Second pass: collect spine idrefs in order.
    let mut reader = Reader::from_str(opf_xml);
    let mut idrefs: Vec<String> = Vec::new();
    let mut in_spine = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if name == b"spine" {
                    in_spine = true;
                }
                if in_spine && name == b"itemref" {
                    if let Some(idref) = find_attr(e, b"idref") {
                        idrefs.push(idref);
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let qname = e.name();
                if local_name(qname.as_ref()) == b"spine" {
                    in_spine = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FormatError::Parse {
                    format: "EPUB".into(),
                    message: format!("error parsing OPF spine: {e}"),
                });
            }
            _ => {}
        }
    }

    // Resolve idrefs to file paths.
    let hrefs: Vec<String> = idrefs
        .iter()
        .filter_map(|idref| {
            manifest
                .iter()
                .find(|(id, _)| id == idref)
                .map(|(_, href)| {
                    href.strip_prefix('/')
                        .map_or_else(|| format!("{opf_dir}{href}"), ToOwned::to_owned)
                })
        })
        .collect();

    Ok(hrefs)
}

/// Strip HTML/XHTML tags from content, collecting only text nodes.
pub(crate) fn strip_html_tags(html: &str, out: &mut String) {
    let mut reader = Reader::from_str(html);
    loop {
        match reader.read_event() {
            Ok(Event::Text(ref t)) => {
                let text = t.unescape().unwrap_or_default();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                        out.push(' ');
                    }
                    out.push_str(trimmed);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// PDF content extraction
// ---------------------------------------------------------------------------

/// Extract text from PDF pages (front N + back N).
fn extract_pdf_text(data: &[u8], config: &ContentScanConfig) -> Result<String, FormatError> {
    let doc = lopdf::Document::load_mem(data).map_err(|e| FormatError::Parse {
        format: "PDF".into(),
        message: e.to_string(),
    })?;

    let pages = doc.get_pages();
    let page_count = pages.len();

    if page_count == 0 {
        return Ok(String::new());
    }

    // Collect page numbers in sorted order.
    let mut page_numbers: Vec<u32> = pages.keys().copied().collect();
    page_numbers.sort_unstable();

    // Select front N and back N page indices.
    let indices = front_back_indices(page_numbers.len(), config.pdf_pages);

    let mut text = String::new();
    for idx in indices {
        let page_num = page_numbers[idx];
        if let Ok(page_text) = doc.extract_text(&[page_num]) {
            text.push_str(&page_text);
            text.push('\n');
        }
        // Some pages may lack extractable text (scanned images, etc.).
    }

    Ok(text)
}

// ---------------------------------------------------------------------------
// FB2 content extraction
// ---------------------------------------------------------------------------

/// Extract text from FB2 `<body>` `<section>` elements (front N + back N).
fn extract_fb2_text(data: &[u8], config: &ContentScanConfig) -> Result<String, FormatError> {
    // Handle optional UTF-8 BOM.
    let start = if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };

    let xml = std::str::from_utf8(&data[start..]).map_err(|e| FormatError::Parse {
        format: "FB2".into(),
        message: format!("invalid UTF-8: {e}"),
    })?;

    // Collect text from each <section> inside <body>.
    let sections = parse_fb2_sections(xml)?;

    if sections.is_empty() {
        return Ok(String::new());
    }

    let indices = front_back_indices(sections.len(), config.fb2_sections);

    let mut text = String::new();
    for idx in indices {
        text.push_str(&sections[idx]);
        text.push('\n');
    }

    Ok(text)
}

/// Parse FB2 XML and return the text content of each `<section>` under `<body>`.
fn parse_fb2_sections(xml: &str) -> Result<Vec<String>, FormatError> {
    let mut reader = Reader::from_str(xml);
    let mut sections: Vec<String> = Vec::new();
    let mut in_body = false;
    let mut section_depth: usize = 0;
    let mut current_section = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if name == b"body" {
                    in_body = true;
                } else if in_body && name == b"section" {
                    section_depth += 1;
                }
            }
            Ok(Event::Text(ref t)) => {
                if in_body && section_depth > 0 {
                    let text = t.unescape().unwrap_or_default();
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if !current_section.is_empty() {
                            current_section.push(' ');
                        }
                        current_section.push_str(trimmed);
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if name == b"body" {
                    in_body = false;
                } else if in_body && name == b"section" && section_depth > 0 {
                    section_depth -= 1;
                    if section_depth == 0 && !current_section.is_empty() {
                        sections.push(std::mem::take(&mut current_section));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FormatError::Parse {
                    format: "FB2".into(),
                    message: format!("error parsing FB2 XML: {e}"),
                });
            }
            _ => {}
        }
    }

    Ok(sections)
}

// ---------------------------------------------------------------------------
// TXT content extraction
// ---------------------------------------------------------------------------

/// Extract text from front and back of a TXT file.
fn extract_txt_text(data: &[u8], config: &ContentScanConfig) -> String {
    let len = data.len();

    if len <= config.txt_bytes * 2 {
        // File is small enough to read entirely.
        return String::from_utf8_lossy(data).into_owned();
    }

    let front = String::from_utf8_lossy(&data[..config.txt_bytes]);
    let back = String::from_utf8_lossy(&data[len - config.txt_bytes..]);

    format!("{front}\n{back}")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute deduplicated indices for the first N and last N items from a
/// collection of `total` items.
///
/// If `total <= n * 2`, returns all indices `0..total`.
fn front_back_indices(total: usize, n: usize) -> Vec<usize> {
    if total <= n * 2 {
        return (0..total).collect();
    }

    let mut indices: Vec<usize> = (0..n).collect();
    for i in (total - n)..total {
        if !indices.contains(&i) {
            indices.push(i);
        }
    }
    indices
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_back_indices_small() {
        assert_eq!(front_back_indices(4, 3), vec![0, 1, 2, 3]);
    }

    #[test]
    fn front_back_indices_exact() {
        assert_eq!(front_back_indices(6, 3), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn front_back_indices_large() {
        let result = front_back_indices(20, 3);
        assert_eq!(result, vec![0, 1, 2, 17, 18, 19]);
    }

    #[test]
    fn front_back_indices_single() {
        assert_eq!(front_back_indices(1, 3), vec![0]);
    }

    #[test]
    fn txt_extraction_small() {
        let data = b"Hello, world!";
        let config = ContentScanConfig::default();
        let result = extract_txt_text(data, &config);
        assert_eq!(result, "Hello, world!");
    }

    #[test]
    fn txt_extraction_large() {
        // Create data larger than 2 * txt_bytes.
        let config = ContentScanConfig {
            txt_bytes: 10,
            ..ContentScanConfig::default()
        };
        let data = b"FRONT_PART_middle_padding_that_is_long_enough_BACK__PART";
        let result = extract_txt_text(data, &config);
        assert!(result.starts_with("FRONT_PART"));
        assert!(result.ends_with("BACK__PART"));
    }

    #[test]
    fn strip_html_basic() {
        let html = "<html><body><p>Hello</p><p>World</p></body></html>";
        let mut out = String::new();
        strip_html_tags(html, &mut out);
        assert!(out.contains("Hello"));
        assert!(out.contains("World"));
    }

    #[test]
    fn unsupported_format_returns_none() {
        let config = ContentScanConfig::default();

        let result = extract_content_text(b"data", BookFormat::Cbz, &config).unwrap();
        assert!(result.is_none());

        let result = extract_content_text(b"data", BookFormat::Djvu, &config).unwrap();
        assert!(result.is_none());

        let result = extract_content_text(b"data", BookFormat::Unknown, &config).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn fb2_section_extraction() {
        let fb2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
  <body>
    <section>
      <p>First section text.</p>
    </section>
    <section>
      <p>Second section text.</p>
    </section>
    <section>
      <p>Third section text.</p>
    </section>
  </body>
</FictionBook>"#;

        let config = ContentScanConfig {
            fb2_sections: 2,
            ..ContentScanConfig::default()
        };
        let result = extract_fb2_text(fb2.as_bytes(), &config).unwrap();
        assert!(result.contains("First section text."));
        assert!(result.contains("Third section text."));
    }

    #[test]
    fn fb2_all_sections_when_few() {
        let fb2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
  <body>
    <section><p>Only section.</p></section>
  </body>
</FictionBook>"#;

        let config = ContentScanConfig::default();
        let result = extract_fb2_text(fb2.as_bytes(), &config).unwrap();
        assert!(result.contains("Only section."));
    }

    #[test]
    fn txt_format_extraction() {
        let data = b"This is a plain text book with ISBN 978-3-16-148410-0 on the title page.";
        let config = ContentScanConfig::default();
        let result = extract_content_text(data, BookFormat::Txt, &config).unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().contains("978-3-16-148410-0"));
    }
}
