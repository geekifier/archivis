//! Fuzzy string similarity for duplicate detection during import.
//!
//! Provides trigram-based Jaccard similarity, title/author normalization,
//! and threshold-based matching used by the import pipeline to detect
//! near-duplicate books.

use std::collections::HashSet;

/// Leading articles stripped during title normalization.
const ARTICLES: &[&str] = &[
    "the", "a", "an", // English
    "der", "die", "das", // German
    "le", "la", "les", // French
];

/// Default threshold for title matching.
pub const TITLE_MATCH_THRESHOLD: f32 = 0.7;

/// Default threshold for author matching.
pub const AUTHOR_MATCH_THRESHOLD: f32 = 0.6;

// ── Public API ──────────────────────────────────────────────────────

/// Normalize a book title for comparison.
///
/// Lowercase, remove articles (`the`/`a`/`an`/`der`/`die`/`das`/`le`/`la`/`les`),
/// remove punctuation, collapse whitespace, trim.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();

    // Replace punctuation: apostrophes are dropped (contractions stay
    // joined), all other non-alphanumeric characters become spaces.
    let cleaned: String = lower
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                Some(c)
            } else if c == '\'' || c == '\u{2019}' {
                None
            } else {
                Some(' ')
            }
        })
        .collect();

    let words: Vec<&str> = cleaned.split_whitespace().collect();

    // Strip leading article if present (only when there are more words).
    let words = if words.len() > 1 && ARTICLES.contains(&words[0]) {
        &words[1..]
    } else {
        &words
    };

    words.join(" ")
}

/// Normalize an author name for comparison.
///
/// Lowercase, handle "Last, First" to "First Last" conversion,
/// remove punctuation, collapse whitespace.
pub fn normalize_author(name: &str) -> String {
    let lower = name.to_lowercase();

    // Handle "Last, First" -> "First Last" ordering.
    let reordered = if let Some((last, first)) = lower.split_once(',') {
        format!("{} {}", first.trim(), last.trim())
    } else {
        lower
    };

    // Remove punctuation, collapse whitespace.
    let cleaned: String = reordered
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect();

    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trigram-based Jaccard similarity between two strings (0.0-1.0).
///
/// Splits each string into character trigrams (3-char sliding window)
/// and computes `|intersection| / |union|`. Returns 1.0 for identical
/// strings, 0.0 for completely different ones.
pub fn trigram_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_trigrams = trigrams(a);
    let b_trigrams = trigrams(b);

    if a_trigrams.is_empty() && b_trigrams.is_empty() {
        return 1.0;
    }

    let intersection = a_trigrams.intersection(&b_trigrams).count();
    let union = a_trigrams.union(&b_trigrams).count();

    if union == 0 {
        return 0.0;
    }

    #[allow(clippy::cast_precision_loss)] // Trigram counts are small.
    {
        intersection as f32 / union as f32
    }
}

/// Check if two titles are likely the same book.
///
/// Normalizes both titles and uses two complementary checks:
/// 1. Trigram Jaccard similarity (good for similar-length strings).
/// 2. Prefix containment (handles subtitles: "Dune" vs "Dune: A Novel").
///
/// Returns `true` when either the trigram score meets the threshold or
/// one normalized title is a prefix of the other.
pub fn titles_match(a: &str, b: &str, threshold: f32) -> bool {
    let na = normalize_title(a);
    let nb = normalize_title(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }

    // Strip all non-alphanumeric for prefix comparison (handles subtitle
    // separators and whitespace differences).
    let compact_a: String = na.chars().filter(|c| c.is_alphanumeric()).collect();
    let compact_b: String = nb.chars().filter(|c| c.is_alphanumeric()).collect();
    if compact_a.starts_with(&compact_b) || compact_b.starts_with(&compact_a) {
        return true;
    }

    trigram_similarity(&na, &nb) >= threshold
}

/// Check if two author names likely refer to the same person.
///
/// Normalizes both names (handling "Last, First" reordering) and
/// computes trigram Jaccard similarity, returning `true` when the
/// score meets or exceeds the threshold.
pub fn authors_match(a: &str, b: &str, threshold: f32) -> bool {
    let na = normalize_author(a);
    let nb = normalize_author(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    trigram_similarity(&na, &nb) >= threshold
}

// ── Internal helpers ────────────────────────────────────────────────

/// Generate character trigrams from a string.
fn trigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        let mut set = HashSet::new();
        if !chars.is_empty() {
            set.insert(s.to_string());
        }
        return set;
    }
    chars.windows(3).map(|w| w.iter().collect()).collect()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_title ──

    #[test]
    fn normalize_title_strips_articles_and_punctuation() {
        assert_eq!(
            normalize_title("The Hitchhiker's Guide to the Galaxy"),
            "hitchhikers guide to the galaxy"
        );
    }

    #[test]
    fn normalize_title_removes_articles_in_multiple_languages() {
        assert_eq!(normalize_title("A Game of Thrones"), "game of thrones");
        assert_eq!(normalize_title("An Introduction"), "introduction");
        assert_eq!(normalize_title("Der Steppenwolf"), "steppenwolf");
        assert_eq!(normalize_title("Die Verwandlung"), "verwandlung");
        assert_eq!(normalize_title("Das Kapital"), "kapital");
        assert_eq!(normalize_title("Le Petit Prince"), "petit prince");
        assert_eq!(normalize_title("La Peste"), "peste");
        assert_eq!(normalize_title("Les Miserables"), "miserables");
    }

    #[test]
    fn normalize_title_collapses_whitespace_and_punctuation() {
        assert_eq!(
            normalize_title("Dune: The Desert Planet"),
            "dune the desert planet"
        );
        assert_eq!(normalize_title("Don't Panic!"), "dont panic");
        assert_eq!(
            normalize_title("  spaces   everywhere  "),
            "spaces everywhere"
        );
    }

    #[test]
    fn normalize_title_single_word_article_kept() {
        assert_eq!(normalize_title("The"), "the");
        assert_eq!(normalize_title("A"), "a");
    }

    // ── normalize_author ──

    #[test]
    fn normalize_author_last_first() {
        assert_eq!(normalize_author("Herbert, Frank"), "frank herbert");
    }

    #[test]
    fn normalize_author_first_last() {
        assert_eq!(normalize_author("Frank Herbert"), "frank herbert");
    }

    #[test]
    fn normalize_author_removes_punctuation() {
        assert_eq!(normalize_author("O'Brien, Patrick"), "patrick o brien");
    }

    // ── trigram_similarity ──

    #[test]
    fn trigram_identical() {
        assert!((trigram_similarity("dune", "dune") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn trigram_dune_and_dune_messiah_moderate() {
        let s = trigram_similarity("dune", "dune messiah");
        assert!(s > 0.1 && s < 0.8, "expected moderate similarity, got {s}");
    }

    #[test]
    fn trigram_completely_different() {
        let s = trigram_similarity("dune", "foundation");
        assert!(s < 0.2, "expected low similarity, got {s}");
    }

    #[test]
    fn trigram_empty_strings() {
        // Two identical empty strings are equal (identity check returns 1.0).
        assert!((trigram_similarity("", "") - 1.0).abs() < f32::EPSILON);
        // One empty, one non-empty => 0.0.
        assert!((trigram_similarity("dune", "") - 0.0).abs() < f32::EPSILON);
        assert!((trigram_similarity("", "dune") - 0.0).abs() < f32::EPSILON);
    }

    // ── titles_match ──

    #[test]
    fn titles_match_same_title_case_insensitive() {
        assert!(titles_match("Dune", "DUNE", TITLE_MATCH_THRESHOLD));
    }

    #[test]
    fn titles_match_with_subtitle() {
        assert!(titles_match("Dune", "DUNE: A Novel", TITLE_MATCH_THRESHOLD));
    }

    #[test]
    fn titles_no_match_different_books() {
        assert!(!titles_match("Dune", "Foundation", TITLE_MATCH_THRESHOLD));
    }

    #[test]
    fn titles_match_empty_returns_false() {
        assert!(!titles_match("", "Dune", TITLE_MATCH_THRESHOLD));
        assert!(!titles_match("Dune", "", TITLE_MATCH_THRESHOLD));
    }

    // ── authors_match ──

    #[test]
    fn authors_match_reordered() {
        assert!(authors_match(
            "Frank Herbert",
            "Herbert, Frank",
            AUTHOR_MATCH_THRESHOLD
        ));
    }

    #[test]
    fn authors_match_identical() {
        assert!(authors_match(
            "Frank Herbert",
            "Frank Herbert",
            AUTHOR_MATCH_THRESHOLD
        ));
    }

    #[test]
    fn authors_no_match_different() {
        assert!(!authors_match(
            "Frank Herbert",
            "Isaac Asimov",
            AUTHOR_MATCH_THRESHOLD
        ));
    }

    #[test]
    fn authors_match_empty_returns_false() {
        assert!(!authors_match("", "Frank Herbert", AUTHOR_MATCH_THRESHOLD));
        assert!(!authors_match("Frank Herbert", "", AUTHOR_MATCH_THRESHOLD));
    }
}
