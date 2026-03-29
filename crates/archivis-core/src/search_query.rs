//! Query DSL parser for the Archivis search bar.
//!
//! Transforms raw user input like `author:"Frank Herbert" series:dune -tag:fanfic`
//! into a structured [`SearchQuery`] AST. The AST is consumed downstream to
//! build FTS5 MATCH expressions, resolve relation names to IDs, and populate
//! [`LibraryFilterState`](crate::models::filter::LibraryFilterState) fields.
//!
//! Design principles:
//! - **Hand-rolled, single-pass** tokenizer — no regex, no parser combinator crate.
//! - **Graceful degradation** — never panics, always returns a valid AST.
//! - **Pure** — no async, no DB, no I/O.

use serde::{Deserialize, Serialize};

// ── Types ───────────────────────────────────────────────────────────

/// A recognized field prefix in the query DSL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryField {
    // Relations (need name→ID resolution downstream)
    Author,
    Series,
    Publisher,
    Tag,
    // FTS column filters (pass through to FTS5 MATCH)
    Title,
    Description,
    // Scalars (map directly to `LibraryFilterState` fields)
    Format,
    /// `metadata_status`
    Status,
    /// `resolution_state`
    Resolution,
    /// `resolution_outcome`
    Outcome,
    Trusted,
    Locked,
    Language,
    /// Supports range syntax: `year:1965..1970`
    Year,
    // Presence
    /// `has:cover`, `has:description`, `has:identifiers`
    Has,
    /// `missing:cover` (sugar for `has:X = false`)
    Missing,
    // Identifiers
    Identifier,
}

impl QueryField {
    /// The canonical DSL prefix name for this field (reverse of `recognize_field`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Series => "series",
            Self::Publisher => "publisher",
            Self::Tag => "tag",
            Self::Title => "title",
            Self::Description => "description",
            Self::Format => "format",
            Self::Status => "status",
            Self::Resolution => "resolution",
            Self::Outcome => "outcome",
            Self::Trusted => "trusted",
            Self::Locked => "locked",
            Self::Language => "language",
            Self::Year => "year",
            Self::Has => "has",
            Self::Missing => "missing",
            Self::Identifier => "identifier",
        }
    }
}

/// A field operator that was recognized but dropped due to an empty value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedField {
    pub field: QueryField,
    pub negated: bool,
}

/// A single clause in the parsed query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum QueryClause {
    /// Plain text terms for full-text search.
    Text { text: String, negated: bool },
    /// Quoted phrase: `"the final empire"`.
    Phrase { phrase: String, negated: bool },
    /// Field-qualified clause: `author:asimov`, `year:1965..1970`.
    Field {
        field: QueryField,
        value: String,
        negated: bool,
    },
    /// OR group combining adjacent clauses.
    Or(Vec<Self>),
}

/// The parsed query AST.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub clauses: Vec<QueryClause>,
    /// Fields that were recognized but had empty values (e.g. `author:`, `-tag:`).
    /// Tracked so downstream code can generate warnings instead of silently dropping them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_empty_fields: Vec<DroppedField>,
}

// ── Field recognition ───────────────────────────────────────────────

/// Try to map a prefix string to a [`QueryField`].
fn recognize_field(prefix: &str) -> Option<QueryField> {
    match prefix {
        "author" => Some(QueryField::Author),
        "series" => Some(QueryField::Series),
        "publisher" | "pub" => Some(QueryField::Publisher),
        "tag" => Some(QueryField::Tag),
        "title" => Some(QueryField::Title),
        "description" | "desc" => Some(QueryField::Description),
        "format" | "fmt" => Some(QueryField::Format),
        "status" => Some(QueryField::Status),
        "resolution" => Some(QueryField::Resolution),
        "outcome" => Some(QueryField::Outcome),
        "trusted" => Some(QueryField::Trusted),
        "locked" => Some(QueryField::Locked),
        "language" | "lang" => Some(QueryField::Language),
        "year" => Some(QueryField::Year),
        "has" => Some(QueryField::Has),
        "missing" => Some(QueryField::Missing),
        "identifier" | "id" => Some(QueryField::Identifier),
        _ => None,
    }
}

// ── Tokenizer ───────────────────────────────────────────────────────

/// Raw token produced by the character-level tokenizer.
#[derive(Debug, Clone)]
enum RawToken {
    /// A quoted sequence (the quotes are already stripped).
    Quoted(String),
    /// An unquoted word (runs until whitespace or `"`).
    Word(String),
}

/// Walk the input character by character, producing [`RawToken`]s.
fn tokenize(raw: &str) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    let mut chars = raw.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        if ch == '"' {
            // Quoted phrase — consume opening quote.
            chars.next();
            let mut buf = String::new();
            loop {
                match chars.next() {
                    Some('"') | None => break,
                    Some(c) => buf.push(c),
                }
            }
            tokens.push(RawToken::Quoted(buf));
        } else {
            // Unquoted word — runs until whitespace or `"`.
            let mut buf = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                // If we hit a quote that is NOT right after a colon, break so
                // the next iteration picks it up as a Quoted token.
                // But if the last char in buf is ':', the quote belongs to a
                // field value like `author:"Frank Herbert"`.
                if c == '"' {
                    if buf.ends_with(':') {
                        // Consume the quote and grab everything until closing
                        // quote as part of this word.
                        chars.next(); // skip opening "
                        let mut quoted_val = String::new();
                        loop {
                            match chars.next() {
                                Some('"') | None => break,
                                Some(qc) => quoted_val.push(qc),
                            }
                        }
                        buf.push('"');
                        buf.push_str(&quoted_val);
                        buf.push('"');
                        // Continue — there might be more characters stuck to
                        // this word (unlikely but be safe).
                        continue;
                    }
                    break;
                }
                buf.push(c);
                chars.next();
            }
            if !buf.is_empty() {
                tokens.push(RawToken::Word(buf));
            }
        }
    }

    tokens
}

// ── Classifier ──────────────────────────────────────────────────────

/// Intermediate classified token before OR-grouping.
#[derive(Debug, Clone)]
enum Classified {
    Clause(QueryClause),
    OrOperator,
}

/// Classify a single [`RawToken`] into a [`Classified`] item.
fn classify(token: RawToken) -> Option<Classified> {
    match token {
        RawToken::Quoted(phrase) => {
            if phrase.is_empty() {
                return None;
            }
            Some(Classified::Clause(QueryClause::Phrase {
                phrase,
                negated: false,
            }))
        }
        RawToken::Word(word) => classify_word(&word),
    }
}

fn classify_word(word: &str) -> Option<Classified> {
    if word.is_empty() {
        return None;
    }

    // Exact `OR` keyword.
    if word == "OR" {
        return Some(Classified::OrOperator);
    }

    // Negation prefix — must be a single `-` followed by at least one character.
    if let Some(rest) = word.strip_prefix('-') {
        if rest.is_empty() {
            // Bare `-` — ignore.
            return None;
        }
        // Double-negation `--term` — treat as plain text.
        if rest.starts_with('-') {
            return Some(Classified::Clause(QueryClause::Text {
                text: word.to_owned(),
                negated: false,
            }));
        }
        // Negated quoted phrase: token is like `-"bad book"`, but the tokenizer
        // would have split that into Word("-") + Quoted("bad book"), so that
        // path is handled separately in `classify_with_pending_negation` below.
        // Here `rest` is an unquoted tail.
        return classify_inner(rest, true);
    }

    classify_inner(word, false)
}

/// Core classification of a (possibly de-negated) unquoted word.
fn classify_inner(word: &str, negated: bool) -> Option<Classified> {
    // Check for field operator.
    if let Some(colon_pos) = word.find(':') {
        let prefix = &word[..colon_pos];
        let raw_value = &word[colon_pos + 1..];

        if let Some(field) = recognize_field(prefix) {
            // Strip surrounding quotes from the value if present.
            let value = raw_value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(raw_value);

            if value.is_empty() {
                // `field:` with no value → skip.
                return None;
            }

            return Some(Classified::Clause(QueryClause::Field {
                field,
                value: value.to_owned(),
                negated,
            }));
        }
        // Unrecognized prefix — fall through to plain text.
    }

    Some(Classified::Clause(QueryClause::Text {
        text: word.to_owned(),
        negated,
    }))
}

// ── OR-grouping ─────────────────────────────────────────────────────

/// Group OR operators: `a OR b OR c` → `Or([a, b, c])`.
fn group_or(items: Vec<Classified>) -> Vec<QueryClause> {
    let mut clauses: Vec<QueryClause> = Vec::new();

    let mut iter = items.into_iter().peekable();
    while let Some(item) = iter.next() {
        match item {
            Classified::OrOperator => {
                // OR at start, or consecutive ORs, or no preceding clause → ignore.
            }
            Classified::Clause(clause) => {
                // Peek ahead: if the next item is an OR operator, start
                // collecting an OR group.
                if matches!(iter.peek(), Some(Classified::OrOperator)) {
                    let mut group = vec![clause];
                    // Consume consecutive `OR <clause>` pairs.
                    while matches!(iter.peek(), Some(Classified::OrOperator)) {
                        iter.next(); // consume OR
                                     // The next item must be a clause for the OR to be valid.
                        match iter.peek() {
                            Some(Classified::Clause(_)) => {
                                if let Some(Classified::Clause(c)) = iter.next() {
                                    group.push(c);
                                }
                            }
                            _ => {
                                // OR at end or followed by another OR → stop.
                                break;
                            }
                        }
                    }
                    if group.len() == 1 {
                        // Degenerate group — unwrap.
                        clauses.push(group.into_iter().next().unwrap());
                    } else {
                        clauses.push(QueryClause::Or(group));
                    }
                } else {
                    clauses.push(clause);
                }
            }
        }
    }

    clauses
}

// ── Text merging ────────────────────────────────────────────────────

/// Merge adjacent non-negated `Text` clauses into a single clause.
fn merge_adjacent_text(clauses: Vec<QueryClause>) -> Vec<QueryClause> {
    let mut result: Vec<QueryClause> = Vec::new();

    for clause in clauses {
        if let QueryClause::Text {
            ref text,
            negated: false,
        } = clause
        {
            if let Some(QueryClause::Text {
                text: ref mut prev,
                negated: false,
            }) = result.last_mut()
            {
                prev.push(' ');
                prev.push_str(text);
                continue;
            }
        }
        result.push(clause);
    }

    result
}

// ── Handling `-"phrase"` (negation before a quoted token) ───────────

/// The tokenizer splits `-"bad book"` into `Word("-")` + `Quoted("bad book")`.
/// This pass fuses them into a single negated phrase clause.
fn fuse_negation_with_following_quote(tokens: Vec<RawToken>) -> Vec<RawToken> {
    let mut result: Vec<RawToken> = Vec::new();
    let mut iter = tokens.into_iter().peekable();

    while let Some(tok) = iter.next() {
        match &tok {
            RawToken::Word(w) if w == "-" => {
                if let Some(RawToken::Quoted(_)) = iter.peek() {
                    // Fuse: turn the next Quoted into a Word with a `-"` prefix
                    // so classify_word handles it via the negation path… but
                    // actually we can produce a classified result directly by
                    // wrapping the Quoted in a special marker.
                    //
                    // Simplest: produce a Quoted and mark negation later. We
                    // use a sentinel wrapper.
                    if let Some(RawToken::Quoted(phrase)) = iter.next() {
                        // Emit a special word that the classifier will never
                        // confuse with real input: `-"` prefix tells us.
                        // Actually let's just build the classified output in
                        // the main pipeline. For now, push a synthetic token.
                        result.push(RawToken::Word(format!("-\"{phrase}\"")));
                    }
                } else {
                    result.push(tok);
                }
            }
            _ => result.push(tok),
        }
    }

    result
}

/// Classify a word that looks like `-"phrase"` into a negated phrase.
fn try_negated_phrase(word: &str) -> Option<Classified> {
    let rest = word.strip_prefix("-\"")?;
    let phrase = rest.strip_suffix('"').unwrap_or(rest);
    if phrase.is_empty() {
        return None;
    }
    Some(Classified::Clause(QueryClause::Phrase {
        phrase: phrase.to_owned(),
        negated: true,
    }))
}

// ── Empty-field detection ───────────────────────────────────────────

/// Pre-scan tokens for recognized field prefixes with empty values.
///
/// Mirrors the negation-stripping and field-recognition logic of
/// `classify_word` / `classify_inner` but only collects the dropped cases.
fn detect_empty_fields(tokens: &[RawToken]) -> Vec<DroppedField> {
    let mut dropped = Vec::new();

    for token in tokens {
        let word = match token {
            RawToken::Word(w) => w.as_str(),
            RawToken::Quoted(_) => continue,
        };

        // Strip negation prefix (same rules as `classify_word`).
        let (inner, negated) = match word.strip_prefix('-') {
            Some(rest) if !rest.is_empty() && !rest.starts_with('-') => (rest, true),
            _ => (word, false),
        };

        if let Some(colon_pos) = inner.find(':') {
            let prefix = &inner[..colon_pos];
            let raw_value = &inner[colon_pos + 1..];

            if let Some(field) = recognize_field(prefix) {
                let value = raw_value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(raw_value);

                if value.is_empty() {
                    dropped.push(DroppedField { field, negated });
                }
            }
        }
    }

    dropped
}

// ── Public API ──────────────────────────────────────────────────────

/// Parse a raw query string into a [`SearchQuery`] AST.
///
/// Returns a valid AST even on malformed input (graceful degradation).
/// Empty field operators (e.g. `author:`) are dropped from `clauses` but
/// tracked in `dropped_empty_fields` for downstream warning generation.
pub fn parse_search_query(raw: &str) -> SearchQuery {
    let raw = raw.trim();
    if raw.is_empty() {
        return SearchQuery::default();
    }

    // 1. Tokenize.
    let tokens = tokenize(raw);

    // 1b. Fuse `-` + `"phrase"` into `-"phrase"`.
    let tokens = fuse_negation_with_following_quote(tokens);

    // 1c. Detect empty field operators before classification drops them.
    let dropped_empty_fields = detect_empty_fields(&tokens);

    // 2. Classify each token.
    let classified: Vec<Classified> = tokens
        .into_iter()
        .filter_map(|tok| match &tok {
            RawToken::Word(w) if w.starts_with("-\"") => {
                try_negated_phrase(w).or_else(|| classify(tok))
            }
            _ => classify(tok),
        })
        .collect();

    // 3. Group OR operators.
    let clauses = group_or(classified);

    // 4. Merge adjacent text.
    let clauses = merge_adjacent_text(clauses);

    SearchQuery {
        clauses,
        dropped_empty_fields,
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: shorthand for parsing.
    fn parse(input: &str) -> Vec<QueryClause> {
        parse_search_query(input).clauses
    }

    // 1. Plain text
    #[test]
    fn plain_text_merged() {
        let clauses = parse("dune messiah");
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "dune messiah".into(),
                negated: false,
            }]
        );
    }

    // 2. Quoted phrase
    #[test]
    fn quoted_phrase() {
        let clauses = parse("\"the final empire\"");
        assert_eq!(
            clauses,
            vec![QueryClause::Phrase {
                phrase: "the final empire".into(),
                negated: false,
            }]
        );
    }

    // 3. Field operator
    #[test]
    fn field_operator() {
        let clauses = parse("author:asimov");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Author,
                value: "asimov".into(),
                negated: false,
            }]
        );
    }

    // 4. Field with quoted value
    #[test]
    fn field_with_quoted_value() {
        let clauses = parse("author:\"Frank Herbert\"");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Author,
                value: "Frank Herbert".into(),
                negated: false,
            }]
        );
    }

    // 5. Negated text
    #[test]
    fn negated_text() {
        let clauses = parse("-dune");
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "dune".into(),
                negated: true,
            }]
        );
    }

    // 6. Negated field
    #[test]
    fn negated_field() {
        let clauses = parse("-tag:fanfic");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Tag,
                value: "fanfic".into(),
                negated: true,
            }]
        );
    }

    // 7. OR grouping
    #[test]
    fn or_grouping() {
        let clauses = parse("dune OR foundation");
        assert_eq!(
            clauses,
            vec![QueryClause::Or(vec![
                QueryClause::Text {
                    text: "dune".into(),
                    negated: false,
                },
                QueryClause::Text {
                    text: "foundation".into(),
                    negated: false,
                },
            ])]
        );
    }

    // 8. Complex query
    #[test]
    fn complex_query() {
        let clauses = parse("author:\"Frank Herbert\" series:dune -tag:fanfic");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Field {
                    field: QueryField::Author,
                    value: "Frank Herbert".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Series,
                    value: "dune".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Tag,
                    value: "fanfic".into(),
                    negated: true,
                },
            ]
        );
    }

    // 9. Mixed text, field, and field
    #[test]
    fn mixed_text_and_fields() {
        let clauses = parse("dune messiah format:epub has:cover");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Text {
                    text: "dune messiah".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Format,
                    value: "epub".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Has,
                    value: "cover".into(),
                    negated: false,
                },
            ]
        );
    }

    // 10. Unrecognized field → plain text
    #[test]
    fn unrecognized_field() {
        let clauses = parse("foo:bar");
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "foo:bar".into(),
                negated: false,
            }]
        );
    }

    // 11. Empty value → skipped from clauses, tracked in `dropped_empty_fields`
    #[test]
    fn empty_field_value_skipped() {
        let q = parse_search_query("author:");
        assert!(q.clauses.is_empty());
        assert_eq!(
            q.dropped_empty_fields,
            vec![DroppedField {
                field: QueryField::Author,
                negated: false,
            }]
        );
    }

    // 12. Unclosed quote → Phrase closed at end of input
    #[test]
    fn unclosed_quote() {
        let clauses = parse("\"dune messiah");
        assert_eq!(
            clauses,
            vec![QueryClause::Phrase {
                phrase: "dune messiah".into(),
                negated: false,
            }]
        );
    }

    // 13. Empty input
    #[test]
    fn empty_input() {
        let clauses = parse("");
        assert!(clauses.is_empty());
    }

    // 14. OR at start
    #[test]
    fn or_at_start() {
        let clauses = parse("OR dune");
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "dune".into(),
                negated: false,
            }]
        );
    }

    // 15. Year range
    #[test]
    fn year_range() {
        let clauses = parse("year:1965..1970");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Year,
                value: "1965..1970".into(),
                negated: false,
            }]
        );
    }

    // 16. Field aliases
    #[test]
    fn field_aliases() {
        let clauses = parse("pub:ace");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Publisher,
                value: "ace".into(),
                negated: false,
            }]
        );

        let clauses = parse("lang:en");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Language,
                value: "en".into(),
                negated: false,
            }]
        );
    }

    // 17. Adjacent text merge
    #[test]
    fn adjacent_text_merge() {
        let clauses = parse("dune messiah author:asimov");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Text {
                    text: "dune messiah".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Author,
                    value: "asimov".into(),
                    negated: false,
                },
            ]
        );
    }

    // 18. Negated phrase
    #[test]
    fn negated_phrase() {
        let clauses = parse("-\"bad book\"");
        assert_eq!(
            clauses,
            vec![QueryClause::Phrase {
                phrase: "bad book".into(),
                negated: true,
            }]
        );
    }

    // 19. Multiple ORs
    #[test]
    fn multiple_ors() {
        let clauses = parse("a OR b OR c");
        assert_eq!(
            clauses,
            vec![QueryClause::Or(vec![
                QueryClause::Text {
                    text: "a".into(),
                    negated: false,
                },
                QueryClause::Text {
                    text: "b".into(),
                    negated: false,
                },
                QueryClause::Text {
                    text: "c".into(),
                    negated: false,
                },
            ])]
        );
    }

    // ── Extra edge cases ────────────────────────────────────────────

    #[test]
    fn or_at_end() {
        let clauses = parse("dune OR");
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "dune".into(),
                negated: false,
            }]
        );
    }

    #[test]
    fn consecutive_or() {
        let clauses = parse("dune OR OR foundation");
        // First OR starts a group from `dune`, but the second OR means no
        // right-hand operand → group collapses to just `dune`. The second OR
        // is skipped, then `foundation` is a plain text. Adjacent non-negated
        // text clauses are merged, so we get a single merged clause.
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "dune foundation".into(),
                negated: false,
            }]
        );
    }

    #[test]
    fn bare_dash_ignored() {
        let clauses = parse("-");
        assert!(clauses.is_empty());
    }

    #[test]
    fn double_negation_is_text() {
        let clauses = parse("--term");
        assert_eq!(
            clauses,
            vec![QueryClause::Text {
                text: "--term".into(),
                negated: false,
            }]
        );
    }

    #[test]
    fn whitespace_only() {
        let clauses = parse("   ");
        assert!(clauses.is_empty());
    }

    #[test]
    fn negated_text_not_merged() {
        // Negated text should NOT be merged with adjacent non-negated text.
        let clauses = parse("dune -messiah");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Text {
                    text: "dune".into(),
                    negated: false,
                },
                QueryClause::Text {
                    text: "messiah".into(),
                    negated: true,
                },
            ]
        );
    }

    #[test]
    fn desc_and_fmt_aliases() {
        let clauses = parse("desc:fantasy fmt:epub");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Field {
                    field: QueryField::Description,
                    value: "fantasy".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Format,
                    value: "epub".into(),
                    negated: false,
                },
            ]
        );
    }

    #[test]
    fn identifier_and_id_alias() {
        let clauses = parse("identifier:isbn:9780451524935 id:asin:B08N5WRWNW");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Field {
                    field: QueryField::Identifier,
                    value: "isbn:9780451524935".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Identifier,
                    value: "asin:B08N5WRWNW".into(),
                    negated: false,
                },
            ]
        );
    }

    #[test]
    fn or_with_fields() {
        let clauses = parse("author:asimov OR author:clarke");
        assert_eq!(
            clauses,
            vec![QueryClause::Or(vec![
                QueryClause::Field {
                    field: QueryField::Author,
                    value: "asimov".into(),
                    negated: false,
                },
                QueryClause::Field {
                    field: QueryField::Author,
                    value: "clarke".into(),
                    negated: false,
                },
            ])]
        );
    }

    #[test]
    fn text_around_or_group() {
        let clauses = parse("scifi dune OR foundation year:2020");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Text {
                    text: "scifi".into(),
                    negated: false,
                },
                QueryClause::Or(vec![
                    QueryClause::Text {
                        text: "dune".into(),
                        negated: false,
                    },
                    QueryClause::Text {
                        text: "foundation".into(),
                        negated: false,
                    },
                ]),
                QueryClause::Field {
                    field: QueryField::Year,
                    value: "2020".into(),
                    negated: false,
                },
            ]
        );
    }

    #[test]
    fn empty_quoted_phrase_skipped() {
        let clauses = parse("\"\"");
        assert!(clauses.is_empty());
    }

    #[test]
    fn mixed_quotes_and_text() {
        let clauses = parse("dune \"the final empire\" foundation");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Text {
                    text: "dune".into(),
                    negated: false,
                },
                QueryClause::Phrase {
                    phrase: "the final empire".into(),
                    negated: false,
                },
                QueryClause::Text {
                    text: "foundation".into(),
                    negated: false,
                },
            ]
        );
    }

    #[test]
    fn missing_field() {
        let clauses = parse("missing:cover");
        assert_eq!(
            clauses,
            vec![QueryClause::Field {
                field: QueryField::Missing,
                value: "cover".into(),
                negated: false,
            }]
        );
    }

    // ── Dropped empty field tracking ────────────────────────────────

    #[test]
    fn empty_field_negated_tracked() {
        let q = parse_search_query("-tag:");
        assert!(q.clauses.is_empty());
        assert_eq!(
            q.dropped_empty_fields,
            vec![DroppedField {
                field: QueryField::Tag,
                negated: true,
            }]
        );
    }

    #[test]
    fn empty_field_multiple_tracked() {
        let q = parse_search_query("author: series:");
        assert!(q.clauses.is_empty());
        assert_eq!(
            q.dropped_empty_fields,
            vec![
                DroppedField {
                    field: QueryField::Author,
                    negated: false,
                },
                DroppedField {
                    field: QueryField::Series,
                    negated: false,
                },
            ]
        );
    }

    #[test]
    fn empty_field_mixed_with_text() {
        let q = parse_search_query("author: dune");
        assert_eq!(
            q.clauses,
            vec![QueryClause::Text {
                text: "dune".into(),
                negated: false,
            }]
        );
        assert_eq!(
            q.dropped_empty_fields,
            vec![DroppedField {
                field: QueryField::Author,
                negated: false,
            }]
        );
    }

    #[test]
    fn empty_field_quoted_empty_value() {
        let q = parse_search_query("author:\"\"");
        assert!(q.clauses.is_empty());
        assert_eq!(
            q.dropped_empty_fields,
            vec![DroppedField {
                field: QueryField::Author,
                negated: false,
            }]
        );
    }

    #[test]
    fn empty_field_all_types_regression() {
        for (input, expected_field) in [
            ("publisher:", QueryField::Publisher),
            ("tag:", QueryField::Tag),
            ("title:", QueryField::Title),
            ("description:", QueryField::Description),
            ("format:", QueryField::Format),
            ("series:", QueryField::Series),
        ] {
            let q = parse_search_query(input);
            assert!(q.clauses.is_empty(), "clauses not empty for {input}");
            assert_eq!(
                q.dropped_empty_fields,
                vec![DroppedField {
                    field: expected_field,
                    negated: false,
                }],
                "wrong dropped field for {input}"
            );
        }
    }

    #[test]
    fn empty_field_unrecognized_prefix_not_tracked() {
        let q = parse_search_query("foo:");
        // Unrecognized prefix falls through to plain text.
        assert_eq!(
            q.clauses,
            vec![QueryClause::Text {
                text: "foo:".into(),
                negated: false,
            }]
        );
        assert!(q.dropped_empty_fields.is_empty());
    }

    #[test]
    fn valid_field_not_tracked_as_dropped() {
        let q = parse_search_query("author:asimov");
        assert_eq!(
            q.clauses,
            vec![QueryClause::Field {
                field: QueryField::Author,
                value: "asimov".into(),
                negated: false,
            }]
        );
        assert!(q.dropped_empty_fields.is_empty());
    }
}
