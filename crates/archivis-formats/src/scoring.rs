//! Metadata quality scoring.
//!
//! Evaluates the completeness of extracted metadata and assigns a confidence
//! score (0.0–1.0) that determines the resulting [`MetadataStatus`].

use archivis_core::models::{MetadataStatus, ScoringProfile};
use archivis_core::scoring::{
    compute_quality_score, derive_metadata_status, is_garbage_author, is_garbage_title,
    is_valid_identifier_by_type, QualitySignals, ScoreWeights,
};

use crate::{ExtractedMetadata, ParsedFilename};

/// Result of scoring extracted metadata.
#[derive(Debug, Clone)]
pub struct MetadataScore {
    /// Overall confidence score in the range 0.0–1.0.
    pub confidence: f32,
    /// Status derived from the confidence score.
    pub status: MetadataStatus,
}

// ── Scoring weights (profile-dependent) ──────────────────────────────

/// Bonus when embedded and filename metadata agree on title.
const CROSS_VALIDATION_BONUS: f32 = 0.2;

/// Number of richness fields checked at ingest time.
const RICHNESS_FIELDS: u8 = 5;

/// Maximum richness bonus for `Balanced` profile.
const BALANCED_RICHNESS_MAX: f32 = 0.30;

/// Maximum richness bonus for `Permissive` profile.
const PERMISSIVE_RICHNESS_MAX: f32 = 0.50;

/// Build [`ScoreWeights`] from a [`ScoringProfile`].
fn weights_for_profile(profile: ScoringProfile) -> ScoreWeights {
    let richness_max = match profile {
        ScoringProfile::Strict => 0.0,
        ScoringProfile::Balanced => BALANCED_RICHNESS_MAX,
        ScoringProfile::Permissive => PERMISSIVE_RICHNESS_MAX,
    };
    ScoreWeights {
        isbn_bonus: 0.4,
        title_author_bonus: 0.3,
        title_only_bonus: 0.1,
        richness_max,
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Score extracted metadata from embedded sources, optionally cross-validated
/// against filename-derived metadata.
///
/// The `profile` controls how much weight is given to metadata richness
/// (description, language, publisher, subjects, publication year) beyond the
/// core signals (ISBN, title, author, cross-validation).
pub fn score_metadata(
    embedded: &ExtractedMetadata,
    filename: Option<&ParsedFilename>,
    profile: &ScoringProfile,
) -> MetadataScore {
    let signals = extract_ingest_signals(embedded, filename);
    let weights = weights_for_profile(*profile);
    let confidence = compute_quality_score(&signals, &weights);
    let status = derive_metadata_status(confidence);

    MetadataScore { confidence, status }
}

/// Extract quality signals from ingest-time metadata.
fn extract_ingest_signals(
    embedded: &ExtractedMetadata,
    filename: Option<&ParsedFilename>,
) -> QualitySignals {
    let has_valid_isbn = embedded.identifiers.iter().any(is_valid_identifier);

    let has_title = embedded
        .title
        .as_deref()
        .is_some_and(|t| !is_garbage_title(t));
    let has_author = embedded.authors.iter().any(|a| !is_garbage_author(a));

    // Cross-validation bonus
    let context_bonus = filename.map_or(0.0, |parsed| {
        if !has_title {
            return 0.0;
        }
        parsed.title.as_ref().map_or(0.0, |file_title| {
            let emb_title = embedded.title.as_deref().unwrap_or_default();
            if titles_match(emb_title, file_title) {
                CROSS_VALIDATION_BONUS
            } else {
                0.0
            }
        })
    });

    // Richness: 5 ingest-time fields
    let richness_present = richness_count(embedded);

    QualitySignals {
        has_title,
        has_author,
        has_strong_identifier: has_valid_isbn,
        richness_present,
        richness_total: RICHNESS_FIELDS,
        context_bonus,
    }
}

// ── Validation helpers ───────────────────────────────────────────────

/// Check whether an extracted identifier is valid (delegates to shared engine).
fn is_valid_identifier(id: &crate::ExtractedIdentifier) -> bool {
    is_valid_identifier_by_type(id.identifier_type, &id.value)
}

/// Count how many ingest-time richness fields are populated.
fn richness_count(embedded: &ExtractedMetadata) -> u8 {
    let mut count: u8 = 0;
    if embedded
        .description
        .as_deref()
        .is_some_and(|d| !d.is_empty())
    {
        count += 1;
    }
    if embedded.language.as_deref().is_some_and(|l| !l.is_empty()) {
        count += 1;
    }
    if embedded.publisher.as_deref().is_some_and(|p| !p.is_empty()) {
        count += 1;
    }
    if !embedded.subjects.is_empty() {
        count += 1;
    }
    if embedded.publication_year.is_some() {
        count += 1;
    }
    count
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
    use archivis_core::models::{IdentifierType, MetadataSource};

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

        // ISBN is placeholder, so no ISBN bonus. Title+author still count.
        assert!(
            (score.confidence - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {}",
            score.confidence
        );
    }

    #[test]
    fn invalid_isbn_checksum_not_counted() {
        // ISBN-13 with bad checksum
        let meta = meta_with_isbn13("9783161484109");
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, Some(&filename), &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, Some(&filename), &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, Some(&filename), &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, Some(&filename), &ScoringProfile::Strict);

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
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

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

    // ── Richness / profile tests ────────────────────────────────────

    /// Helper: metadata with title+author and all five richness fields populated.
    fn rich_meta_no_isbn() -> ExtractedMetadata {
        ExtractedMetadata {
            title: Some("Pride and Prejudice".into()),
            authors: vec!["Jane Austen".into()],
            description: Some("A classic novel of manners.".into()),
            language: Some("en".into()),
            publisher: Some("Global Grey".into()),
            subjects: vec!["Fiction".into(), "Romance".into()],
            publication_year: Some(1813),
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        }
    }

    #[test]
    fn strict_profile_no_richness_bonus() {
        let meta = rich_meta_no_isbn();
        let score = score_metadata(&meta, None, &ScoringProfile::Strict);

        // 0.3 (title+author), no richness
        assert!(
            (score.confidence - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::NeedsReview);
    }

    #[test]
    fn balanced_profile_rich_metadata_reaches_identified() {
        let meta = rich_meta_no_isbn();
        let score = score_metadata(&meta, None, &ScoringProfile::Balanced);

        // 0.3 (title+author) + 0.30 (5/5 richness) = 0.60
        let expected = 0.3 + BALANCED_RICHNESS_MAX;
        assert!(
            (score.confidence - expected).abs() < f32::EPSILON,
            "expected {expected}, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Identified);
    }

    #[test]
    fn balanced_with_cross_validation_reaches_identified() {
        let meta = rich_meta_no_isbn();
        let filename = ParsedFilename {
            title: Some("Pride and Prejudice".into()),
            ..Default::default()
        };
        let score = score_metadata(&meta, Some(&filename), &ScoringProfile::Balanced);

        // 0.3 + 0.2 (cross-val) + 0.30 (richness) = 0.80
        let expected = 0.3 + 0.2 + BALANCED_RICHNESS_MAX;
        assert!(
            (score.confidence - expected).abs() < f32::EPSILON,
            "expected {expected}, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Identified);
    }

    #[test]
    fn permissive_profile_rich_metadata_scores_higher() {
        let meta = rich_meta_no_isbn();
        let score = score_metadata(&meta, None, &ScoringProfile::Permissive);

        // 0.3 (title+author) + 0.50 (5/5 richness) = 0.80
        let expected = 0.3 + PERMISSIVE_RICHNESS_MAX;
        assert!(
            (score.confidence - expected).abs() < f32::EPSILON,
            "expected {expected}, got {}",
            score.confidence
        );
        assert_eq!(score.status, MetadataStatus::Identified);
    }

    #[test]
    fn partial_richness_scales_linearly() {
        let meta = ExtractedMetadata {
            title: Some("Test Book".into()),
            authors: vec!["Some Writer".into()],
            description: Some("A description".into()),
            language: Some("en".into()),
            // No publisher, no subjects, no publication_year → 2/5
            source: MetadataSource::Embedded,
            ..ExtractedMetadata::default()
        };
        let score = score_metadata(&meta, None, &ScoringProfile::Balanced);

        // 0.3 (title+author) + (2/5 * 0.30) = 0.3 + 0.12 = 0.42
        let expected = (2.0_f32 / 5.0).mul_add(BALANCED_RICHNESS_MAX, 0.3);
        assert!(
            (score.confidence - expected).abs() < f32::EPSILON,
            "expected {expected}, got {}",
            score.confidence
        );
    }

    #[test]
    fn richness_with_isbn_still_caps_at_one() {
        let mut meta = rich_meta_no_isbn();
        meta.identifiers.push(ExtractedIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9783161484100".into(),
        });
        let filename = ParsedFilename {
            title: Some("Pride and Prejudice".into()),
            ..Default::default()
        };
        let score = score_metadata(&meta, Some(&filename), &ScoringProfile::Permissive);

        // 0.4 + 0.3 + 0.2 + 0.50 = 1.40 → capped at 1.0
        assert!(
            (score.confidence - 1.0).abs() < f32::EPSILON,
            "expected 1.0, got {}",
            score.confidence
        );
    }
}
