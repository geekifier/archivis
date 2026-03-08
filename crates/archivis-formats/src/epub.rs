use std::collections::HashMap;
use std::io::{Cursor, Read};

use archivis_core::errors::FormatError;
use archivis_core::models::{IdentifierType, MetadataSource};
use quick_xml::events::Event;
use quick_xml::Reader;
use tracing::{debug, warn};

use crate::{CoverData, ExtractedIdentifier, ExtractedMetadata};

/// Extract metadata from an EPUB file provided as raw bytes.
///
/// Parses `META-INF/container.xml` to locate the OPF package document,
/// then extracts Dublin Core metadata, Calibre/EPUB 3 series info,
/// identifiers, and the cover image from the ZIP archive.
///
/// # Errors
///
/// Returns `FormatError::Parse` if the ZIP archive or XML is invalid,
/// or if required entries like `container.xml` or the OPF are missing.
pub fn extract_epub_metadata(data: &[u8]) -> Result<ExtractedMetadata, FormatError> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| FormatError::Parse {
        format: "EPUB".into(),
        message: format!("invalid ZIP archive: {e}"),
    })?;

    let opf_path = find_opf_path(&mut archive)?;
    debug!(opf_path = %opf_path, "located OPF package document");

    let opf_content = read_zip_entry(&mut archive, &opf_path)?;
    let opf_dir = opf_directory(&opf_path);

    let mut metadata = parse_opf(&opf_content)?;

    // Attempt cover extraction; log a warning on failure but don't propagate.
    match extract_cover(&opf_content, &opf_dir, &mut archive) {
        Ok(Some(cover)) => {
            debug!("extracted cover image");
            metadata.cover_image = Some(cover);
        }
        Ok(None) => {
            debug!("no cover image reference found in OPF");
        }
        Err(e) => {
            warn!("cover extraction failed, continuing without cover: {e}");
        }
    }

    metadata.source = MetadataSource::Embedded;
    Ok(metadata)
}

// ── Container / OPF location ─────────────────────────────────────────

/// Read `META-INF/container.xml` and return the `full-path` of the root OPF file.
pub(crate) fn find_opf_path(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
) -> Result<String, FormatError> {
    let xml = read_zip_entry(archive, "META-INF/container.xml")?;
    let mut reader = Reader::from_str(&xml);

    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e) | Event::Start(ref e))
                if local_name(e.name().as_ref()) == b"rootfile" =>
            {
                for attr in e.attributes().flatten() {
                    if local_name(attr.key.as_ref()) == b"full-path" {
                        let path = String::from_utf8_lossy(&attr.value).into_owned();
                        if path.is_empty() {
                            return Err(FormatError::Parse {
                                format: "EPUB".into(),
                                message: "rootfile full-path is empty".into(),
                            });
                        }
                        return Ok(path);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FormatError::Parse {
                    format: "EPUB".into(),
                    message: format!("malformed container.xml: {e}"),
                });
            }
            _ => {}
        }
    }

    Err(FormatError::Parse {
        format: "EPUB".into(),
        message: "no rootfile found in container.xml".into(),
    })
}

/// Return the directory portion of the OPF path (for resolving relative references).
pub(crate) fn opf_directory(opf_path: &str) -> String {
    opf_path
        .rfind('/')
        .map_or_else(String::new, |pos| format!("{}/", &opf_path[..pos]))
}

// ── OPF parsing ──────────────────────────────────────────────────────

/// Mutable state carried through the OPF parsing loop.
struct OpfParseState {
    current_element: Option<DcElement>,
    in_metadata: bool,
    current_opf_role: Option<String>,
    current_opf_scheme: Option<String>,
    meta_name: Option<String>,
    meta_content: Option<String>,
    meta_property: Option<String>,
    format_version: Option<String>,
    /// The `id` attribute of the current `<dc:title>` element being parsed.
    current_title_id: Option<String>,
    /// Collected `<dc:title>` elements: (optional id, text).
    title_candidates: Vec<(Option<String>, String)>,
    /// Maps `#id` → `title-type` value from `<meta property="title-type" refines="#id">`.
    title_type_refinements: HashMap<String, String>,
    /// The `refines` attribute of the current `<meta>` element.
    meta_refines: Option<String>,
}

/// Parse the OPF package document and extract metadata.
fn parse_opf(opf_xml: &str) -> Result<ExtractedMetadata, FormatError> {
    let mut meta = ExtractedMetadata::default();
    let mut reader = Reader::from_str(opf_xml);
    let mut state = OpfParseState {
        current_element: None,
        in_metadata: false,
        current_opf_role: None,
        current_opf_scheme: None,
        meta_name: None,
        meta_content: None,
        meta_property: None,
        format_version: None,
        current_title_id: None,
        title_candidates: Vec::new(),
        title_type_refinements: HashMap::new(),
        meta_refines: None,
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => handle_opf_start(e, &mut state),
            Ok(Event::Empty(ref e)) if state.in_metadata => {
                handle_opf_empty(e, &mut state, &mut meta);
            }
            Ok(Event::Text(ref t)) => {
                handle_opf_text(t, &mut state, &mut meta);
            }
            Ok(Event::End(ref e)) => handle_opf_end(e, &mut state, &mut meta),
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FormatError::Parse {
                    format: "EPUB".into(),
                    message: format!("malformed OPF XML: {e}"),
                });
            }
            _ => {}
        }
    }

    meta.format_version = state.format_version;

    // Resolve title and subtitle from EPUB 3 title-type refinements.
    resolve_titles(
        &mut meta,
        &state.title_candidates,
        &state.title_type_refinements,
    );

    Ok(meta)
}

fn handle_opf_start(e: &quick_xml::events::BytesStart<'_>, state: &mut OpfParseState) {
    let qname = e.name();
    let name = local_name(qname.as_ref());

    if name == b"package" && state.format_version.is_none() {
        state.format_version = find_attr(e, b"version");
    }

    if name == b"metadata" {
        state.in_metadata = true;
        return;
    }

    if !state.in_metadata {
        return;
    }

    match name {
        b"title" => {
            state.current_element = Some(DcElement::Title);
            state.current_title_id = find_attr(e, b"id");
        }
        b"creator" => {
            state.current_element = Some(DcElement::Creator);
            state.current_opf_role = find_attr(e, b"role");
        }
        b"language" => state.current_element = Some(DcElement::Language),
        b"identifier" => {
            state.current_element = Some(DcElement::Identifier);
            state.current_opf_scheme = find_attr(e, b"scheme");
        }
        b"publisher" => state.current_element = Some(DcElement::Publisher),
        b"date" => state.current_element = Some(DcElement::Date),
        b"description" => state.current_element = Some(DcElement::Description),
        b"subject" => state.current_element = Some(DcElement::Subject),
        b"source" => state.current_element = Some(DcElement::Source),
        b"meta" => {
            state.meta_name = find_attr(e, b"name");
            state.meta_property = find_attr(e, b"property");
            state.meta_refines = find_attr(e, b"refines");
            state.current_element = Some(DcElement::Meta);
        }
        _ => {}
    }
}

fn handle_opf_empty(
    e: &quick_xml::events::BytesStart<'_>,
    _state: &mut OpfParseState,
    meta: &mut ExtractedMetadata,
) {
    let qname = e.name();
    let name = local_name(qname.as_ref());
    if name == b"meta" {
        let mn = find_attr(e, b"name");
        let mc = find_attr(e, b"content");
        process_meta_element(mn.as_deref(), mc.as_deref(), None, meta);
    }
}

fn handle_opf_text(
    t: &quick_xml::events::BytesText<'_>,
    state: &mut OpfParseState,
    meta: &mut ExtractedMetadata,
) {
    let Some(ref elem) = state.current_element else {
        return;
    };
    let text = t.unescape().unwrap_or_default().trim().to_owned();
    if text.is_empty() {
        return;
    }
    match elem {
        DcElement::Title => {
            state
                .title_candidates
                .push((state.current_title_id.take(), text));
        }
        DcElement::Creator => {
            let dominated_by_role = state
                .current_opf_role
                .as_deref()
                .is_some_and(|r| !r.eq_ignore_ascii_case("aut"));
            if !dominated_by_role {
                meta.authors.push(text);
            }
        }
        DcElement::Language => meta.language = Some(text),
        DcElement::Identifier => {
            parse_identifier(&text, state.current_opf_scheme.as_deref(), meta);
        }
        DcElement::Publisher => meta.publisher = Some(text),
        DcElement::Date => meta.publication_year = crate::parse_year_from_date_str(&text),
        DcElement::Description => meta.description = Some(text),
        DcElement::Subject => meta.subjects.push(text),
        DcElement::Source => {
            parse_identifier(&text, None, meta);
        }
        DcElement::Meta => {
            state.meta_content = Some(text);
        }
    }
}

fn handle_opf_end(
    e: &quick_xml::events::BytesEnd<'_>,
    state: &mut OpfParseState,
    meta: &mut ExtractedMetadata,
) {
    let qname = e.name();
    let name = local_name(qname.as_ref());

    if name == b"metadata" {
        state.in_metadata = false;
    }

    if name == b"meta" && state.current_element == Some(DcElement::Meta) {
        // Capture EPUB 3 title-type refinements: <meta property="title-type" refines="#id">main|subtitle</meta>
        if state.meta_property.as_deref() == Some("title-type") {
            if let (Some(refines), Some(ref content)) = (&state.meta_refines, &state.meta_content) {
                let id = refines.strip_prefix('#').unwrap_or(refines).to_owned();
                state
                    .title_type_refinements
                    .insert(id, content.to_lowercase());
            }
        }

        process_meta_element(
            state.meta_name.as_deref(),
            state.meta_content.as_deref(),
            state.meta_property.as_deref(),
            meta,
        );
        state.meta_name = None;
        state.meta_property = None;
        state.meta_content = None;
        state.meta_refines = None;
    }

    state.current_element = None;
    state.current_opf_role = None;
    state.current_opf_scheme = None;
}

/// Dublin Core elements we track while parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DcElement {
    Title,
    Creator,
    Language,
    Identifier,
    Publisher,
    Date,
    Description,
    Subject,
    Source,
    Meta,
}

/// Process a `<meta>` element from either EPUB 2 or EPUB 3 format.
fn process_meta_element(
    name: Option<&str>,
    content: Option<&str>,
    property: Option<&str>,
    meta: &mut ExtractedMetadata,
) {
    // EPUB 2: <meta name="calibre:series" content="..."/>
    if let (Some(n), Some(c)) = (name, content) {
        match n {
            "calibre:series" => meta.series = Some(c.to_owned()),
            "calibre:series_index" => {
                if let Ok(pos) = c.parse::<f32>() {
                    meta.series_position = Some(pos);
                }
            }
            _ => {}
        }
    }

    // EPUB 3: <meta property="belongs-to-collection">...</meta>
    //         <meta property="group-position">...</meta>
    if let (Some(prop), Some(val)) = (property, content) {
        match prop {
            "belongs-to-collection" => {
                if meta.series.is_none() {
                    meta.series = Some(val.to_owned());
                }
            }
            "group-position" => {
                if meta.series_position.is_none() {
                    if let Ok(pos) = val.parse::<f32>() {
                        meta.series_position = Some(pos);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Resolve title and subtitle from collected `<dc:title>` candidates and
/// EPUB 3 `title-type` refinements.
///
/// If refinements map a candidate to `main`, it becomes the title.
/// If one maps to `subtitle`, it becomes the subtitle.
/// When no refinements exist, the first candidate is used as the title
/// (backwards-compatible with EPUB 2).
fn resolve_titles(
    meta: &mut ExtractedMetadata,
    candidates: &[(Option<String>, String)],
    refinements: &HashMap<String, String>,
) {
    if candidates.is_empty() {
        return;
    }

    if refinements.is_empty() {
        // No EPUB 3 refinements — use first candidate as title.
        meta.title = Some(candidates[0].1.clone());
        return;
    }

    // Find candidates by their title-type refinement.
    for (id, text) in candidates {
        if let Some(ref title_id) = id {
            if let Some(title_type) = refinements.get(title_id) {
                match title_type.as_str() {
                    "main" => meta.title = Some(text.clone()),
                    "subtitle" => meta.subtitle = Some(text.clone()),
                    _ => {}
                }
            }
        }
    }

    // If no candidate was marked as "main", fall back to the first candidate.
    if meta.title.is_none() {
        meta.title = Some(candidates[0].1.clone());
    }
}

// ── Identifier parsing ───────────────────────────────────────────────

/// Parse an identifier value and add it to metadata.
///
/// Uses the `opf:scheme` attribute hint when available, then falls back
/// to pattern-matching the raw value for ISBN-10, ISBN-13, and ASIN.
fn parse_identifier(raw: &str, scheme: Option<&str>, meta: &mut ExtractedMetadata) {
    let normalized = raw.replace(['-', ' '], "");

    // If scheme explicitly says ISBN, try to classify.
    if let Some(s) = scheme {
        let s_upper = s.to_uppercase();
        if s_upper.contains("ISBN") {
            if let Some(id) = classify_isbn(&normalized) {
                meta.identifiers.push(id);
                return;
            }
        }
        if s_upper == "ASIN" {
            meta.identifiers.push(ExtractedIdentifier {
                identifier_type: IdentifierType::Asin,
                value: normalized,
            });
            return;
        }
    }

    // Heuristic: try pattern-matching the value itself.
    if let Some(id) = classify_isbn(&normalized) {
        meta.identifiers.push(id);
        return;
    }

    // Check for ASIN pattern (10 alphanumeric chars starting with 'B').
    if normalized.len() == 10
        && normalized.starts_with('B')
        && normalized.chars().all(|c| c.is_ascii_alphanumeric())
    {
        meta.identifiers.push(ExtractedIdentifier {
            identifier_type: IdentifierType::Asin,
            value: normalized,
        });
        return;
    }

    // Unrecognised identifier; skip it.
    debug!(value = %raw, "skipping unrecognised identifier");
}

/// Attempt to classify a normalised string as ISBN-13 or ISBN-10.
pub(crate) fn classify_isbn(normalized: &str) -> Option<ExtractedIdentifier> {
    // ISBN-13: exactly 13 digits, starts with 978 or 979.
    if normalized.len() == 13
        && normalized.chars().all(|c| c.is_ascii_digit())
        && (normalized.starts_with("978") || normalized.starts_with("979"))
    {
        return Some(ExtractedIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: normalized.to_owned(),
        });
    }

    // ISBN-10: exactly 10 characters, first 9 digits, last digit or 'X'.
    if normalized.len() == 10 {
        let (body, check) = normalized.split_at(9);
        if body.chars().all(|c| c.is_ascii_digit())
            && check
                .chars()
                .all(|c| c.is_ascii_digit() || c == 'X' || c == 'x')
        {
            return Some(ExtractedIdentifier {
                identifier_type: IdentifierType::Isbn10,
                value: normalized.to_uppercase(),
            });
        }
    }

    None
}

// ── Cover extraction ─────────────────────────────────────────────────

/// Attempt to extract the cover image from the EPUB archive.
///
/// Performs a single-pass XML parse of the OPF to find both the cover
/// reference (EPUB 2 meta or EPUB 3 property) and the manifest items,
/// then resolves the cover path and reads it from the archive.
fn extract_cover(
    opf_xml: &str,
    opf_dir: &str,
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
) -> Result<Option<CoverData>, FormatError> {
    let cover_info = find_cover_in_opf(opf_xml)?;

    let item = match cover_info {
        CoverInfo::Resolved(item) => item,
        CoverInfo::MetaContentId(id, items) => {
            let Some(item) = items.into_iter().find(|item| item.id == id) else {
                debug!("cover reference found but no matching manifest item");
                return Ok(None);
            };
            item
        }
        CoverInfo::None => return Ok(None),
    };

    let cover_path = if item.href.starts_with('/') {
        item.href[1..].to_owned()
    } else {
        format!("{opf_dir}{}", item.href)
    };

    let bytes = read_zip_entry_bytes(archive, &cover_path)?;

    Ok(Some(CoverData {
        bytes,
        media_type: item.media_type,
    }))
}

/// Result of the single-pass OPF cover search.
enum CoverInfo {
    /// EPUB 3: a manifest `<item>` with `properties="cover-image"` — already resolved.
    Resolved(ManifestItem),
    /// EPUB 2: `<meta name="cover" content="id"/>` — need to look up id in manifest items.
    MetaContentId(String, Vec<ManifestItem>),
    /// No cover reference found.
    None,
}

/// A parsed `<item>` from the OPF `<manifest>`.
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
    properties_has_cover_image: bool,
}

/// Single-pass scan of the OPF that collects both cover references and manifest items.
fn find_cover_in_opf(opf_xml: &str) -> Result<CoverInfo, FormatError> {
    let mut reader = Reader::from_str(opf_xml);
    let mut in_metadata = false;
    let mut in_manifest = false;
    let mut epub2_cover_id: Option<String> = None;
    let mut items = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                match name {
                    b"metadata" => in_metadata = true,
                    b"manifest" => in_manifest = true,
                    b"item" if in_manifest => {
                        if let Some(item) = parse_manifest_item(e) {
                            if item.properties_has_cover_image {
                                return Ok(CoverInfo::Resolved(item));
                            }
                            items.push(item);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());

                // EPUB 2: <meta name="cover" content="id"/>
                if in_metadata && name == b"meta" && epub2_cover_id.is_none() {
                    let attr_name = find_attr(e, b"name");
                    if attr_name.as_deref() == Some("cover") {
                        epub2_cover_id = find_attr(e, b"content");
                    }
                }

                // Manifest item (self-closing)
                if in_manifest && name == b"item" {
                    if let Some(item) = parse_manifest_item(e) {
                        if item.properties_has_cover_image {
                            return Ok(CoverInfo::Resolved(item));
                        }
                        items.push(item);
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                match name {
                    b"metadata" => in_metadata = false,
                    b"manifest" => in_manifest = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(FormatError::Parse {
                    format: "EPUB".into(),
                    message: format!("error scanning OPF for cover: {e}"),
                });
            }
            _ => {}
        }
    }

    Ok(epub2_cover_id.map_or(CoverInfo::None, |id| CoverInfo::MetaContentId(id, items)))
}

/// Extract a `ManifestItem` from a `<item>` element's attributes.
fn parse_manifest_item(e: &quick_xml::events::BytesStart<'_>) -> Option<ManifestItem> {
    let id = find_attr(e, b"id")?;
    let href = find_attr(e, b"href")?;
    let media_type = find_attr(e, b"media-type").unwrap_or_default();
    let properties_has_cover_image = find_attr(e, b"properties")
        .is_some_and(|p| p.split_whitespace().any(|v| v == "cover-image"));

    Some(ManifestItem {
        id,
        href,
        media_type,
        properties_has_cover_image,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Return the local name of an XML element (strip namespace prefix).
pub(crate) fn local_name(full: &[u8]) -> &[u8] {
    full.iter()
        .position(|&b| b == b':')
        .map_or(full, |pos| &full[pos + 1..])
}

/// Find an attribute by local name on an XML element, returning its decoded value.
pub(crate) fn find_attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|attr| {
        if local_name(attr.key.as_ref()) == key {
            Some(String::from_utf8_lossy(&attr.value).into_owned())
        } else {
            None
        }
    })
}

/// Read a file from the ZIP archive as a UTF-8 string.
pub(crate) fn read_zip_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<String, FormatError> {
    let mut entry = archive.by_name(path).map_err(|e| FormatError::Parse {
        format: "EPUB".into(),
        message: format!("missing entry '{path}': {e}"),
    })?;

    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| FormatError::Parse {
            format: "EPUB".into(),
            message: format!("failed to read '{path}': {e}"),
        })?;

    Ok(buf)
}

/// Read a file from the ZIP archive as raw bytes.
fn read_zip_entry_bytes(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, FormatError> {
    let mut entry = archive.by_name(path).map_err(|e| FormatError::Parse {
        format: "EPUB".into(),
        message: format!("missing entry '{path}': {e}"),
    })?;

    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| FormatError::Parse {
            format: "EPUB".into(),
            message: format!("failed to read '{path}': {e}"),
        })?;

    Ok(buf)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Helper: build a minimal EPUB ZIP archive from an OPF string and optional extra entries.
    fn build_epub_with_opf(opf: &str, extras: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = zip::ZipWriter::new(cursor);

        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // mimetype (must be first, stored)
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        // META-INF/container.xml
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )
        .unwrap();

        // OPF
        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(opf.as_bytes()).unwrap();

        // Extra entries (e.g. cover images)
        for (path, data) in extras {
            zip.start_file((*path).to_string(), stored).unwrap();
            zip.write_all(data).unwrap();
        }

        zip.finish().unwrap().into_inner()
    }

    fn basic_opf() -> String {
        r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0"
         unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"
            xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>The Rust Programming Language</dc:title>
    <dc:creator opf:role="aut">Steve Klabnik</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier opf:scheme="ISBN">978-1-7185-0044-0</dc:identifier>
    <dc:publisher>No Starch Press</dc:publisher>
    <dc:date>2019-08-06</dc:date>
    <dc:description>A comprehensive guide to Rust</dc:description>
    <dc:subject>Programming</dc:subject>
    <dc:subject>Systems</dc:subject>
  </metadata>
  <manifest/>
  <spine/>
</package>"#
            .to_owned()
    }

    #[test]
    fn extracts_basic_metadata() {
        let data = build_epub_with_opf(&basic_opf(), &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("The Rust Programming Language"));
        assert_eq!(meta.authors, vec!["Steve Klabnik"]);
        assert_eq!(meta.language.as_deref(), Some("en"));
        assert_eq!(meta.publisher.as_deref(), Some("No Starch Press"));
        assert_eq!(meta.publication_year, Some(2019));
        assert_eq!(
            meta.description.as_deref(),
            Some("A comprehensive guide to Rust")
        );
        assert_eq!(meta.subjects, vec!["Programming", "Systems"]);
        assert_eq!(meta.source, MetadataSource::Embedded);
        assert_eq!(meta.format_version.as_deref(), Some("3.0"));
    }

    #[test]
    fn extracts_isbn13() {
        let data = build_epub_with_opf(&basic_opf(), &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.identifiers.len(), 1);
        assert_eq!(meta.identifiers[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(meta.identifiers[0].value, "9781718500440");
    }

    #[test]
    fn extracts_isbn10() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"
            xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Test Book</dc:title>
    <dc:identifier opf:scheme="ISBN">0-596-51774-X</dc:identifier>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.identifiers.len(), 1);
        assert_eq!(meta.identifiers[0].identifier_type, IdentifierType::Isbn10);
        assert_eq!(meta.identifiers[0].value, "059651774X");
        assert_eq!(meta.format_version.as_deref(), Some("2.0"));
    }

    #[test]
    fn extracts_multiple_authors() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"
            xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Collaborative Work</dc:title>
    <dc:creator opf:role="aut">Alice Smith</dc:creator>
    <dc:creator opf:role="aut">Bob Jones</dc:creator>
    <dc:creator opf:role="edt">Carol Editor</dc:creator>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        // Should include only authors, not the editor.
        assert_eq!(meta.authors, vec!["Alice Smith", "Bob Jones"]);
    }

    #[test]
    fn creators_without_role_are_treated_as_authors() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Simple Book</dc:title>
    <dc:creator>Jane Doe</dc:creator>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();
        assert_eq!(meta.authors, vec!["Jane Doe"]);
    }

    #[test]
    fn extracts_calibre_series() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Foundation</dc:title>
    <dc:creator>Isaac Asimov</dc:creator>
    <meta name="calibre:series" content="Foundation"/>
    <meta name="calibre:series_index" content="1"/>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.series.as_deref(), Some("Foundation"));
        assert_eq!(meta.series_position, Some(1.0));
    }

    #[test]
    fn extracts_epub3_series() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dune Messiah</dc:title>
    <dc:creator>Frank Herbert</dc:creator>
    <meta property="belongs-to-collection">Dune</meta>
    <meta property="group-position">2</meta>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.series.as_deref(), Some("Dune"));
        assert_eq!(meta.series_position, Some(2.0));
    }

    #[test]
    fn calibre_series_takes_precedence_over_epub3() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test</dc:title>
    <meta name="calibre:series" content="Calibre Series"/>
    <meta name="calibre:series_index" content="3"/>
    <meta property="belongs-to-collection">EPUB3 Series</meta>
    <meta property="group-position">5</meta>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        // Calibre metadata is processed first (before EPUB 3), so it wins.
        assert_eq!(meta.series.as_deref(), Some("Calibre Series"));
        assert_eq!(meta.series_position, Some(3.0));
    }

    #[test]
    fn extracts_cover_epub2() {
        let cover_bytes = b"fake-png-image-data";

        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Book With Cover</dc:title>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="cover-img" href="images/cover.png" media-type="image/png"/>
  </manifest>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[("OEBPS/images/cover.png", cover_bytes)]);
        let meta = extract_epub_metadata(&data).unwrap();

        let cover = meta.cover_image.unwrap();
        assert_eq!(cover.bytes, cover_bytes);
        assert_eq!(cover.media_type, "image/png");
    }

    #[test]
    fn extracts_cover_epub3() {
        let cover_bytes = b"fake-jpeg-cover";

        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>EPUB3 Cover</dc:title>
  </metadata>
  <manifest>
    <item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
  </manifest>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[("OEBPS/cover.jpg", cover_bytes)]);
        let meta = extract_epub_metadata(&data).unwrap();

        let cover = meta.cover_image.unwrap();
        assert_eq!(cover.bytes, cover_bytes);
        assert_eq!(cover.media_type, "image/jpeg");
    }

    #[test]
    fn missing_cover_continues_gracefully() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>No Cover</dc:title>
    <meta name="cover" content="missing-id"/>
  </metadata>
  <manifest>
    <item id="missing-id" href="nonexistent.png" media-type="image/png"/>
  </manifest>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        // Cover extraction should fail gracefully.
        assert!(meta.cover_image.is_none());
    }

    #[test]
    fn isbn_heuristic_without_scheme() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Heuristic ISBN</dc:title>
    <dc:identifier>978-3-16-148410-0</dc:identifier>
    <dc:identifier>urn:uuid:12345678-1234-1234-1234-123456789012</dc:identifier>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        // Only the ISBN-13 should be extracted; the UUID is skipped.
        assert_eq!(meta.identifiers.len(), 1);
        assert_eq!(meta.identifiers[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(meta.identifiers[0].value, "9783161484100");
    }

    #[test]
    fn invalid_zip_returns_error() {
        let result = extract_epub_metadata(b"not a zip file at all");
        assert!(result.is_err());
    }

    #[test]
    fn no_container_xml_returns_error() {
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = zip::ZipWriter::new(cursor);

        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let data = zip.finish().unwrap().into_inner();
        let result = extract_epub_metadata(&data);
        assert!(result.is_err());
    }

    #[test]
    fn empty_values_are_filtered() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>  Real Title  </dc:title>
    <dc:creator>  </dc:creator>
    <dc:subject>  </dc:subject>
    <dc:subject>Valid Subject</dc:subject>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("Real Title"));
        assert!(meta.authors.is_empty());
        assert_eq!(meta.subjects, vec!["Valid Subject"]);
    }

    #[test]
    fn opf_in_subdirectory_resolves_cover_path() {
        // The OPF is at OEBPS/content.opf, so cover href "img/cover.jpg"
        // should resolve to "OEBPS/img/cover.jpg" in the ZIP.
        let cover_bytes = b"cover-in-subdir";

        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Subdir Cover</dc:title>
    <meta name="cover" content="cvr"/>
  </metadata>
  <manifest>
    <item id="cvr" href="img/cover.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[("OEBPS/img/cover.jpg", cover_bytes)]);
        let meta = extract_epub_metadata(&data).unwrap();

        let cover = meta.cover_image.unwrap();
        assert_eq!(cover.bytes, cover_bytes);
    }

    #[test]
    fn classify_isbn_unit_tests() {
        // ISBN-13
        let id = classify_isbn("9781718500440").unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn13);
        assert_eq!(id.value, "9781718500440");

        // ISBN-10
        let id = classify_isbn("059651774X").unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn10);
        assert_eq!(id.value, "059651774X");

        // Not an ISBN
        assert!(classify_isbn("12345").is_none());
        assert!(classify_isbn("ABCDEFGHIJ").is_none());

        // 13 digits but wrong prefix
        assert!(classify_isbn("1234567890123").is_none());
    }

    #[test]
    fn asin_identifier() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"
            xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Kindle Book</dc:title>
    <dc:identifier opf:scheme="ASIN">B08N5WRWNW</dc:identifier>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.identifiers.len(), 1);
        assert_eq!(meta.identifiers[0].identifier_type, IdentifierType::Asin);
        assert_eq!(meta.identifiers[0].value, "B08N5WRWNW");
    }

    #[test]
    fn extracts_svg_cover_epub3() {
        let cover_svg = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 150">
  <rect width="100" height="150" fill="#336699"/>
</svg>"##;

        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>SVG Cover Book</dc:title>
  </metadata>
  <manifest>
    <item id="cover" href="images/cover.svg" media-type="image/svg+xml" properties="cover-image"/>
  </manifest>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[("OEBPS/images/cover.svg", cover_svg)]);
        let meta = extract_epub_metadata(&data).unwrap();

        let cover = meta.cover_image.unwrap();
        assert_eq!(cover.media_type, "image/svg+xml");
        // Verify we got the SVG bytes back
        assert!(std::str::from_utf8(&cover.bytes).unwrap().contains("<svg"));
    }

    #[test]
    fn extracts_isbn_from_dc_source() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Original Sin</dc:title>
    <dc:source id="SourceISBN">9798217060672</dc:source>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.identifiers.len(), 1);
        assert_eq!(meta.identifiers[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(meta.identifiers[0].value, "9798217060672");
    }

    #[test]
    fn dc_source_non_isbn_skipped() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Derived Work</dc:title>
    <dc:source>https://example.com/original</dc:source>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert!(meta.identifiers.is_empty());
    }

    #[test]
    fn epub3_title_type_refinements_prefers_main() {
        let opf = r##"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0"
         unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title id="t1">Building a Second Brain</dc:title>
    <dc:title id="t2">A Proven Method to Organize Your Digital Life</dc:title>
    <meta refines="#t1" property="title-type">main</meta>
    <meta refines="#t2" property="title-type">subtitle</meta>
  </metadata>
  <manifest/>
  <spine/>
</package>"##;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("Building a Second Brain"));
        assert_eq!(
            meta.subtitle.as_deref(),
            Some("A Proven Method to Organize Your Digital Life")
        );
    }

    #[test]
    fn epub3_title_type_reversed_order() {
        // Subtitle appears before main in XML — order should not matter.
        let opf = r##"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title id="sub">The Subtitle First</dc:title>
    <dc:title id="main">The Main Title</dc:title>
    <meta refines="#sub" property="title-type">subtitle</meta>
    <meta refines="#main" property="title-type">main</meta>
  </metadata>
  <manifest/>
  <spine/>
</package>"##;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("The Main Title"));
        assert_eq!(meta.subtitle.as_deref(), Some("The Subtitle First"));
    }

    #[test]
    fn epub3_multiple_titles_no_refinements_uses_first() {
        let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>First Title</dc:title>
    <dc:title>Second Title</dc:title>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        // Without refinements, first candidate should be used.
        assert_eq!(meta.title.as_deref(), Some("First Title"));
        assert!(meta.subtitle.is_none());
    }

    #[test]
    fn epub3_subtitle_extracted() {
        let opf = r##"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title id="main-title">Atomic Habits</dc:title>
    <dc:title id="sub-title">An Easy &amp; Proven Way to Build Good Habits</dc:title>
    <meta refines="#main-title" property="title-type">main</meta>
    <meta refines="#sub-title" property="title-type">subtitle</meta>
  </metadata>
  <manifest/>
  <spine/>
</package>"##;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("Atomic Habits"));
        assert_eq!(
            meta.subtitle.as_deref(),
            Some("An Easy & Proven Way to Build Good Habits")
        );
    }

    #[test]
    fn no_namespace_prefixes_still_works() {
        // Some EPUBs omit namespace prefixes entirely.
        let opf = r#"<?xml version="1.0"?>
<package version="2.0">
  <metadata>
    <title>No Prefix</title>
    <creator>Namespace Free</creator>
    <language>fr</language>
  </metadata>
  <manifest/>
  <spine/>
</package>"#;

        let data = build_epub_with_opf(opf, &[]);
        let meta = extract_epub_metadata(&data).unwrap();

        assert_eq!(meta.title.as_deref(), Some("No Prefix"));
        assert_eq!(meta.authors, vec!["Namespace Free"]);
        assert_eq!(meta.language.as_deref(), Some("fr"));
    }
}
