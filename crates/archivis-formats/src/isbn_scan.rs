//! Shared ISBN text scanner.
//!
//! Scans arbitrary text for ISBN-13 and ISBN-10 patterns, normalises them,
//! optionally validates checksums, and deduplicates results. Used by both
//! embedded-metadata extractors (`pdf.rs`, `epub.rs`) and content-scan
//! pipelines.

use std::sync::LazyLock;

use archivis_core::isbn::validate_isbn;
use archivis_core::models::IdentifierType;
use regex::Regex;

use crate::ExtractedIdentifier;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single ISBN found in a text scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedIsbn {
    /// Whether this is an ISBN-13 or ISBN-10.
    pub identifier_type: IdentifierType,
    /// The normalised ISBN value (digits only, uppercase X for ISBN-10 check).
    pub value: String,
    /// Whether the checksum is valid according to the ISBN algorithm.
    pub checksum_valid: bool,
}

// ---------------------------------------------------------------------------
// Regex patterns
// ---------------------------------------------------------------------------

// ISBN-13: starts with 978 or 979, 13 digits total.
// Hyphens allowed but NOT spaces (to avoid greedy matches across adjacent ISBNs).
static ISBN13_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(97[89][\d\-]{10,16}\d)").expect("valid regex"));

// ISBN-10: 9 digits + check (digit or X).
// Only hyphens as separators (no spaces), to avoid greedy cross-matching.
static ISBN10_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d[\d\-]{8,12}[\dXx])").expect("valid regex"));

// ---------------------------------------------------------------------------
// Core scanning function
// ---------------------------------------------------------------------------

/// Scan `text` for ISBN-13 and ISBN-10 patterns.
///
/// When `require_checksum` is `true`, only ISBNs whose checksum validates are
/// included in the results. When `false`, all pattern-matches with the correct
/// digit count are returned (with `checksum_valid` set accordingly).
///
/// Results are deduplicated by `(identifier_type, value)`.
pub fn scan_text_for_isbns(text: &str, require_checksum: bool) -> Vec<ScannedIsbn> {
    let mut results: Vec<ScannedIsbn> = Vec::new();

    // --- ISBN-13 ---
    for cap in ISBN13_RE.captures_iter(text) {
        let raw = &cap[1];
        let normalized: String = raw.chars().filter(char::is_ascii_digit).collect();
        if normalized.len() != 13 {
            continue;
        }

        let validation = validate_isbn(&normalized);
        if require_checksum && !validation.valid {
            continue;
        }

        let already = results
            .iter()
            .any(|s| s.identifier_type == IdentifierType::Isbn13 && s.value == normalized);
        if !already {
            results.push(ScannedIsbn {
                identifier_type: IdentifierType::Isbn13,
                value: normalized,
                checksum_valid: validation.valid,
            });
        }
    }

    // --- ISBN-10 ---
    for cap in ISBN10_RE.captures_iter(text) {
        let raw = &cap[1];
        let normalized: String = raw
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
            .collect::<String>()
            .to_uppercase();
        if normalized.len() != 10
            || !normalized[..9].chars().all(|c| c.is_ascii_digit())
            || normalized.starts_with("978")
            || normalized.starts_with("979")
        {
            continue;
        }

        let validation = validate_isbn(&normalized);
        if require_checksum && !validation.valid {
            continue;
        }

        let already = results
            .iter()
            .any(|s| s.identifier_type == IdentifierType::Isbn10 && s.value == normalized);
        if !already {
            results.push(ScannedIsbn {
                identifier_type: IdentifierType::Isbn10,
                value: normalized,
                checksum_valid: validation.valid,
            });
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Convenience adapter
// ---------------------------------------------------------------------------

/// Convert a slice of [`ScannedIsbn`] results into [`ExtractedIdentifier`]s,
/// matching the format used by the embedded-metadata extractors.
pub fn to_extracted_identifiers(scanned: &[ScannedIsbn]) -> Vec<ExtractedIdentifier> {
    scanned
        .iter()
        .map(|s| ExtractedIdentifier {
            identifier_type: s.identifier_type,
            value: s.value.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_isbn13_with_hyphens() {
        let results = scan_text_for_isbns("ISBN 978-3-16-148410-0", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(results[0].value, "9783161484100");
    }

    #[test]
    fn scan_isbn10() {
        let results = scan_text_for_isbns("ISBN 0-306-40615-2", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].identifier_type, IdentifierType::Isbn10);
        assert_eq!(results[0].value, "0306406152");
    }

    #[test]
    fn deduplication() {
        let results = scan_text_for_isbns("978-3-16-148410-0 and again 978-3-16-148410-0", false);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn checksum_valid_field() {
        // Valid ISBN-13 (hyphenated to match regex pattern).
        let results = scan_text_for_isbns("978-3-16-148410-0", false);
        assert_eq!(results.len(), 1);
        assert!(results[0].checksum_valid);

        // Invalid checksum ISBN-13 (changed last digit).
        let results = scan_text_for_isbns("978-3-16-148410-9", false);
        assert_eq!(results.len(), 1);
        assert!(!results[0].checksum_valid);
    }

    #[test]
    fn require_checksum_filters_invalid() {
        // Invalid checksum — should be excluded when require_checksum is true.
        let results = scan_text_for_isbns("978-3-16-148410-9", true);
        assert!(results.is_empty());

        // Valid checksum — should be included.
        let results = scan_text_for_isbns("978-3-16-148410-0", true);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn to_extracted_identifiers_converts() {
        let scanned = scan_text_for_isbns("ISBN 978-3-16-148410-0", false);
        let extracted = to_extracted_identifiers(&scanned);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(extracted[0].value, "9783161484100");
    }

    #[test]
    fn isbn10_with_x_check_digit() {
        let results = scan_text_for_isbns("080442957X", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].identifier_type, IdentifierType::Isbn10);
        assert_eq!(results[0].value, "080442957X");
        assert!(results[0].checksum_valid);
    }

    #[test]
    fn multiple_isbns_in_text() {
        let text = "First: 978-3-16-148410-0, second: 0-306-40615-2";
        let results = scan_text_for_isbns(text, false);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn no_isbns_in_plain_text() {
        let results = scan_text_for_isbns("This is a normal sentence with no ISBNs.", false);
        assert!(results.is_empty());
    }

    /// MOBI `filepos` byte offsets must not survive HTML tag stripping.
    #[test]
    fn mobi_filepos_not_matched_after_html_strip() {
        use crate::content_text::strip_html_tags;

        let mobi_html = concat!(
            r#"<a filepos=0000116858><u>Chapter Four</u></a>"#,
            r#"<a filepos=0000321046><u>Chapter Five</u></a>"#,
        );
        let mut stripped = String::new();
        strip_html_tags(mobi_html, &mut stripped);

        let results = scan_text_for_isbns(&stripped, true);
        assert!(
            results.is_empty(),
            "filepos values must not appear after stripping: {stripped:?}"
        );
    }
}
