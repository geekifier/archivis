use archivis_core::errors::FormatError;
use archivis_core::models::{IdentifierType, MetadataSource};
use tracing::warn;

use crate::epub::classify_isbn;
use crate::{CoverData, ExtractedIdentifier, ExtractedMetadata};

/// Extract metadata from a MOBI/AZW3 file provided as raw bytes.
///
/// Parses the PDB/MOBI headers and EXTH records to extract title, authors,
/// identifiers, cover image, and other metadata fields.
///
/// DRM-encrypted files are handled gracefully: header metadata is still
/// extracted, but cover and text extraction may be unavailable.
///
/// # Errors
///
/// Returns `FormatError::Parse` if the data cannot be parsed as a valid
/// MOBI file.
pub fn extract_mobi_metadata(data: &[u8]) -> Result<ExtractedMetadata, FormatError> {
    let book = mobi_book::MobiBook::new(data).map_err(|e| FormatError::Parse {
        format: "MOBI".into(),
        message: e.to_string(),
    })?;

    if book.has_drm() {
        warn!("MOBI file is DRM-encrypted; extracting header metadata only");
    }

    let meta = book.metadata();

    let mut extracted = ExtractedMetadata {
        title: meta.title.clone(),
        authors: meta.authors.clone(),
        description: meta.description.clone(),
        language: meta.language.clone(),
        publisher: meta.publisher.clone(),
        publication_year: meta
            .publication_date
            .as_deref()
            .and_then(crate::parse_year_from_date_str),
        subjects: meta.subjects.clone(),
        source: MetadataSource::Embedded,
        ..ExtractedMetadata::default()
    };

    // ISBN
    if let Some(ref isbn_raw) = meta.isbn {
        let normalized = isbn_raw.replace(['-', ' '], "");
        if let Some(id) = classify_isbn(&normalized) {
            extracted.identifiers.push(id);
        }
    }

    // ASIN
    if let Some(ref asin) = meta.asin {
        extracted.identifiers.push(ExtractedIdentifier {
            identifier_type: IdentifierType::Asin,
            value: asin.clone(),
        });
    }

    // Cover image (fall back to thumbnail)
    if !book.has_drm() {
        extracted.cover_image = extract_cover(&book);
    }

    Ok(extracted)
}

/// Try to extract a cover image, falling back to thumbnail.
fn extract_cover(book: &mobi_book::MobiBook<'_>) -> Option<CoverData> {
    if let Ok(Some(cover)) = book.cover() {
        return Some(CoverData {
            media_type: cover.format.mime_type().to_owned(),
            bytes: cover.data,
        });
    }

    if let Ok(Some(thumb)) = book.thumbnail() {
        return Some(CoverData {
            media_type: thumb.format.mime_type().to_owned(),
            bytes: thumb.data,
        });
    }

    None
}

/// Extract decompressed text content from a MOBI/AZW3 file.
///
/// Returns the full text content (which may contain HTML markup).
/// DRM-encrypted files return an empty string.
///
/// # Errors
///
/// Returns `FormatError::Parse` if decompression fails on a non-DRM file.
pub fn extract_mobi_text(data: &[u8], max_bytes: usize) -> Result<String, FormatError> {
    let book = mobi_book::MobiBook::new(data).map_err(|e| FormatError::Parse {
        format: "MOBI".into(),
        message: e.to_string(),
    })?;

    if book.has_drm() {
        return Ok(String::new());
    }

    let text = book.text_content().map_err(|e| FormatError::Parse {
        format: "MOBI".into(),
        message: format!("text extraction failed: {e}"),
    })?;

    if text.len() <= max_bytes * 2 {
        return Ok(text);
    }

    // Sample front + back
    let front = &text[..safe_char_boundary(&text, max_bytes)];
    let back_start = safe_char_boundary_rev(&text, text.len() - max_bytes);
    let back = &text[back_start..];

    Ok(format!("{front}\n{back}"))
}

/// Find the nearest char boundary at or before `index`.
fn safe_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Find the nearest char boundary at or after `index`.
fn safe_char_boundary_rev(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_isbn13_from_mobi() {
        let normalized = "9781718500440";
        let id = classify_isbn(normalized).unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn13);
        assert_eq!(id.value, "9781718500440");
    }

    #[test]
    fn classify_isbn10_from_mobi() {
        let normalized = "059651774X";
        let id = classify_isbn(normalized).unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn10);
        assert_eq!(id.value, "059651774X");
    }

    #[test]
    fn classify_isbn_strips_hyphens() {
        let raw = "978-1-7185-0044-0";
        let normalized = raw.replace(['-', ' '], "");
        let id = classify_isbn(&normalized).unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn13);
    }

    #[test]
    fn asin_identifier_created() {
        let asin = "B08N5WRWNW";
        let id = ExtractedIdentifier {
            identifier_type: IdentifierType::Asin,
            value: asin.to_owned(),
        };
        assert_eq!(id.identifier_type, IdentifierType::Asin);
        assert_eq!(id.value, "B08N5WRWNW");
    }

    #[test]
    fn safe_char_boundary_within_ascii() {
        let s = "hello world";
        assert_eq!(safe_char_boundary(s, 5), 5);
    }

    #[test]
    fn safe_char_boundary_beyond_len() {
        let s = "hi";
        assert_eq!(safe_char_boundary(s, 100), 2);
    }

    #[test]
    fn safe_char_boundary_rev_within_ascii() {
        let s = "hello world";
        assert_eq!(safe_char_boundary_rev(s, 5), 5);
    }

    #[test]
    fn safe_char_boundary_rev_beyond_len() {
        let s = "hi";
        assert_eq!(safe_char_boundary_rev(s, 100), 2);
    }

    #[test]
    fn safe_char_boundary_multibyte() {
        let s = "he\u{00e9}llo"; // "héllo" — é is 2 bytes at indices 2,3
                                 // index 3 is in the middle of the é character
        assert_eq!(safe_char_boundary(s, 3), 2);
        assert_eq!(safe_char_boundary_rev(s, 3), 4);
    }
}
