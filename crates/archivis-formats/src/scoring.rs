//! Metadata quality scoring.
//!
//! Evaluates the completeness of extracted metadata and assigns a confidence
//! score (0.0–1.0) that determines the resulting [`MetadataStatus`].

use archivis_core::models::{IdentifierType, MetadataStatus};

use crate::{ExtractedMetadata, ParsedFilename};

/// Result of scoring extracted metadata.
#[derive(Debug, Clone)]
pub struct MetadataScore {
    /// Overall confidence score in the range 0.0–1.0.
    pub confidence: f32,
    /// Status derived from the confidence score.
    pub status: MetadataStatus,
}

// ── Thresholds ───────────────────────────────────────────────────────

/// Minimum confidence to be considered `Identified`.
const IDENTIFIED_THRESHOLD: f32 = 0.6;

/// Minimum confidence to be considered `NeedsReview`.
const NEEDS_REVIEW_THRESHOLD: f32 = 0.2;

// ── Scoring weights ──────────────────────────────────────────────────

/// Bonus for having at least one valid ISBN.
const ISBN_BONUS: f32 = 0.4;

/// Bonus for having both title and at least one author.
const TITLE_AUTHOR_BONUS: f32 = 0.3;

/// Bonus for having a title but no author.
const TITLE_ONLY_BONUS: f32 = 0.1;

/// Bonus when embedded and filename metadata agree on title.
const CROSS_VALIDATION_BONUS: f32 = 0.2;

// ── Known garbage values ─────────────────────────────────────────────

/// Author values that are effectively useless.
const GARBAGE_AUTHORS: &[&str] = &[
    "unknown",
    "unknown author",
    "various",
    "various authors",
    "author",
    "n/a",
    "na",
    "none",
    "calibre",
    "calibre ebook management",
];

/// Title values that indicate no real title was extracted.
const GARBAGE_TITLES: &[&str] = &[
    "unknown",
    "unknown title",
    "untitled",
    "title",
    "n/a",
    "na",
    "none",
    "calibre",
];

/// Placeholder ISBNs to reject.
const PLACEHOLDER_ISBNS: &[&str] = &[
    "0000000000",
    "0000000000000",
    "9780000000000",
    "1234567890",
    "1234567890123",
    "9781234567890",
];

// ── Public API ───────────────────────────────────────────────────────

/// Score extracted metadata from embedded sources, optionally cross-validated
/// against filename-derived metadata.
pub fn score_metadata(
    embedded: &ExtractedMetadata,
    filename: Option<&ParsedFilename>,
) -> MetadataScore {
    let mut confidence = 0.0_f32;

    let has_valid_isbn = embedded.identifiers.iter().any(is_valid_identifier);

    if has_valid_isbn {
        confidence += ISBN_BONUS;
    }

    let has_title = embedded
        .title
        .as_deref()
        .is_some_and(|t| !is_garbage_title(t));
    let has_author = embedded.authors.iter().any(|a| !is_garbage_author(a));

    if has_title && has_author {
        confidence += TITLE_AUTHOR_BONUS;
    } else if has_title {
        confidence += TITLE_ONLY_BONUS;
    }

    // Cross-validation: do embedded and filename metadata agree?
    if let Some(parsed) = filename {
        if has_title {
            if let Some(ref file_title) = parsed.title {
                let emb_title = embedded.title.as_deref().unwrap_or_default();
                if titles_match(emb_title, file_title) {
                    confidence += CROSS_VALIDATION_BONUS;
                }
            }
        }
    }

    confidence = confidence.min(1.0);

    let status = if confidence >= IDENTIFIED_THRESHOLD {
        MetadataStatus::Identified
    } else if confidence >= NEEDS_REVIEW_THRESHOLD {
        MetadataStatus::NeedsReview
    } else {
        MetadataStatus::Unidentified
    };

    MetadataScore { confidence, status }
}

// ── Validation helpers ───────────────────────────────────────────────

/// Check whether an identifier is a valid, non-placeholder ISBN with a
/// correct checksum.
fn is_valid_identifier(id: &crate::ExtractedIdentifier) -> bool {
    match id.identifier_type {
        IdentifierType::Isbn13 => {
            !is_placeholder_isbn(&id.value) && validate_isbn13_checksum(&id.value)
        }
        IdentifierType::Isbn10 => {
            !is_placeholder_isbn(&id.value) && validate_isbn10_checksum(&id.value)
        }
        // ASINs and other identifiers are always considered valid if present.
        _ => true,
    }
}

fn is_placeholder_isbn(isbn: &str) -> bool {
    PLACEHOLDER_ISBNS.contains(&isbn)
}

/// Validate an ISBN-13 check digit (modulo 10).
fn validate_isbn13_checksum(isbn: &str) -> bool {
    let digits: Vec<u32> = isbn.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() != 13 {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .enumerate()
        .map(|(i, &d)| if i % 2 == 0 { d } else { d * 3 })
        .sum();
    sum % 10 == 0
}

/// Validate an ISBN-10 check digit (modulo 11).
fn validate_isbn10_checksum(isbn: &str) -> bool {
    let chars: Vec<char> = isbn.chars().collect();
    if chars.len() != 10 {
        return false;
    }
    let mut sum = 0u32;
    for (i, &ch) in chars.iter().enumerate() {
        let val = if ch == 'X' || ch == 'x' {
            if i != 9 {
                return false;
            }
            10
        } else {
            match ch.to_digit(10) {
                Some(d) => d,
                None => return false,
            }
        };
        // i is at most 9 (ISBN-10 has exactly 10 chars), so the cast is safe.
        let weight = 10 - u32::try_from(i).expect("index <= 9");
        sum += val * weight;
    }
    sum % 11 == 0
}

fn is_garbage_title(title: &str) -> bool {
    let lower = title.trim().to_lowercase();
    GARBAGE_TITLES.contains(&lower.as_str())
}

fn is_garbage_author(author: &str) -> bool {
    let lower = author.trim().to_lowercase();
    GARBAGE_AUTHORS.contains(&lower.as_str())
}

/// Compare two titles for a fuzzy match.
///
/// Both are lowercased and stripped of non-alphanumeric characters before
/// comparison. This handles differences in punctuation, whitespace, and
/// subtitle separators.
fn titles_match(a: &str, b: &str) -> bool {
    let norm_a = normalise_for_comparison(a);
    let norm_b = normalise_for_comparison(b);
    if norm_a.is_empty() || norm_b.is_empty() {
        return false;
    }
    // One must be a prefix of the other (handles subtitle differences).
    norm_a.starts_with(&norm_b) || norm_b.starts_with(&norm_a)
}

fn normalise_for_comparison(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use archivis_core::models::MetadataSource;

    use super::*;
    use crate::ExtractedIdentifier;

    fn meta_with_isbn13(isbn: &str) -> ExtractedMetadata {
        ExtractedMetadata {
            title: Some("A Good Book".into()),
            authors: vec!["Real Author".into()],
            identifiers: vec![ExtractedIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: isbn.into(),
            }],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        }
    }

    #[test]
    fn isbn_plus_title_author_scores_high() {
        // Valid ISBN-13: 978-3-16-148410-0 → checksum OK
        let meta = meta_with_isbn13("9783161484100");
        let score = score_metadata(&meta, None);

        // 0.4 (ISBN) + 0.3 (title+author) = 0.7
        assert!(
            (score.confidence - 0.7).abs() < f32::EPSILON,
            "expected 0.7, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Identified);
    }

    #[test]
    fn title_and_author_without_isbn() {
        let meta = ExtractedMetadata {
            title: Some("Dune".into()),
            authors: vec!["Frank Herbert".into()],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None);

        // 0.3 (title+author)
        assert!(
            (score.confidence - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::NeedsReview);
    }

    #[test]
    fn title_only_scores_low() {
        let meta = ExtractedMetadata {
            title: Some("Dune".into()),
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None);

        // 0.1 (title only)
        assert!(
            (score.confidence - 0.1).abs() < f32::EPSILON,
            "expected 0.1, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Unidentified);
    }

    #[test]
    fn nothing_useful_scores_zero() {
        let meta = ExtractedMetadata {
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None);

        assert!(
            score.confidence.abs() < f32::EPSILON,
            "expected 0.0, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Unidentified);
    }

    #[test]
    fn garbage_author_ignored() {
        let meta = ExtractedMetadata {
            title: Some("Dune".into()),
            authors: vec!["Unknown Author".into()],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None);

        // Only title bonus, garbage author doesn't count.
        assert!(
            (score.confidence - 0.1).abs() < f32::EPSILON,
            "expected 0.1, got {}",
            score.confidence
        );
    }

    #[test]
    fn garbage_title_ignored() {
        let meta = ExtractedMetadata {
            title: Some("Unknown".into()),
            authors: vec!["Frank Herbert".into()],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None);

        // Neither title nor title+author bonus.
        assert!(
            score.confidence.abs() < f32::EPSILON,
            "expected 0.0, got {}",
            score.confidence
        );
    }

    #[test]
    fn placeholder_isbn_rejected() {
        let meta = meta_with_isbn13("0000000000000");
        let score = score_metadata(&meta, None);

        // ISBN is placeholder, so no ISBN bonus. Title+author still count.
        assert!(
            (score.confidence - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {}",
            score.confidence
        );
    }

    #[test]
    fn isbn13_checksum_validation() {
        // Valid: 978-3-16-148410-0
        assert!(validate_isbn13_checksum("9783161484100"));
        // Invalid: last digit wrong
        assert!(!validate_isbn13_checksum("9783161484109"));
        // Too short
        assert!(!validate_isbn13_checksum("978316"));
    }

    #[test]
    fn isbn10_checksum_validation() {
        // Valid: 0-306-40615-2
        assert!(validate_isbn10_checksum("0306406152"));
        // Valid with X check digit: 0-8044-2957-X
        assert!(validate_isbn10_checksum("080442957X"));
        // Invalid: last digit wrong
        assert!(!validate_isbn10_checksum("0306406153"));
        // Too short
        assert!(!validate_isbn10_checksum("03064"));
    }

    #[test]
    fn invalid_isbn_checksum_not_counted() {
        // ISBN-13 with bad checksum
        let meta = meta_with_isbn13("9783161484109");
        let score = score_metadata(&meta, None);

        // No ISBN bonus (bad checksum).
        assert!(
            (score.confidence - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {}",
            score.confidence
        );
    }

    #[test]
    fn cross_validation_bonus() {
        let meta = ExtractedMetadata {
            title: Some("Dune".into()),
            authors: vec!["Frank Herbert".into()],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let filename = ParsedFilename {
            title: Some("Dune".into()),
            author: Some("Frank Herbert".into()),
            ..Default::default()
        };
        let score = score_metadata(&meta, Some(&filename));

        // 0.3 (title+author) + 0.2 (cross-validation) = 0.5
        assert!(
            (score.confidence - 0.5).abs() < f32::EPSILON,
            "expected 0.5, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::NeedsReview);
    }

    #[test]
    fn cross_validation_fuzzy_match() {
        let meta = ExtractedMetadata {
            title: Some("Dune: The Novel".into()),
            authors: vec!["Frank Herbert".into()],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let filename = ParsedFilename {
            title: Some("Dune".into()),
            ..Default::default()
        };
        let score = score_metadata(&meta, Some(&filename));

        // "dunethenovel" starts with "dune" → cross-validation matches.
        // 0.3 (title+author) + 0.2 (cross-validation) = 0.5
        assert!(
            (score.confidence - 0.5).abs() < f32::EPSILON,
            "expected 0.5, got {}",
            score.confidence
        );
    }

    #[test]
    fn cross_validation_no_match() {
        let meta = ExtractedMetadata {
            title: Some("Dune".into()),
            authors: vec!["Frank Herbert".into()],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let filename = ParsedFilename {
            title: Some("Foundation".into()),
            ..Default::default()
        };
        let score = score_metadata(&meta, Some(&filename));

        // Titles don't match → no cross-validation bonus.
        assert!(
            (score.confidence - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {}",
            score.confidence
        );
    }

    #[test]
    fn confidence_capped_at_one() {
        // ISBN + title+author + cross-validation = 0.4 + 0.3 + 0.2 = 0.9
        // Even if we somehow got more, it should cap at 1.0.
        let meta = meta_with_isbn13("9783161484100");
        let filename = ParsedFilename {
            title: Some("A Good Book".into()),
            ..Default::default()
        };
        let score = score_metadata(&meta, Some(&filename));

        // 0.4 + 0.3 + 0.2 = 0.9
        assert!(
            (score.confidence - 0.9).abs() < f32::EPSILON,
            "expected 0.9, got {}",
            score.confidence
        );
        assert!(score.confidence <= 1.0);
        assert_eq!(score.status, MetadataStatus::Identified);
    }

    #[test]
    fn isbn10_valid_gives_bonus() {
        let meta = ExtractedMetadata {
            title: Some("Test Book".into()),
            authors: vec!["Test Author".into()],
            identifiers: vec![ExtractedIdentifier {
                identifier_type: IdentifierType::Isbn10,
                value: "0306406152".into(),
            }],
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None);

        // 0.4 (ISBN) + 0.3 (title+author) = 0.7
        assert!(
            (score.confidence - 0.7).abs() < f32::EPSILON,
            "expected 0.7, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Identified);
    }

    #[test]
    fn title_match_is_case_insensitive() {
        assert!(titles_match("DUNE", "dune"));
        assert!(titles_match(
            "The Rust Programming Language",
            "the rust programming language"
        ));
    }

    #[test]
    fn title_match_ignores_punctuation() {
        assert!(titles_match("Dune: The Novel", "Dune - The Novel"));
        assert!(titles_match("Don't Panic!", "Dont Panic"));
    }

    #[test]
    fn title_match_prefix_handles_subtitles() {
        assert!(titles_match("Dune", "Dune: The Desert Planet"));
        assert!(titles_match("Dune: The Desert Planet", "Dune"));
    }

    #[test]
    fn empty_title_does_not_match() {
        assert!(!titles_match("", "Dune"));
        assert!(!titles_match("Dune", ""));
        assert!(!titles_match("", ""));
    }
}
