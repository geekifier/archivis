//! Shared metadata quality scoring engine.
//!
//! Pure functions for computing quality scores and metadata status from
//! quality signals. Used by both the ingest scorer (`archivis-formats`)
//! and the live scorer (`archivis-tasks`).

use crate::models::{IdentifierType, MetadataStatus};

// ── Thresholds ───────────────────────────────────────────────────────

/// Minimum confidence to be considered `Identified`.
pub const IDENTIFIED_THRESHOLD: f32 = 0.6;

/// Minimum confidence to be considered `NeedsReview`.
pub const NEEDS_REVIEW_THRESHOLD: f32 = 0.2;

// ── Garbage detection ────────────────────────────────────────────────

/// Author values that are effectively useless.
pub const GARBAGE_AUTHORS: &[&str] = &[
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
pub const GARBAGE_TITLES: &[&str] = &[
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
pub const PLACEHOLDER_ISBNS: &[&str] = &[
    "0000000000",
    "0000000000000",
    "9780000000000",
    "1234567890",
    "1234567890123",
    "9781234567890",
];

// ── Scoring weights ──────────────────────────────────────────────────

/// Weights controlling how different signals contribute to the quality score.
#[derive(Debug, Clone)]
pub struct ScoreWeights {
    /// Bonus for having at least one valid ISBN or strong identifier.
    pub isbn_bonus: f32,
    /// Bonus for having both title and at least one author.
    pub title_author_bonus: f32,
    /// Bonus for having a title but no author.
    pub title_only_bonus: f32,
    /// Maximum bonus from richness fields.
    pub richness_max: f32,
}

/// Fixed balanced weights for the live metadata quality score.
pub const BALANCED_WEIGHTS: ScoreWeights = ScoreWeights {
    isbn_bonus: 0.4,
    title_author_bonus: 0.3,
    title_only_bonus: 0.1,
    richness_max: 0.30,
};

// ── Quality signals ──────────────────────────────────────────────────

/// Intermediate representation of metadata quality signals, decoupled from
/// the concrete metadata types used at ingest vs. live evaluation.
#[derive(Debug, Clone)]
pub struct QualitySignals {
    pub has_title: bool,
    pub has_author: bool,
    pub has_strong_identifier: bool,
    /// Number of richness fields that are populated.
    pub richness_present: u8,
    /// Total number of richness fields checked.
    pub richness_total: u8,
    /// Additional context bonus (e.g. cross-validation at ingest time).
    pub context_bonus: f32,
}

// ── Core engine ──────────────────────────────────────────────────────

/// Compute a quality score from signals and weights.
///
/// Returns a value in 0.0–1.0.
pub fn compute_quality_score(signals: &QualitySignals, weights: &ScoreWeights) -> f32 {
    let mut score = 0.0_f32;

    if signals.has_strong_identifier {
        score += weights.isbn_bonus;
    }

    if signals.has_title && signals.has_author {
        score += weights.title_author_bonus;
    } else if signals.has_title {
        score += weights.title_only_bonus;
    }

    score += signals.context_bonus;

    // Richness bonus
    if signals.richness_total > 0 {
        #[allow(clippy::cast_precision_loss)]
        let ratio = f32::from(signals.richness_present) / f32::from(signals.richness_total);
        score += ratio * weights.richness_max;
    }

    score.min(1.0)
}

/// Map a quality score to a [`MetadataStatus`] using the standard thresholds.
pub fn derive_metadata_status(score: f32) -> MetadataStatus {
    if score >= IDENTIFIED_THRESHOLD {
        MetadataStatus::Identified
    } else if score >= NEEDS_REVIEW_THRESHOLD {
        MetadataStatus::NeedsReview
    } else {
        MetadataStatus::Unidentified
    }
}

// ── Validation helpers ───────────────────────────────────────────────

/// Check whether a title is a known garbage/placeholder value.
pub fn is_garbage_title(title: &str) -> bool {
    let lower = title.trim().to_lowercase();
    GARBAGE_TITLES.contains(&lower.as_str())
}

/// Check whether an author name is a known garbage/placeholder value.
pub fn is_garbage_author(author: &str) -> bool {
    let lower = author.trim().to_lowercase();
    GARBAGE_AUTHORS.contains(&lower.as_str())
}

/// Check whether an ISBN string is a known placeholder.
pub fn is_placeholder_isbn(isbn: &str) -> bool {
    PLACEHOLDER_ISBNS.contains(&isbn)
}

/// Validate an ISBN-13 check digit (modulo 10).
pub fn validate_isbn13_checksum(isbn: &str) -> bool {
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
pub fn validate_isbn10_checksum(isbn: &str) -> bool {
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
        let weight = 10 - u32::try_from(i).expect("index <= 9");
        sum += val * weight;
    }
    sum % 11 == 0
}

/// Check whether an identifier of the given type and value is valid.
///
/// ISBN-10/13 require a non-placeholder value with a correct checksum.
/// All other identifier types (ASIN, Google Books, etc.) are considered
/// valid if present.
pub fn is_valid_identifier_by_type(id_type: IdentifierType, value: &str) -> bool {
    match id_type {
        IdentifierType::Isbn13 => !is_placeholder_isbn(value) && validate_isbn13_checksum(value),
        IdentifierType::Isbn10 => !is_placeholder_isbn(value) && validate_isbn10_checksum(value),
        _ => true,
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn13_checksum_validation() {
        assert!(validate_isbn13_checksum("9783161484100"));
        assert!(!validate_isbn13_checksum("9783161484109"));
        assert!(!validate_isbn13_checksum("978316"));
    }

    #[test]
    fn isbn10_checksum_validation() {
        assert!(validate_isbn10_checksum("0306406152"));
        assert!(validate_isbn10_checksum("080442957X"));
        assert!(!validate_isbn10_checksum("0306406153"));
        assert!(!validate_isbn10_checksum("03064"));
    }

    #[test]
    fn placeholder_isbn_detected() {
        assert!(is_placeholder_isbn("0000000000000"));
        assert!(is_placeholder_isbn("9781234567890"));
        assert!(!is_placeholder_isbn("9783161484100"));
    }

    #[test]
    fn garbage_title_detected() {
        assert!(is_garbage_title("Unknown"));
        assert!(is_garbage_title("  UNTITLED  "));
        assert!(!is_garbage_title("Dune"));
    }

    #[test]
    fn garbage_author_detected() {
        assert!(is_garbage_author("Unknown Author"));
        assert!(is_garbage_author("calibre"));
        assert!(!is_garbage_author("Frank Herbert"));
    }

    #[test]
    fn is_valid_identifier_isbn13() {
        assert!(is_valid_identifier_by_type(
            IdentifierType::Isbn13,
            "9783161484100"
        ));
        assert!(!is_valid_identifier_by_type(
            IdentifierType::Isbn13,
            "9783161484109"
        )); // bad checksum
        assert!(!is_valid_identifier_by_type(
            IdentifierType::Isbn13,
            "0000000000000"
        )); // placeholder
    }

    #[test]
    fn is_valid_identifier_isbn10() {
        assert!(is_valid_identifier_by_type(
            IdentifierType::Isbn10,
            "0306406152"
        ));
        assert!(!is_valid_identifier_by_type(
            IdentifierType::Isbn10,
            "0306406153"
        )); // bad checksum
    }

    #[test]
    fn is_valid_identifier_other_types_always_valid() {
        assert!(is_valid_identifier_by_type(
            IdentifierType::Asin,
            "B000FA64PK"
        ));
        assert!(is_valid_identifier_by_type(
            IdentifierType::GoogleBooks,
            "any_value"
        ));
    }

    #[test]
    fn compute_quality_score_full_book() {
        let signals = QualitySignals {
            has_title: true,
            has_author: true,
            has_strong_identifier: true,
            richness_present: 7,
            richness_total: 7,
            context_bonus: 0.0,
        };
        let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
        // 0.4 + 0.3 + 0.30 = 1.0
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "expected 1.0, got {score}"
        );
    }

    #[test]
    fn compute_quality_score_title_only() {
        let signals = QualitySignals {
            has_title: true,
            has_author: false,
            has_strong_identifier: false,
            richness_present: 0,
            richness_total: 7,
            context_bonus: 0.0,
        };
        let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
        assert!(
            (score - 0.1).abs() < f32::EPSILON,
            "expected 0.1, got {score}"
        );
    }

    #[test]
    fn derive_status_from_score() {
        assert_eq!(derive_metadata_status(0.7), MetadataStatus::Identified);
        assert_eq!(derive_metadata_status(0.6), MetadataStatus::Identified);
        assert_eq!(derive_metadata_status(0.3), MetadataStatus::NeedsReview);
        assert_eq!(derive_metadata_status(0.2), MetadataStatus::NeedsReview);
        assert_eq!(derive_metadata_status(0.1), MetadataStatus::Unidentified);
        assert_eq!(derive_metadata_status(0.0), MetadataStatus::Unidentified);
    }

    #[test]
    fn score_capped_at_one() {
        let signals = QualitySignals {
            has_title: true,
            has_author: true,
            has_strong_identifier: true,
            richness_present: 7,
            richness_total: 7,
            context_bonus: 0.5, // extra bonus that would push past 1.0
        };
        let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "expected 1.0, got {score}"
        );
    }
}
