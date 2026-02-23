//! PDF metadata extraction.
//!
//! Reads metadata from two sources within a PDF file:
//! 1. The **Info dictionary** (legacy, `/Title`, `/Author`, etc.)
//! 2. **XMP metadata** (Dublin Core / PRISM, embedded as an XML stream)
//!
//! When both sources supply a value for the same field, the XMP value wins
//! because XMP is richer and more standardised.

use archivis_core::errors::FormatError;
use archivis_core::models::MetadataSource;
// `IdentifierType` is used by tests (via `use super::*`).
#[cfg(test)]
use archivis_core::models::IdentifierType;
use lopdf::{Document, Object};

use crate::isbn_scan;
use crate::{ExtractedIdentifier, ExtractedMetadata};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract metadata from a PDF byte slice.
pub fn extract_pdf_metadata(data: &[u8]) -> Result<ExtractedMetadata, FormatError> {
    let doc = Document::load_mem(data).map_err(|e| FormatError::Parse {
        format: "PDF".into(),
        message: e.to_string(),
    })?;

    let mut meta = ExtractedMetadata {
        source: MetadataSource::Embedded,
        ..ExtractedMetadata::default()
    };

    // Page count is always available from the page tree.
    let page_count = doc.get_pages().len();
    meta.page_count = Some(i32::try_from(page_count).unwrap_or(i32::MAX));

    // Info dictionary (may not exist — that is OK).
    extract_info_dict(&doc, &mut meta);

    // XMP metadata (overrides Info dict values when present).
    extract_xmp(&doc, &mut meta);

    // Scan accumulated text fields for ISBN patterns.
    scan_for_isbns(&mut meta);

    Ok(meta)
}

// ---------------------------------------------------------------------------
// Info dictionary helpers
// ---------------------------------------------------------------------------

fn extract_info_dict(doc: &Document, meta: &mut ExtractedMetadata) {
    let Ok(info_id) = doc.trailer.get(b"Info").and_then(Object::as_reference) else {
        return;
    };

    let Ok(info) = doc.get_object(info_id).and_then(Object::as_dict) else {
        return;
    };

    if let Some(title) = pdf_dict_string(info, b"Title") {
        meta.title = Some(title);
    }

    if let Some(raw_author) = pdf_dict_string(info, b"Author") {
        meta.authors = split_authors(&raw_author);
    }

    if let Some(subject) = pdf_dict_string(info, b"Subject") {
        meta.description = Some(subject);
    }

    if let Some(keywords) = pdf_dict_string(info, b"Keywords") {
        meta.subjects = split_list(&keywords);
    }

    if let Some(date) = pdf_dict_string(info, b"CreationDate") {
        meta.publication_date = parse_pdf_date(&date);
    }
}

/// Decode a PDF string object from a dictionary key.
///
/// PDF strings may be Latin-1 or UTF-16BE (indicated by a BOM). We handle
/// both, trimming whitespace and returning `None` for empty values.
fn pdf_dict_string(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
    let obj = dict.get(key).ok()?;
    let bytes = obj.as_str().ok()?;
    let decoded = decode_pdf_bytes(bytes);
    let trimmed = decoded.trim().to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Decode raw PDF string bytes into a Rust `String`.
///
/// Handles UTF-16BE (with BOM `FE FF`) and falls back to Latin-1.
fn decode_pdf_bytes(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // UTF-16 BE
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        // Latin-1 (ISO 8859-1) — each byte maps directly to a Unicode codepoint.
        bytes.iter().map(|&b| b as char).collect()
    }
}

// ---------------------------------------------------------------------------
// XMP metadata helpers
// ---------------------------------------------------------------------------

fn extract_xmp(doc: &Document, meta: &mut ExtractedMetadata) {
    let Ok(catalog) = doc.catalog() else {
        return;
    };

    let metadata_ref = match catalog.get(b"Metadata") {
        Ok(Object::Reference(id)) => *id,
        _ => return,
    };

    let Ok(stream) = doc.get_object(metadata_ref).and_then(Object::as_stream) else {
        return;
    };

    let xml_bytes = stream.get_plain_content().unwrap_or_default();
    let xml = String::from_utf8_lossy(&xml_bytes);

    parse_xmp(&xml, meta);
}

/// Parse XMP XML and overlay values onto `meta`.
fn parse_xmp(xml: &str, meta: &mut ExtractedMetadata) {
    // dc:title
    if let Some(title) = xmp_single_value(xml, "dc:title") {
        meta.title = Some(title);
    }

    // dc:creator → authors
    if let Some(creators) = xmp_bag_values(xml, "dc:creator") {
        if !creators.is_empty() {
            meta.authors = creators;
        }
    }

    // dc:description
    if let Some(desc) = xmp_single_value(xml, "dc:description") {
        meta.description = Some(desc);
    }

    // dc:subject → subjects
    if let Some(subjects) = xmp_bag_values(xml, "dc:subject") {
        if !subjects.is_empty() {
            meta.subjects = subjects;
        }
    }

    // dc:language
    if let Some(lang) = xmp_bag_first(xml, "dc:language") {
        meta.language = Some(lang);
    }

    // dc:publisher
    if let Some(publisher) = xmp_bag_first(xml, "dc:publisher") {
        meta.publisher = Some(publisher);
    }

    // dc:date → publication_date
    if let Some(date) = xmp_bag_first(xml, "dc:date") {
        let trimmed = date.trim().to_owned();
        if !trimmed.is_empty() {
            meta.publication_date = Some(trimmed);
        }
    }

    // dc:identifier — look for ISBNs
    if let Some(identifiers) = xmp_bag_values(xml, "dc:identifier") {
        for id_val in identifiers {
            extract_isbn_from_text(&id_val, &mut meta.identifiers);
        }
    }

    // prism:isbn
    if let Some(isbn) = xmp_simple_element(xml, "prism:isbn") {
        extract_isbn_from_text(&isbn, &mut meta.identifiers);
    }
}

/// Extract the text content of a single-valued XMP element.
///
/// Handles both `<tag>text</tag>` and Alt-bag (`<tag><rdf:Alt><rdf:li ...>text</rdf:li></rdf:Alt></tag>`).
fn xmp_single_value(xml: &str, tag: &str) -> Option<String> {
    // First try Alt bag (common for dc:title, dc:description).
    if let Some(val) = xmp_alt_value(xml, tag) {
        return Some(val);
    }
    // Fall back to direct element text.
    xmp_simple_element(xml, tag)
}

/// Extract the first `rdf:li` from an `rdf:Alt` inside `<tag>`.
fn xmp_alt_value(xml: &str, tag: &str) -> Option<String> {
    let block = extract_tag_content(xml, tag)?;
    extract_rdf_li_first(&block)
}

/// Extract all `rdf:li` values from an `rdf:Bag` or `rdf:Seq` inside `<tag>`.
fn xmp_bag_values(xml: &str, tag: &str) -> Option<Vec<String>> {
    let block = extract_tag_content(xml, tag)?;
    let items = extract_all_rdf_li(&block);
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// Extract the first value from a Bag/Seq/Alt, or the plain element text.
fn xmp_bag_first(xml: &str, tag: &str) -> Option<String> {
    let block = extract_tag_content(xml, tag)?;
    if let Some(first) = extract_rdf_li_first(&block) {
        return Some(first);
    }
    let trimmed = block.trim().to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Extract direct element text for `<tag>text</tag>`.
fn xmp_simple_element(xml: &str, tag: &str) -> Option<String> {
    let content = extract_tag_content(xml, tag)?;
    // If it contains nested XML, skip.
    if content.contains('<') {
        return None;
    }
    let trimmed = content.trim().to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Low-level: extract text between `<tag...>` and `</tag>`.
///
/// Handles namespace-prefixed tags (e.g. `dc:title`).
fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    // Build patterns: `<dc:title` and `</dc:title>`
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");

    let open_start = xml.find(&open_prefix)?;
    let after_tag_name = open_start + open_prefix.len();
    let rest = &xml[after_tag_name..];

    // Find the end of the opening tag `>`.
    let gt = rest.find('>')?;
    let content_start = after_tag_name + gt + 1;

    let close_start = xml[content_start..].find(&close_tag)?;
    Some(xml[content_start..content_start + close_start].to_owned())
}

/// Extract the first `<rdf:li ...>text</rdf:li>` from a fragment.
fn extract_rdf_li_first(xml: &str) -> Option<String> {
    let items = extract_all_rdf_li(xml);
    items.into_iter().next()
}

/// Extract all `<rdf:li ...>text</rdf:li>` values from a fragment.
fn extract_all_rdf_li(xml: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(pos) = xml[search_from..].find("<rdf:li") {
        let open = search_from + pos;
        let Some(gt_offset) = xml[open..].find('>') else {
            break;
        };
        let gt = open + gt_offset + 1;
        let Some(close_offset) = xml[gt..].find("</rdf:li>") else {
            break;
        };
        let close = gt + close_offset;
        let text = xml[gt..close].trim().to_owned();
        if !text.is_empty() {
            results.push(text);
        }
        search_from = close + "</rdf:li>".len();
    }

    results
}

// ---------------------------------------------------------------------------
// Author / list splitting
// ---------------------------------------------------------------------------

/// Split an author string on common delimiters (`;`, `,`, ` and `).
fn split_authors(raw: &str) -> Vec<String> {
    // Split on semicolons first (strongest delimiter).
    let parts: Vec<&str> = raw.split(';').collect();
    let parts: Vec<String> = if parts.len() > 1 {
        parts.iter().map(|s| s.trim().to_owned()).collect()
    } else {
        // Try ` and `.
        let parts: Vec<&str> = raw.split(" and ").collect();
        if parts.len() > 1 {
            parts.iter().map(|s| s.trim().to_owned()).collect()
        } else {
            // Fall back to comma-splitting.
            raw.split(',').map(|s| s.trim().to_owned()).collect()
        }
    };
    parts.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Split a keyword / subject string on `;` or `,`.
fn split_list(raw: &str) -> Vec<String> {
    let delim = if raw.contains(';') { ';' } else { ',' };
    raw.split(delim)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// PDF date parsing
// ---------------------------------------------------------------------------

/// Parse a PDF date string (`D:YYYYMMDDHHmmSSOHH'mm'`) into a simple
/// date string (`YYYY-MM-DD`). Returns `None` on failure.
fn parse_pdf_date(raw: &str) -> Option<String> {
    // Strip optional `D:` prefix.
    let s = raw.strip_prefix("D:").unwrap_or(raw);

    // We need at least 4 characters (the year).
    if s.len() < 4 || !s[..4].chars().all(|c| c.is_ascii_digit()) {
        tracing::warn!(raw, "PDF date too short or non-numeric year");
        return None;
    }

    let year = &s[..4];
    let month = s
        .get(4..6)
        .filter(|m| m.chars().all(|c| c.is_ascii_digit()));
    let day = s
        .get(6..8)
        .filter(|d| d.chars().all(|c| c.is_ascii_digit()));

    match (month, day) {
        (Some(m), Some(d)) => Some(format!("{year}-{m}-{d}")),
        (Some(m), None) => Some(format!("{year}-{m}")),
        _ => Some(year.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// ISBN scanning
// ---------------------------------------------------------------------------

/// Scan title, description, and existing identifiers for ISBN patterns and
/// add any new ones to the metadata.
///
/// Delegates to the shared [`isbn_scan`] module.
fn scan_for_isbns(meta: &mut ExtractedMetadata) {
    let mut texts = Vec::new();
    if let Some(ref t) = meta.title {
        texts.push(t.clone());
    }
    if let Some(ref d) = meta.description {
        texts.push(d.clone());
    }
    for s in &meta.subjects {
        texts.push(s.clone());
    }

    let mut found: Vec<ExtractedIdentifier> = Vec::new();
    for text in &texts {
        extract_isbn_from_text(text, &mut found);
    }

    // De-duplicate against what we already have.
    for id in found {
        let dominated = meta.identifiers.iter().any(|existing| {
            existing.identifier_type == id.identifier_type && existing.value == id.value
        });
        if !dominated {
            meta.identifiers.push(id);
        }
    }
}

/// Look for ISBN-13 and ISBN-10 patterns in a text string.
///
/// Delegates to [`isbn_scan::scan_text_for_isbns`] with `require_checksum = false`
/// (matching the original behaviour of accepting any pattern-matched ISBN
/// regardless of checksum validity).
fn extract_isbn_from_text(text: &str, out: &mut Vec<ExtractedIdentifier>) {
    let scanned = isbn_scan::scan_text_for_isbns(text, false);
    let extracted = isbn_scan::to_extracted_identifiers(&scanned);

    for id in extracted {
        let already = out
            .iter()
            .any(|e| e.identifier_type == id.identifier_type && e.value == id.value);
        if !already {
            out.push(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::similar_names)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, Stream};

    /// Create a minimal valid PDF document with an Info dictionary.
    fn build_test_pdf(info_entries: Vec<(&str, Object)>) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");

        let pages_id = doc.new_object_id();

        // Minimal page.
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });

        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        // Info dictionary.
        if !info_entries.is_empty() {
            let mut info = lopdf::Dictionary::new();
            for (key, value) in info_entries {
                info.set(key, value);
            }
            let info_id = doc.add_object(Object::Dictionary(info));
            doc.trailer.set("Info", info_id);
        }

        let mut buf = Vec::new();
        doc.save_to(&mut buf).expect("save PDF");
        buf
    }

    /// Create a PDF with an XMP metadata stream.
    fn build_test_pdf_with_xmp(xmp_xml: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");

        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });

        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        // XMP stream.
        let xmp_stream = Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            xmp_xml.as_bytes().to_vec(),
        );
        let xmp_id = doc.add_object(xmp_stream);

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "Metadata" => xmp_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut buf = Vec::new();
        doc.save_to(&mut buf).expect("save PDF");
        buf
    }

    #[test]
    fn extract_title_and_author() {
        let data = build_test_pdf(vec![
            (
                "Title",
                Object::string_literal("The Rust Programming Language"),
            ),
            (
                "Author",
                Object::string_literal("Steve Klabnik; Carol Nichols"),
            ),
        ]);
        let meta = extract_pdf_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("The Rust Programming Language"));
        assert_eq!(meta.authors, vec!["Steve Klabnik", "Carol Nichols"]);
        assert_eq!(meta.source, MetadataSource::Embedded);
    }

    #[test]
    fn extract_keywords_as_subjects() {
        let data = build_test_pdf(vec![(
            "Keywords",
            Object::string_literal("Rust; Programming; Systems"),
        )]);
        let meta = extract_pdf_metadata(&data).unwrap();

        assert_eq!(meta.subjects, vec!["Rust", "Programming", "Systems"]);
    }

    #[test]
    fn extract_page_count() {
        let data = build_test_pdf(vec![]);
        let meta = extract_pdf_metadata(&data).unwrap();

        assert_eq!(meta.page_count, Some(1));
    }

    #[test]
    fn parse_pdf_date_full() {
        assert_eq!(
            parse_pdf_date("D:20231215120000+05'30'"),
            Some("2023-12-15".into()),
        );
    }

    #[test]
    fn parse_pdf_date_year_only() {
        assert_eq!(parse_pdf_date("D:2020"), Some("2020".into()));
    }

    #[test]
    fn parse_pdf_date_no_prefix() {
        assert_eq!(parse_pdf_date("20180315"), Some("2018-03-15".into()));
    }

    #[test]
    fn parse_pdf_date_invalid() {
        assert_eq!(parse_pdf_date("abc"), None);
    }

    #[test]
    fn creation_date_extracted() {
        let data = build_test_pdf(vec![(
            "CreationDate",
            Object::string_literal("D:20220601093000Z"),
        )]);
        let meta = extract_pdf_metadata(&data).unwrap();

        assert_eq!(meta.publication_date.as_deref(), Some("2022-06-01"));
    }

    #[test]
    fn no_info_dict_returns_page_count_only() {
        let data = build_test_pdf(vec![]);
        let meta = extract_pdf_metadata(&data).unwrap();

        assert!(meta.title.is_none());
        assert!(meta.authors.is_empty());
        assert_eq!(meta.page_count, Some(1));
    }

    #[test]
    fn author_split_on_and() {
        assert_eq!(split_authors("Alice and Bob"), vec!["Alice", "Bob"],);
    }

    #[test]
    fn author_split_on_comma() {
        assert_eq!(
            split_authors("Alice, Bob, Charlie"),
            vec!["Alice", "Bob", "Charlie"],
        );
    }

    #[test]
    fn author_single() {
        assert_eq!(split_authors("Alice"), vec!["Alice"]);
    }

    #[test]
    fn isbn_extraction_from_text() {
        let mut out = Vec::new();
        extract_isbn_from_text("ISBN 978-3-16-148410-0", &mut out);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(out[0].value, "9783161484100");
    }

    #[test]
    fn isbn10_extraction() {
        let mut out = Vec::new();
        extract_isbn_from_text("ISBN 0-306-40615-2", &mut out);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].identifier_type, IdentifierType::Isbn10);
        assert_eq!(out[0].value, "0306406152");
    }

    #[test]
    fn isbn_deduplication() {
        let mut out = Vec::new();
        extract_isbn_from_text("978-3-16-148410-0 978-3-16-148410-0", &mut out);

        assert_eq!(out.len(), 1);
    }

    #[test]
    fn xmp_metadata_extraction() {
        let xmp = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:prism="http://prismstandard.org/namespaces/basic/2.0/">
    <dc:title>
      <rdf:Alt>
        <rdf:li xml:lang="x-default">XMP Title Override</rdf:li>
      </rdf:Alt>
    </dc:title>
    <dc:creator>
      <rdf:Seq>
        <rdf:li>Author One</rdf:li>
        <rdf:li>Author Two</rdf:li>
      </rdf:Seq>
    </dc:creator>
    <dc:description>
      <rdf:Alt>
        <rdf:li xml:lang="x-default">A fine description.</rdf:li>
      </rdf:Alt>
    </dc:description>
    <dc:subject>
      <rdf:Bag>
        <rdf:li>Science</rdf:li>
        <rdf:li>Fiction</rdf:li>
      </rdf:Bag>
    </dc:subject>
    <dc:language>
      <rdf:Bag>
        <rdf:li>en</rdf:li>
      </rdf:Bag>
    </dc:language>
    <dc:publisher>
      <rdf:Bag>
        <rdf:li>Great Publisher</rdf:li>
      </rdf:Bag>
    </dc:publisher>
    <dc:date>
      <rdf:Seq>
        <rdf:li>2023-07-01</rdf:li>
      </rdf:Seq>
    </dc:date>
    <dc:identifier>
      <rdf:Bag>
        <rdf:li>urn:isbn:9780136019701</rdf:li>
      </rdf:Bag>
    </dc:identifier>
    <prism:isbn>978-0-13-601970-1</prism:isbn>
  </rdf:Description>
</rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

        let data = build_test_pdf_with_xmp(xmp);
        let meta = extract_pdf_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("XMP Title Override"));
        assert_eq!(meta.authors, vec!["Author One", "Author Two"]);
        assert_eq!(meta.description.as_deref(), Some("A fine description."));
        assert_eq!(meta.subjects, vec!["Science", "Fiction"]);
        assert_eq!(meta.language.as_deref(), Some("en"));
        assert_eq!(meta.publisher.as_deref(), Some("Great Publisher"));
        assert_eq!(meta.publication_date.as_deref(), Some("2023-07-01"));
        // ISBN from dc:identifier and prism:isbn (deduplicated).
        assert!(meta.identifiers.iter().any(|id| {
            id.identifier_type == IdentifierType::Isbn13 && id.value == "9780136019701"
        }));
    }

    #[test]
    fn xmp_overrides_info_dict() {
        // Build a PDF with both Info dict and XMP.
        let mut doc = Document::with_version("1.5");

        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let info = dictionary! {
            "Title" => Object::string_literal("Info Title"),
            "Author" => Object::string_literal("Info Author"),
        };
        let info_id = doc.add_object(Object::Dictionary(info));
        doc.trailer.set("Info", info_id);

        let xmp = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title><rdf:Alt><rdf:li xml:lang="x-default">XMP Title</rdf:li></rdf:Alt></dc:title>
  </rdf:Description>
</rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;
        let xmp_stream = Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            xmp.as_bytes().to_vec(),
        );
        let xmp_id = doc.add_object(xmp_stream);

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "Metadata" => xmp_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut buf = Vec::new();
        doc.save_to(&mut buf).expect("save PDF");

        let meta = extract_pdf_metadata(&buf).unwrap();

        // Title should come from XMP (overrides Info dict).
        assert_eq!(meta.title.as_deref(), Some("XMP Title"));
        // Author stays from Info dict since XMP didn't provide dc:creator.
        assert_eq!(meta.authors, vec!["Info Author"]);
    }

    #[test]
    fn invalid_pdf_returns_error() {
        let result = extract_pdf_metadata(b"not a pdf");
        assert!(result.is_err());
        match result.unwrap_err() {
            FormatError::Parse { format, .. } => assert_eq!(format, "PDF"),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn utf16be_string_decoding() {
        // UTF-16BE BOM + "Hi"
        let bytes: Vec<u8> = vec![0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69];
        assert_eq!(decode_pdf_bytes(&bytes), "Hi");
    }

    #[test]
    fn latin1_string_decoding() {
        // Latin-1: "caf\xe9" → "café"
        let bytes: Vec<u8> = vec![0x63, 0x61, 0x66, 0xe9];
        assert_eq!(decode_pdf_bytes(&bytes), "caf\u{e9}");
    }

    #[test]
    fn extract_rdf_li_values() {
        let fragment = r"
            <rdf:Seq>
                <rdf:li>First</rdf:li>
                <rdf:li>Second</rdf:li>
                <rdf:li>Third</rdf:li>
            </rdf:Seq>
        ";
        let items = extract_all_rdf_li(fragment);
        assert_eq!(items, vec!["First", "Second", "Third"]);
    }

    #[test]
    fn split_list_semicolons() {
        assert_eq!(
            split_list("Fiction; Science; History"),
            vec!["Fiction", "Science", "History"],
        );
    }

    #[test]
    fn split_list_commas() {
        assert_eq!(
            split_list("Fiction, Science, History"),
            vec!["Fiction", "Science", "History"],
        );
    }
}
