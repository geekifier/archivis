//! Shared author-name normalization used by all format extractors and the
//! import pipeline.

/// Normalize a vec of raw author strings: split multi-author entries on
/// common delimiters, trim whitespace, and filter empties.
pub fn normalize_authors(raw: Vec<String>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|s| split_author_string(&s))
        .collect()
}

/// Split a single raw author string into one or more clean author names.
///
/// Strategy (first match wins):
/// 1. **Semicolons** — strongest delimiter, always split.
/// 2. **" and " / " & "** — split only when both sides look like full names
///    (>= 2 words each).
/// 3. **"Last, First" comma pairs** — if the `, `-separated token count is
///    even and >= 2, attempt to pair them as `"First Last"`.
/// 4. **Fallback** — return trimmed original as a single-element vec.
pub fn split_author_string(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // 1. Semicolon split
    if trimmed.contains(';') {
        let parts: Vec<String> = trimmed
            .split(';')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if !parts.is_empty() {
            return parts;
        }
        return Vec::new();
    }

    // 2. " and " / " & " — only if both sides have >= 2 words
    for sep in [" and ", " & "] {
        if let Some(pos) = trimmed.find(sep) {
            let left = trimmed[..pos].trim();
            let right = trimmed[pos + sep.len()..].trim();
            if word_count(left) >= 2 && word_count(right) >= 2 {
                return vec![left.to_owned(), right.to_owned()];
            }
        }
    }

    // 3. "Last, First" comma-pair detection
    if trimmed.contains(", ") {
        let tokens: Vec<&str> = trimmed.split(", ").collect();
        if tokens.len() >= 2 && tokens.len() % 2 == 0 && is_last_first_pattern(&tokens) {
            return tokens
                .chunks(2)
                .map(|pair| format!("{} {}", pair[1].trim(), pair[0].trim()))
                .collect();
        }
    }

    // 4. Fallback — single author
    vec![trimmed.to_owned()]
}

/// Count whitespace-separated words.
fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Heuristic: even-indexed tokens are last names (no `.`), odd-indexed tokens
/// are first names (may contain `.`, typically shorter or equal length).
fn is_last_first_pattern(tokens: &[&str]) -> bool {
    for (i, tok) in tokens.iter().enumerate() {
        let t = tok.trim();
        if t.is_empty() {
            return false;
        }
        if i % 2 == 0 {
            // Last name — should not contain `.`
            if t.contains('.') {
                return false;
            }
        }
        // Odd-indexed (first name) tokens: no strict constraint needed
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Semicolon splitting ─────────────────────────────────────────

    #[test]
    fn trailing_semicolon() {
        assert_eq!(split_author_string("Andy Weir;"), vec!["Andy Weir"]);
    }

    #[test]
    fn multi_author_semicolons() {
        assert_eq!(
            split_author_string("Helen Pluckrose;James A. Lindsay;"),
            vec!["Helen Pluckrose", "James A. Lindsay"],
        );
    }

    #[test]
    fn semicolons_without_trailing() {
        assert_eq!(split_author_string("A;B"), vec!["A", "B"]);
    }

    #[test]
    fn only_semicolons_empty() {
        assert!(split_author_string(";").is_empty());
        assert!(split_author_string(";;").is_empty());
        assert!(split_author_string("").is_empty());
    }

    // ── " and " / " & " splitting ──────────────────────────────────

    #[test]
    fn and_split_both_multi_word() {
        assert_eq!(
            split_author_string("John Smith and Helen Hoover"),
            vec!["John Smith", "Helen Hoover"],
        );
    }

    #[test]
    fn and_not_split_single_word_side() {
        assert_eq!(split_author_string("Alice and Bob"), vec!["Alice and Bob"],);
    }

    #[test]
    fn ampersand_split_both_multi_word() {
        assert_eq!(
            split_author_string("John Smith & Helen Hoover"),
            vec!["John Smith", "Helen Hoover"],
        );
    }

    #[test]
    fn ampersand_not_split_single_word_side() {
        assert_eq!(split_author_string("Alice & Bob"), vec!["Alice & Bob"]);
    }

    // ── "Last, First" comma-pair detection ─────────────────────────

    #[test]
    fn comma_pairs_three_authors() {
        assert_eq!(
            split_author_string("Bolognia, Jean L., Jorizzo, Joseph L., Schaffer, Julie V."),
            vec!["Jean L. Bolognia", "Joseph L. Jorizzo", "Julie V. Schaffer"],
        );
    }

    #[test]
    fn single_last_first_flipped() {
        assert_eq!(split_author_string("Herbert, Frank"), vec!["Frank Herbert"],);
    }

    #[test]
    fn single_last_first_cline() {
        assert_eq!(split_author_string("Cline, Ernest"), vec!["Ernest Cline"],);
    }

    #[test]
    fn single_last_first_no_space_first_name() {
        assert_eq!(split_author_string("Huber, AnnaLee"), vec!["AnnaLee Huber"],);
    }

    #[test]
    fn single_last_first_multi_word_last_name() {
        assert_eq!(
            split_author_string("Le Guin, Ursula K."),
            vec!["Ursula K. Le Guin"],
        );
    }

    #[test]
    fn odd_comma_tokens_not_pattern() {
        assert_eq!(
            split_author_string("Alice, Bob, Charlie"),
            vec!["Alice, Bob, Charlie"],
        );
    }

    // ── Whitespace / single author ─────────────────────────────────

    #[test]
    fn whitespace_trimming() {
        assert_eq!(split_author_string("  Andy Weir  "), vec!["Andy Weir"]);
    }

    #[test]
    fn single_clean_author() {
        assert_eq!(split_author_string("Frank Herbert"), vec!["Frank Herbert"],);
    }

    // ── normalize_authors (flatten) ────────────────────────────────

    #[test]
    fn normalize_flattens_across_entries() {
        assert_eq!(
            normalize_authors(vec!["Andy Weir".into(), "A;B".into()]),
            vec!["Andy Weir", "A", "B"],
        );
    }
}
