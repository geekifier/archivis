//! String similarity utilities for metadata matching.
//!
//! Provides title normalization, Jaro-Winkler similarity scoring, and
//! author name comparison used by the multi-provider resolver.

use std::collections::HashSet;

/// Leading articles to strip during title normalization.
const ARTICLES: &[&str] = &[
    "the", "a", "an", // English
    "der", "die", "das", // German
    "le", "la", "les", // French
];

// ── Public API ──────────────────────────────────────────────────────

/// Normalize a title for comparison.
///
/// Lowercases, strips leading articles (`the`, `a`, `an`, `der`, `die`,
/// `das`, `le`, `la`, `les`), removes punctuation, and collapses
/// whitespace.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();

    // Remove punctuation: apostrophes are dropped (contractions stay
    // joined), all other non-alphanumeric characters become spaces.
    let cleaned: String = lower
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                Some(c)
            } else if c == '\'' || c == '\u{2019}' {
                // Drop apostrophes entirely (don't → dont, hitchhiker's → hitchhikers).
                None
            } else {
                Some(' ')
            }
        })
        .collect();

    // Collapse whitespace and trim.
    let words: Vec<&str> = cleaned.split_whitespace().collect();

    // Strip leading article if present.
    let words = if words.len() > 1 && ARTICLES.contains(&words[0]) {
        &words[1..]
    } else {
        &words
    };

    words.join(" ")
}

/// Jaro-Winkler similarity between two strings (0.0-1.0).
///
/// Good for short strings like book titles and author names. Returns 1.0
/// for identical strings, 0.0 for completely different ones.
pub fn similarity(a: &str, b: &str) -> f32 {
    let a_norm = normalize_title(a);
    let b_norm = normalize_title(b);
    jaro_winkler(&a_norm, &b_norm)
}

/// Compare two author name lists. Returns similarity 0.0-1.0.
///
/// Handles name order differences (e.g., "Herbert, Frank" vs
/// "Frank Herbert") and finds the best matching pairs.
pub fn author_similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_norm: Vec<String> = a.iter().map(|n| normalize_author(n)).collect();
    let b_norm: Vec<String> = b.iter().map(|n| normalize_author(n)).collect();

    // For each author in `a`, find the best match in `b`.
    let mut total_sim = 0.0_f32;
    let max_len = a_norm.len().max(b_norm.len());

    for a_author in &a_norm {
        let best = b_norm
            .iter()
            .map(|b_author| jaro_winkler(a_author, b_author))
            .fold(0.0_f32, f32::max);
        total_sim += best;
    }

    #[allow(clippy::cast_precision_loss)] // Author lists are small (< 100 names).
    {
        total_sim / max_len as f32
    }
}

// ── Internal helpers ────────────────────────────────────────────────

/// Normalize an author name for comparison.
///
/// Converts "Last, First" to "first last" form, lowercases, removes
/// punctuation, and collapses whitespace.
fn normalize_author(name: &str) -> String {
    let lower = name.to_lowercase();

    // Handle "Last, First" → "First Last" ordering.
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

/// Compute the Jaro similarity between two strings.
fn jaro(s1: &str, s2: &str) -> f32 {
    if s1 == s2 {
        return 1.0;
    }
    if s1.is_empty() || s2.is_empty() {
        return 0.0;
    }

    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();
    let s1_len = s1_chars.len();
    let s2_len = s2_chars.len();

    // Maximum matching distance.
    let match_distance = (s1_len.max(s2_len) / 2).saturating_sub(1);

    let mut s1_matched = vec![false; s1_len];
    let mut s2_matched = vec![false; s2_len];

    let mut matches = 0_usize;
    let mut transpositions = 0_usize;

    // Find matching characters.
    for i in 0..s1_len {
        let start = i.saturating_sub(match_distance);
        let end = (i + match_distance + 1).min(s2_len);

        for j in start..end {
            if s2_matched[j] || s1_chars[i] != s2_chars[j] {
                continue;
            }
            s1_matched[i] = true;
            s2_matched[j] = true;
            matches += 1;
            break;
        }
    }

    if matches == 0 {
        return 0.0;
    }

    // Count transpositions.
    let mut k = 0_usize;
    for i in 0..s1_len {
        if !s1_matched[i] {
            continue;
        }
        while !s2_matched[k] {
            k += 1;
        }
        if s1_chars[i] != s2_chars[k] {
            transpositions += 1;
        }
        k += 1;
    }

    // These values are small (string lengths), so precision loss is negligible.
    #[allow(clippy::cast_precision_loss)]
    let m = matches as f32;
    #[allow(clippy::cast_precision_loss)]
    let t = transpositions as f32 / 2.0;
    #[allow(clippy::cast_precision_loss)]
    let s1f = s1_len as f32;
    #[allow(clippy::cast_precision_loss)]
    let s2f = s2_len as f32;
    (m / s1f + m / s2f + (m - t) / m) / 3.0
}

/// Compute the Jaro-Winkler similarity between two strings.
///
/// Adds a prefix bonus for strings that share a common prefix (up to 4
/// characters), which is useful for book titles and author names.
fn jaro_winkler(s1: &str, s2: &str) -> f32 {
    let jaro_sim = jaro(s1, s2);

    // Common prefix length (max 4).
    let prefix_len = s1
        .chars()
        .zip(s2.chars())
        .take(4)
        .take_while(|(a, b)| a == b)
        .count();

    // Winkler scaling factor (standard: 0.1).
    let p = 0.1_f32;
    #[allow(clippy::cast_precision_loss)] // prefix_len <= 4
    let prefix = prefix_len as f32;
    (prefix * p).mul_add(1.0 - jaro_sim, jaro_sim)
}

/// Compute trigram Jaccard similarity between two strings.
///
/// Splits each string into character trigrams and computes the Jaccard
/// coefficient. Useful as an alternative similarity metric.
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

/// Generate character trigrams from a string.
fn trigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        // For very short strings, use the whole string as a single "trigram".
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
    fn normalize_strips_english_articles() {
        assert_eq!(
            normalize_title("The Hitchhiker's Guide"),
            "hitchhikers guide" // apostrophe dropped, "The" stripped
        );
        assert_eq!(normalize_title("A Game of Thrones"), "game of thrones");
        assert_eq!(normalize_title("An Introduction"), "introduction");
    }

    #[test]
    fn normalize_strips_non_english_articles() {
        assert_eq!(normalize_title("Der Steppenwolf"), "steppenwolf");
        assert_eq!(normalize_title("Die Verwandlung"), "verwandlung");
        assert_eq!(normalize_title("Das Kapital"), "kapital");
        assert_eq!(normalize_title("Le Petit Prince"), "petit prince");
        assert_eq!(normalize_title("La Peste"), "peste");
        assert_eq!(normalize_title("Les Miserables"), "miserables");
    }

    #[test]
    fn normalize_removes_punctuation_and_collapses_whitespace() {
        // "The" is NOT a leading article here, so it stays.
        assert_eq!(
            normalize_title("Dune: The Desert Planet"),
            "dune the desert planet"
        );
        // Apostrophes are dropped (contractions stay joined).
        assert_eq!(normalize_title("Don't Panic!"), "dont panic");
        assert_eq!(
            normalize_title("  spaces   everywhere  "),
            "spaces everywhere"
        );
    }

    #[test]
    fn normalize_does_not_strip_article_from_single_word() {
        // "The" alone should remain (it's the entire title).
        assert_eq!(normalize_title("The"), "the");
        assert_eq!(normalize_title("A"), "a");
    }

    // ── similarity (Jaro-Winkler) ──

    #[test]
    fn identical_strings_score_one() {
        assert!((similarity("Dune", "Dune") - 1.0).abs() < f32::EPSILON);
        assert!((similarity("dune", "DUNE") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn completely_different_strings_score_low() {
        let s = similarity("Dune", "Foundation");
        assert!(s < 0.6, "expected low similarity, got {s}");
    }

    #[test]
    fn similar_strings_score_high() {
        let s = similarity("Dune", "Dune Messiah");
        assert!(s > 0.6, "expected moderate-high similarity, got {s}");
    }

    #[test]
    fn empty_strings() {
        assert!((similarity("", "") - 1.0).abs() < f32::EPSILON);
        assert!(similarity("Dune", "").abs() < f32::EPSILON);
        assert!(similarity("", "Dune").abs() < f32::EPSILON);
    }

    // ── author_similarity ──

    #[test]
    fn author_last_first_reorder() {
        let sim = author_similarity(
            &["Frank Herbert".to_string()],
            &["Herbert, Frank".to_string()],
        );
        assert!(
            sim > 0.9,
            "expected high similarity for reordered name, got {sim}"
        );
    }

    #[test]
    fn author_identical() {
        let sim = author_similarity(
            &["Frank Herbert".to_string()],
            &["Frank Herbert".to_string()],
        );
        assert!((sim - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn author_empty_lists_match() {
        let sim = author_similarity(&[], &[]);
        assert!((sim - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn author_one_empty_scores_zero() {
        let sim = author_similarity(&["Frank Herbert".to_string()], &[]);
        assert!(sim.abs() < f32::EPSILON);
    }

    #[test]
    fn author_different_names_score_low() {
        let sim = author_similarity(
            &["Frank Herbert".to_string()],
            &["Isaac Asimov".to_string()],
        );
        assert!(sim < 0.5, "expected low similarity, got {sim}");
    }

    #[test]
    fn author_multiple_authors() {
        let sim = author_similarity(
            &["Frank Herbert".to_string(), "Brian Herbert".to_string()],
            &["Herbert, Frank".to_string(), "Herbert, Brian".to_string()],
        );
        assert!(sim > 0.85, "expected high similarity, got {sim}");
    }

    // ── trigram_similarity ──

    #[test]
    fn trigram_identical() {
        assert!((trigram_similarity("dune", "dune") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn trigram_completely_different() {
        let s = trigram_similarity("dune", "foundation");
        assert!(s < 0.2, "expected low trigram similarity, got {s}");
    }

    #[test]
    fn trigram_moderate_overlap() {
        let s = trigram_similarity("dune", "dune messiah");
        assert!(
            s > 0.1 && s < 0.8,
            "expected moderate trigram similarity, got {s}"
        );
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
}
