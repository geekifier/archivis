//! Shared ISBN and LCCN text scanner.
//!
//! Scans arbitrary text for ISBN-13 and ISBN-10 patterns, normalises them,
//! optionally validates checksums, and deduplicates results. Used by both
//! embedded-metadata extractors (`pdf.rs`, `epub.rs`) and content-scan
//! pipelines.
//!
//! Also detects explicitly-labeled LCCNs (Library of Congress Control Numbers)
//! to prevent post-2001 10-digit LCCNs from being misclassified as ISBN-10s.

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

/// A single LCCN occurrence found in text, preserving its byte span for exclusion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LccnOccurrence {
    /// Normalized digits only.
    pub value: String,
    /// Byte span of the digit group in source text.
    pub span: std::ops::Range<usize>,
}

// ---------------------------------------------------------------------------
// Regex patterns
// ---------------------------------------------------------------------------

// ISBN-13: starts with 978 or 979, 13 digits total.
// Hyphens and spaces allowed as group separators.  Trailing \b prevents
// greedy consumption across adjacent ISBNs separated by spaces.
static ISBN13_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(97[89][\d\- ]{9,16}\d)\b").expect("valid regex"));

// ISBN-10: 9 digits + check (digit or X).
// Hyphens and spaces allowed as group separators.  Trailing \b prevents
// greedy consumption across adjacent ISBNs.
static ISBN10_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d[\d\- ]{8,12}[\dXx])\b").expect("valid regex"));

// "LCCN" or "Library of Congress Control Number" + optional colon + digits.
// Uses `\s+` between words to tolerate newlines/multiple spaces from text extraction.
static LCCN_LABEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:LCCN|Library\s+of\s+Congress\s+Control\s+Number)\b\s*:?\s*(?P<value>(?:\d[\s-]?){8,12})\b",
    )
    .expect("valid regex")
});

// lccn.loc.gov/DIGITS
static LCCN_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\blccn\.loc\.gov/(?P<value>\d{8,12})\b").expect("valid regex")
});

// ---------------------------------------------------------------------------
// LCCN scanning
// ---------------------------------------------------------------------------

/// Scan `text` for explicitly-labeled LCCN occurrences.
///
/// Returns all occurrences with their byte spans preserved (not deduplicated)
/// so that each occurrence can be used for ISBN-10 exclusion.
pub fn scan_text_for_lccn_occurrences(text: &str) -> Vec<LccnOccurrence> {
    let mut results = Vec::new();

    for re in [&*LCCN_LABEL_RE, &*LCCN_URL_RE] {
        for caps in re.captures_iter(text) {
            let value_match = caps.name("value").expect("`value` capture group");
            let span = value_match.start()..value_match.end();
            let normalized: String = value_match
                .as_str()
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            if (8..=12).contains(&normalized.len()) {
                results.push(LccnOccurrence {
                    value: normalized,
                    span,
                });
            }
        }
    }

    results
}

/// Deduplicate [`LccnOccurrence`] list by value, preserving first-seen order.
pub fn dedup_lccn_values(occurrences: &[LccnOccurrence]) -> Vec<String> {
    let mut seen = Vec::new();
    for occ in occurrences {
        if !seen.contains(&occ.value) {
            seen.push(occ.value.clone());
        }
    }
    seen
}

fn spans_overlap(a: &std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

// ---------------------------------------------------------------------------
// Core scanning functions
// ---------------------------------------------------------------------------

/// Scan `text` for ISBN-13 and ISBN-10 patterns, excluding ISBN-10 matches
/// whose byte spans overlap with known LCCN occurrences.
///
/// When `require_checksum` is `true`, only ISBNs whose checksum validates are
/// included in the results. When `false`, all pattern-matches with the correct
/// digit count are returned (with `checksum_valid` set accordingly).
///
/// Results are deduplicated by `(identifier_type, value)`.
pub fn scan_text_for_isbns_with_lccn_exclusions(
    text: &str,
    require_checksum: bool,
    lccn_occurrences: &[LccnOccurrence],
) -> Vec<ScannedIsbn> {
    let mut results: Vec<ScannedIsbn> = Vec::new();

    // --- ISBN-13 ---
    // ISBN-13 starts with 978/979, no collision with LCCNs.
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
        let m = cap.get(1).expect("capture group 1");
        let match_span = m.start()..m.end();
        let raw = m.as_str();

        // Skip if this match overlaps any LCCN occurrence
        if lccn_occurrences
            .iter()
            .any(|lccn| spans_overlap(&match_span, &lccn.span))
        {
            continue;
        }

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

/// Scan `text` for ISBN-13 and ISBN-10 patterns.
///
/// When `require_checksum` is `true`, only ISBNs whose checksum validates are
/// included in the results. When `false`, all pattern-matches with the correct
/// digit count are returned (with `checksum_valid` set accordingly).
///
/// Results are deduplicated by `(identifier_type, value)`.
pub fn scan_text_for_isbns(text: &str, require_checksum: bool) -> Vec<ScannedIsbn> {
    scan_text_for_isbns_with_lccn_exclusions(text, require_checksum, &[])
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

    #[test]
    fn scan_isbn13_compact() {
        let results = scan_text_for_isbns("ISBN 9783161484100", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(results[0].value, "9783161484100");
    }

    #[test]
    fn scan_isbn13_with_spaces() {
        let results = scan_text_for_isbns("ISBN 978 3 16 148410 0", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(results[0].value, "9783161484100");
    }

    #[test]
    fn scan_isbn10_with_spaces() {
        let results = scan_text_for_isbns("ISBN 0 306 40615 2", false);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].identifier_type, IdentifierType::Isbn10);
        assert_eq!(results[0].value, "0306406152");
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

    // -----------------------------------------------------------------------
    // LCCN scanning tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_lccn_labeled() {
        let results = scan_text_for_lccn_occurrences("LCCN 2023013542");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2023013542");
    }

    #[test]
    fn scan_lccn_labeled_colon() {
        let results = scan_text_for_lccn_occurrences("LCCN: 2023013542");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2023013542");
    }

    #[test]
    fn scan_lccn_full_label() {
        let results =
            scan_text_for_lccn_occurrences("Library of Congress Control Number 2023013542");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2023013542");
    }

    #[test]
    fn scan_lccn_full_label_multiline() {
        let results =
            scan_text_for_lccn_occurrences("Library of Congress\n  Control Number 2023013542");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2023013542");
    }

    #[test]
    fn scan_lccn_url() {
        let results = scan_text_for_lccn_occurrences("https://lccn.loc.gov/2023013542");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2023013542");
    }

    #[test]
    fn scan_lccn_with_hyphens() {
        let results = scan_text_for_lccn_occurrences("LCCN 2023-013542");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "2023013542");
    }

    #[test]
    fn scan_lccn_all_spans_preserved() {
        let text = "LCCN 2023013542 some text lccn.loc.gov/2023013542";
        let results = scan_text_for_lccn_occurrences(text);
        assert_eq!(results.len(), 2, "both occurrences should be preserved");
        assert_eq!(results[0].value, "2023013542");
        assert_eq!(results[1].value, "2023013542");
        assert_ne!(results[0].span, results[1].span);
    }

    #[test]
    fn lccn_excluded_from_isbn10() {
        // 2023013542 passes ISBN-10 checksum
        let text = "LCCN 2023013542";
        let lccns = scan_text_for_lccn_occurrences(text);
        assert_eq!(lccns.len(), 1);
        let isbns = scan_text_for_isbns_with_lccn_exclusions(text, false, &lccns);
        assert!(
            isbns.is_empty(),
            "LCCN should not appear as ISBN-10: {isbns:?}"
        );
    }

    #[test]
    fn lccn_both_occurrences_excluded() {
        let text = "LCCN 2023013542 ... https://lccn.loc.gov/2023013542";
        let lccns = scan_text_for_lccn_occurrences(text);
        assert_eq!(lccns.len(), 2);
        let isbns = scan_text_for_isbns_with_lccn_exclusions(text, false, &lccns);
        assert!(
            isbns.is_empty(),
            "neither LCCN occurrence should produce ISBN-10: {isbns:?}"
        );
    }

    #[test]
    fn lccn_does_not_block_unrelated_isbn10() {
        let text = "LCCN 2023013542 ISBN 0-306-40615-2";
        let lccns = scan_text_for_lccn_occurrences(text);
        assert_eq!(lccns.len(), 1);
        let isbns = scan_text_for_isbns_with_lccn_exclusions(text, false, &lccns);
        assert_eq!(isbns.len(), 1);
        assert_eq!(isbns[0].value, "0306406152");
        assert_eq!(isbns[0].identifier_type, IdentifierType::Isbn10);
    }

    #[test]
    fn known_false_positives_excluded() {
        // Realistic CIP text with known false-positive LCCNs
        let text = concat!(
            "Library of Congress Control Number 2023013542\n",
            "LCCN: 2016007702\n",
            "LCCN 2021059049\n",
        );
        let lccns = scan_text_for_lccn_occurrences(text);
        assert_eq!(lccns.len(), 3);
        let isbns = scan_text_for_isbns_with_lccn_exclusions(text, false, &lccns);
        assert!(
            isbns.is_empty(),
            "none of the known false positives should appear as ISBNs: {isbns:?}"
        );
    }

    #[test]
    fn isbn13_unaffected_by_lccn_exclusion() {
        let text = "LCCN 2023013542 ISBN 978-3-16-148410-0";
        let lccns = scan_text_for_lccn_occurrences(text);
        let isbns = scan_text_for_isbns_with_lccn_exclusions(text, false, &lccns);
        assert_eq!(isbns.len(), 1);
        assert_eq!(isbns[0].identifier_type, IdentifierType::Isbn13);
        assert_eq!(isbns[0].value, "9783161484100");
    }

    #[test]
    fn dedup_lccn_values_works() {
        let occurrences = vec![
            LccnOccurrence {
                value: "2023013542".into(),
                span: 5..15,
            },
            LccnOccurrence {
                value: "2023013542".into(),
                span: 40..50,
            },
        ];
        let deduped = dedup_lccn_values(&occurrences);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0], "2023013542");
    }
}
