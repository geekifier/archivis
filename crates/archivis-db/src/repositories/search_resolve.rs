//! Resolves a parsed [`SearchQuery`] AST against the database.
//!
//! Takes the pure AST from [`archivis_core::search_query`] and:
//! 1. Resolves relation field operators (author, series, publisher, tag) to UUIDs.
//! 2. Parses scalar field values (format, status, year ranges, booleans).
//! 3. Maps FTS column filters (title, description) into column-qualified expressions.
//! 4. Assembles remaining text into a clean FTS text query.
//! 5. Produces warnings for ambiguous, unknown, or invalid values.

use std::str::FromStr;

use archivis_core::errors::DbError;
use archivis_core::models::filter::{
    canonicalize_identifier_type, canonicalize_identifier_value, is_supported_identifier_type,
};
use archivis_core::models::{BookFormat, MetadataStatus, ResolutionOutcome, ResolutionState};
use archivis_core::search_query::{QueryClause, QueryField, SearchQuery};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Types ───────────────────────────────────────────────────────────

/// FTS5 column-qualified filter, e.g. `title : "dune"`.
#[derive(Debug, Clone)]
pub struct FtsColumnFilter {
    pub column: String,
    pub term: String,
    pub negated: bool,
}

/// Warning emitted during DSL resolution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryWarning {
    AmbiguousRelation {
        field: String,
        query: String,
        match_count: usize,
        matches: Vec<AmbiguousMatch>,
    },
    UnknownRelation {
        field: String,
        query: String,
    },
    InvalidValue {
        field: String,
        value: String,
        reason: String,
    },
    /// A DSL field operator was used without a value (e.g. `author:`).
    EmptyFieldValue {
        field: String,
    },
    /// A structured field operator appeared inside an OR group, which is not supported.
    UnsupportedOrField {
        field: String,
        value: String,
        negated: bool,
    },
}

/// One candidate in an ambiguous relation resolution.
#[derive(Debug, Clone, Serialize)]
pub struct AmbiguousMatch {
    pub id: String,
    pub name: String,
}

/// Result of resolving a [`SearchQuery`] against the database.
#[derive(Debug, Clone, Default)]
pub struct ResolvedQuery {
    /// Remaining FTS text (plain terms + phrases, after DSL extraction).
    pub text_query: Option<String>,
    /// FTS5 column-qualified expressions.
    pub fts_column_filters: Vec<FtsColumnFilter>,

    // Resolved relation IDs
    pub author_id: Option<Uuid>,
    pub series_id: Option<Uuid>,
    pub publisher_id: Option<Uuid>,
    pub tag_ids: Vec<Uuid>,

    // Negated relation IDs
    pub neg_tag_ids: Vec<Uuid>,

    // Scalar overrides from DSL
    pub format: Option<BookFormat>,
    pub metadata_status: Option<MetadataStatus>,
    pub resolution_state: Option<ResolutionState>,
    pub resolution_outcome: Option<ResolutionOutcome>,
    pub trusted: Option<bool>,
    pub locked: Option<bool>,
    pub language: Option<String>,
    pub year_min: Option<i32>,
    pub year_max: Option<i32>,
    pub has_cover: Option<bool>,
    pub has_description: Option<bool>,
    pub has_identifiers: Option<bool>,
    pub identifier_type: Option<String>,
    pub identifier_value: Option<String>,

    pub warnings: Vec<QueryWarning>,
}

// ── Resolver ────────────────────────────────────────────────────────

pub struct SearchResolver;

impl SearchResolver {
    /// Resolve a parsed [`SearchQuery`] into a [`ResolvedQuery`] by looking up
    /// relation names in the database and parsing scalar values.
    pub async fn resolve(pool: &SqlitePool, query: &SearchQuery) -> Result<ResolvedQuery, DbError> {
        let mut result = ResolvedQuery::default();
        let mut text_parts: Vec<String> = Vec::new();

        for clause in &query.clauses {
            Self::resolve_clause(pool, clause, &mut result, &mut text_parts).await?;
        }

        // Combine text parts into a single FTS query string.
        let combined = text_parts.join(" ");
        if !combined.trim().is_empty() {
            result.text_query = Some(combined);
        }

        Ok(result)
    }

    // ── Clause dispatch ─────────────────────────────────────────

    async fn resolve_clause(
        pool: &SqlitePool,
        clause: &QueryClause,
        result: &mut ResolvedQuery,
        text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        match clause {
            QueryClause::Text { text, negated } => {
                Self::resolve_text(text, *negated, text_parts);
            }
            QueryClause::Phrase { phrase, negated } => {
                Self::resolve_phrase(phrase, *negated, text_parts);
            }
            QueryClause::Or(clauses) => {
                Self::resolve_or(pool, clauses, result, text_parts).await?;
            }
            QueryClause::Field {
                field,
                value,
                negated,
            } => {
                Self::resolve_field(pool, *field, value, *negated, result, text_parts).await?;
            }
        }
        Ok(())
    }

    // ── Text / Phrase ───────────────────────────────────────────

    fn resolve_text(text: &str, negated: bool, text_parts: &mut Vec<String>) {
        if negated {
            // Prepend NOT to each word for FTS5.
            let negated_words: Vec<String> = text
                .split_whitespace()
                .map(|w| format!("NOT {w}"))
                .collect();
            text_parts.push(negated_words.join(" "));
        } else {
            text_parts.push(text.to_owned());
        }
    }

    fn resolve_phrase(phrase: &str, negated: bool, text_parts: &mut Vec<String>) {
        if negated {
            text_parts.push(format!("NOT \"{phrase}\""));
        } else {
            text_parts.push(format!("\"{phrase}\""));
        }
    }

    // ── OR group ────────────────────────────────────────────────

    async fn resolve_or(
        pool: &SqlitePool,
        clauses: &[QueryClause],
        result: &mut ResolvedQuery,
        text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        let mut or_text_parts: Vec<String> = Vec::new();

        for clause in clauses {
            match clause {
                QueryClause::Text { text, negated } => {
                    Self::resolve_text(text, *negated, &mut or_text_parts);
                }
                QueryClause::Phrase { phrase, negated } => {
                    Self::resolve_phrase(phrase, *negated, &mut or_text_parts);
                }
                QueryClause::Field {
                    field,
                    value,
                    negated,
                } => {
                    // Structured field operators cannot be OR'd — the resolver can only
                    // store a single value per field.  Emit a warning and skip.
                    result.warnings.push(QueryWarning::UnsupportedOrField {
                        field: field.as_str().to_owned(),
                        value: value.clone(),
                        negated: *negated,
                    });
                }
                QueryClause::Or(inner) => {
                    // Nested OR — flatten recursively.
                    Box::pin(Self::resolve_or(pool, inner, result, &mut or_text_parts)).await?;
                }
            }
        }

        if !or_text_parts.is_empty() {
            text_parts.push(or_text_parts.join(" OR "));
        }

        Ok(())
    }

    // ── Field dispatch ──────────────────────────────────────────

    async fn resolve_field(
        pool: &SqlitePool,
        field: QueryField,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
        text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        match field {
            // Relation fields — need DB lookup.
            QueryField::Author => {
                Self::resolve_author(pool, value, negated, result, text_parts).await?;
            }
            QueryField::Series => {
                Self::resolve_series(pool, value, negated, result, text_parts).await?;
            }
            QueryField::Publisher => {
                Self::resolve_publisher(pool, value, negated, result, text_parts).await?;
            }
            QueryField::Tag => {
                Self::resolve_tag(pool, value, negated, result, text_parts).await?;
            }

            // FTS column fields — pass through to FTS5.
            QueryField::Title | QueryField::Description => {
                let col = if field == QueryField::Title {
                    "title"
                } else {
                    "description"
                };
                result.fts_column_filters.push(FtsColumnFilter {
                    column: col.to_owned(),
                    term: value.to_owned(),
                    negated,
                });
            }

            // Scalar enum fields.
            QueryField::Format => {
                Self::resolve_scalar_enum::<BookFormat>("format", value, negated, result);
            }
            QueryField::Status => {
                Self::resolve_scalar_enum::<MetadataStatus>("status", value, negated, result);
            }
            QueryField::Resolution => {
                Self::resolve_scalar_enum::<ResolutionState>("resolution", value, negated, result);
            }
            QueryField::Outcome => {
                Self::resolve_scalar_enum::<ResolutionOutcome>("outcome", value, negated, result);
            }

            // Scalar simple fields.
            QueryField::Trusted => Self::resolve_bool_field("trusted", value, negated, result),
            QueryField::Locked => Self::resolve_bool_field("locked", value, negated, result),
            QueryField::Language => {
                Self::resolve_negatable_string("language", value, negated, result);
            }
            QueryField::Year => Self::resolve_year(value, negated, result),

            // Presence fields.
            QueryField::Has => Self::resolve_presence(value, negated, false, result),
            QueryField::Missing => Self::resolve_presence(value, negated, true, result),

            // Identifier fields.
            QueryField::Identifier => {
                Self::resolve_identifier(field, value, negated, result);
            }
        }

        Ok(())
    }

    /// Store a simple string field, rejecting negation.
    fn resolve_negatable_string(
        field_name: &str,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
    ) {
        if negated {
            result.warnings.push(QueryWarning::InvalidValue {
                field: field_name.to_owned(),
                value: value.to_owned(),
                reason: "negation not supported for this field".to_owned(),
            });
        } else {
            // Currently only `language` uses this path.
            result.language = Some(value.to_owned());
        }
    }

    /// Store an identifier field, rejecting negation and honouring first-one-wins.
    fn resolve_identifier(
        field: QueryField,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
    ) {
        if field != QueryField::Identifier {
            return;
        }

        let field_name = field.as_str();

        if negated {
            result.warnings.push(QueryWarning::InvalidValue {
                field: field_name.to_owned(),
                value: value.to_owned(),
                reason: "negation not supported for this field".to_owned(),
            });
            return;
        }

        let (identifier_type, raw_value) = match value.split_once(':') {
            Some((raw_type, raw_value)) if raw_type.trim().is_empty() => {
                result.warnings.push(QueryWarning::InvalidValue {
                    field: field_name.to_owned(),
                    value: value.to_owned(),
                    reason: "identifier type must not be empty".to_owned(),
                });
                return;
            }
            Some((raw_type, raw_value)) => (
                canonicalize_identifier_type(raw_type),
                raw_value,
            ),
            None => (None, value),
        };

        if let Some(ref ty) = identifier_type {
            if !is_supported_identifier_type(ty) {
                result.warnings.push(QueryWarning::InvalidValue {
                    field: field_name.to_owned(),
                    value: value.to_owned(),
                    reason: format!("unknown identifier type: {ty}"),
                });
                return;
            }
        }

        let Some(identifier_value) =
            canonicalize_identifier_value(identifier_type.as_deref(), raw_value)
        else {
            result.warnings.push(QueryWarning::InvalidValue {
                field: field_name.to_owned(),
                value: value.to_owned(),
                reason: "identifier value must not be empty".to_owned(),
            });
            return;
        };

        if result.identifier_value.is_none() {
            result.identifier_type = identifier_type;
            result.identifier_value = Some(identifier_value);
        }
    }

    // ── Relation resolvers ──────────────────────────────────────

    async fn resolve_author(
        pool: &SqlitePool,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
        _text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        // 1. Try exact name match.
        if let Some(row) = find_author_by_name(pool, value).await? {
            let id = Uuid::parse_str(&row.id)
                .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?;
            if negated {
                // Negated author — fall back to negated FTS column filter.
                result.fts_column_filters.push(FtsColumnFilter {
                    column: "author_names".to_owned(),
                    term: value.to_owned(),
                    negated: true,
                });
            } else {
                result.author_id = Some(id);
            }
            return Ok(());
        }

        // 2. Substring search fallback.
        let matches = search_authors_lightweight(pool, value).await?;
        match matches.len() {
            1 => {
                let m = &matches[0];
                let id = Uuid::parse_str(&m.id)
                    .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?;
                if negated {
                    result.fts_column_filters.push(FtsColumnFilter {
                        column: "author_names".to_owned(),
                        term: value.to_owned(),
                        negated: true,
                    });
                } else {
                    result.author_id = Some(id);
                }
            }
            0 => {
                result.warnings.push(QueryWarning::UnknownRelation {
                    field: "author".to_owned(),
                    query: value.to_owned(),
                });
            }
            n => {
                result.warnings.push(QueryWarning::AmbiguousRelation {
                    field: "author".to_owned(),
                    query: value.to_owned(),
                    match_count: n,
                    matches: matches
                        .into_iter()
                        .map(|m| AmbiguousMatch {
                            id: m.id,
                            name: m.name,
                        })
                        .collect(),
                });
            }
        }

        Ok(())
    }

    async fn resolve_series(
        pool: &SqlitePool,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
        _text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        // 1. Try exact name match.
        if let Some(series) = find_series_by_name(pool, value).await? {
            let id = Uuid::parse_str(&series.id)
                .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?;
            if negated {
                result.fts_column_filters.push(FtsColumnFilter {
                    column: "series_names".to_owned(),
                    term: value.to_owned(),
                    negated: true,
                });
            } else {
                result.series_id = Some(id);
            }
            return Ok(());
        }

        // 2. Substring search fallback.
        let matches = search_series_lightweight(pool, value).await?;
        match matches.len() {
            1 => {
                let m = &matches[0];
                let id = Uuid::parse_str(&m.id)
                    .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?;
                if negated {
                    result.fts_column_filters.push(FtsColumnFilter {
                        column: "series_names".to_owned(),
                        term: value.to_owned(),
                        negated: true,
                    });
                } else {
                    result.series_id = Some(id);
                }
            }
            0 => {
                result.warnings.push(QueryWarning::UnknownRelation {
                    field: "series".to_owned(),
                    query: value.to_owned(),
                });
            }
            n => {
                result.warnings.push(QueryWarning::AmbiguousRelation {
                    field: "series".to_owned(),
                    query: value.to_owned(),
                    match_count: n,
                    matches: matches
                        .into_iter()
                        .map(|m| AmbiguousMatch {
                            id: m.id,
                            name: m.name,
                        })
                        .collect(),
                });
            }
        }

        Ok(())
    }

    async fn resolve_publisher(
        pool: &SqlitePool,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
        _text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        // 1. Try exact name match.
        if let Some(row) = find_publisher_by_name(pool, value).await? {
            let id = Uuid::parse_str(&row.id)
                .map_err(|e| DbError::Query(format!("invalid publisher UUID: {e}")))?;
            if negated {
                result.fts_column_filters.push(FtsColumnFilter {
                    column: "publisher_name".to_owned(),
                    term: value.to_owned(),
                    negated: true,
                });
            } else {
                result.publisher_id = Some(id);
            }
            return Ok(());
        }

        // 2. Substring search fallback.
        let matches = search_publishers_lightweight(pool, value).await?;
        match matches.len() {
            1 => {
                let m = &matches[0];
                let id = Uuid::parse_str(&m.id)
                    .map_err(|e| DbError::Query(format!("invalid publisher UUID: {e}")))?;
                if negated {
                    result.fts_column_filters.push(FtsColumnFilter {
                        column: "publisher_name".to_owned(),
                        term: value.to_owned(),
                        negated: true,
                    });
                } else {
                    result.publisher_id = Some(id);
                }
            }
            0 => {
                result.warnings.push(QueryWarning::UnknownRelation {
                    field: "publisher".to_owned(),
                    query: value.to_owned(),
                });
            }
            n => {
                result.warnings.push(QueryWarning::AmbiguousRelation {
                    field: "publisher".to_owned(),
                    query: value.to_owned(),
                    match_count: n,
                    matches: matches
                        .into_iter()
                        .map(|m| AmbiguousMatch {
                            id: m.id,
                            name: m.name,
                        })
                        .collect(),
                });
            }
        }

        Ok(())
    }

    async fn resolve_tag(
        pool: &SqlitePool,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
        _text_parts: &mut Vec<String>,
    ) -> Result<(), DbError> {
        // 1. Try exact name match (may return multiple across categories).
        let exact = find_tags_by_name(pool, value).await?;
        if exact.len() == 1 {
            let row = &exact[0];
            let id = Uuid::parse_str(&row.id)
                .map_err(|e| DbError::Query(format!("invalid tag UUID: {e}")))?;
            if negated {
                result.neg_tag_ids.push(id);
            } else {
                result.tag_ids.push(id);
            }
            return Ok(());
        }
        if exact.len() > 1 {
            // Multiple tags with the same name (different categories) — ambiguous.
            result.warnings.push(QueryWarning::AmbiguousRelation {
                field: "tag".to_owned(),
                query: value.to_owned(),
                match_count: exact.len(),
                matches: exact
                    .iter()
                    .map(|r| AmbiguousMatch {
                        id: r.id.clone(),
                        name: r.name.clone(),
                    })
                    .collect(),
            });
            return Ok(());
        }

        // 2. Substring search fallback.
        let matches = search_tags_lightweight(pool, value).await?;
        match matches.len() {
            1 => {
                let m = &matches[0];
                let id = Uuid::parse_str(&m.id)
                    .map_err(|e| DbError::Query(format!("invalid tag UUID: {e}")))?;
                if negated {
                    result.neg_tag_ids.push(id);
                } else {
                    result.tag_ids.push(id);
                }
            }
            0 => {
                result.warnings.push(QueryWarning::UnknownRelation {
                    field: "tag".to_owned(),
                    query: value.to_owned(),
                });
            }
            n => {
                result.warnings.push(QueryWarning::AmbiguousRelation {
                    field: "tag".to_owned(),
                    query: value.to_owned(),
                    match_count: n,
                    matches: matches
                        .into_iter()
                        .map(|m| AmbiguousMatch {
                            id: m.id,
                            name: m.name,
                        })
                        .collect(),
                });
            }
        }

        Ok(())
    }

    // ── Scalar field helpers ────────────────────────────────────

    /// Resolve a scalar enum field (format, status, resolution, outcome).
    fn resolve_scalar_enum<T: FromStr<Err = String>>(
        field_name: &str,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
    ) {
        if negated {
            result.warnings.push(QueryWarning::InvalidValue {
                field: field_name.to_owned(),
                value: value.to_owned(),
                reason: "negation not supported for this field".to_owned(),
            });
            return;
        }

        match T::from_str(value) {
            Ok(parsed) => {
                // Use `Any` to dispatch into the right field. Since we can't use
                // `Any` with generics cleanly, we match on `field_name` instead.
                match field_name {
                    "format" => {
                        // SAFETY: T is BookFormat when `field_name == "format"`.
                        // We re-parse to avoid unsafe; the cost is trivial.
                        if let Ok(v) = BookFormat::from_str(value) {
                            result.format = Some(v);
                        }
                    }
                    "status" => {
                        if let Ok(v) = MetadataStatus::from_str(value) {
                            result.metadata_status = Some(v);
                        }
                    }
                    "resolution" => {
                        if let Ok(v) = ResolutionState::from_str(value) {
                            result.resolution_state = Some(v);
                        }
                    }
                    "outcome" => {
                        if let Ok(v) = ResolutionOutcome::from_str(value) {
                            result.resolution_outcome = Some(v);
                        }
                    }
                    _ => {}
                }
                // Suppress "unused" warning from the generic parse.
                drop(parsed);
            }
            Err(reason) => {
                result.warnings.push(QueryWarning::InvalidValue {
                    field: field_name.to_owned(),
                    value: value.to_owned(),
                    reason,
                });
            }
        }
    }

    /// Resolve a boolean field (`trusted`, `locked`).
    fn resolve_bool_field(
        field_name: &str,
        value: &str,
        negated: bool,
        result: &mut ResolvedQuery,
    ) {
        if negated {
            result.warnings.push(QueryWarning::InvalidValue {
                field: field_name.to_owned(),
                value: value.to_owned(),
                reason: "negation not supported for this field".to_owned(),
            });
            return;
        }

        match parse_bool(value) {
            Some(v) => match field_name {
                "trusted" => result.trusted = Some(v),
                "locked" => result.locked = Some(v),
                _ => {}
            },
            None => {
                result.warnings.push(QueryWarning::InvalidValue {
                    field: field_name.to_owned(),
                    value: value.to_owned(),
                    reason: format!("expected boolean (true/false/yes/no/1/0), got '{value}'"),
                });
            }
        }
    }

    /// Resolve a `year` field supporting ranges like `1965..1970`, `>1965`, `>=1965`, etc.
    fn resolve_year(value: &str, negated: bool, result: &mut ResolvedQuery) {
        if negated {
            result.warnings.push(QueryWarning::InvalidValue {
                field: "year".to_owned(),
                value: value.to_owned(),
                reason: "negation not supported for this field".to_owned(),
            });
            return;
        }

        // Range: `1965..1970`
        if let Some((left, right)) = value.split_once("..") {
            match (left.parse::<i32>(), right.parse::<i32>()) {
                (Ok(min), Ok(max)) => {
                    result.year_min = Some(min);
                    result.year_max = Some(max);
                }
                _ => {
                    result.warnings.push(QueryWarning::InvalidValue {
                        field: "year".to_owned(),
                        value: value.to_owned(),
                        reason: "invalid year range, expected e.g. 1965..1970".to_owned(),
                    });
                }
            }
            return;
        }

        // Comparison operators: `>=`, `<=`, `>`, `<`
        if let Some(rest) = value.strip_prefix(">=") {
            match rest.parse::<i32>() {
                Ok(v) => result.year_min = Some(v),
                Err(_) => result.warnings.push(QueryWarning::InvalidValue {
                    field: "year".to_owned(),
                    value: value.to_owned(),
                    reason: "invalid year after '>='".to_owned(),
                }),
            }
            return;
        }
        if let Some(rest) = value.strip_prefix("<=") {
            match rest.parse::<i32>() {
                Ok(v) => result.year_max = Some(v),
                Err(_) => result.warnings.push(QueryWarning::InvalidValue {
                    field: "year".to_owned(),
                    value: value.to_owned(),
                    reason: "invalid year after '<='".to_owned(),
                }),
            }
            return;
        }
        if let Some(rest) = value.strip_prefix('>') {
            match rest.parse::<i32>() {
                Ok(v) => result.year_min = Some(v + 1),
                Err(_) => result.warnings.push(QueryWarning::InvalidValue {
                    field: "year".to_owned(),
                    value: value.to_owned(),
                    reason: "invalid year after '>'".to_owned(),
                }),
            }
            return;
        }
        if let Some(rest) = value.strip_prefix('<') {
            match rest.parse::<i32>() {
                Ok(v) => result.year_max = Some(v - 1),
                Err(_) => result.warnings.push(QueryWarning::InvalidValue {
                    field: "year".to_owned(),
                    value: value.to_owned(),
                    reason: "invalid year after '<'".to_owned(),
                }),
            }
            return;
        }

        // Exact year: `1965` → year_min=1965, year_max=1965
        match value.parse::<i32>() {
            Ok(v) => {
                result.year_min = Some(v);
                result.year_max = Some(v);
            }
            Err(_) => {
                result.warnings.push(QueryWarning::InvalidValue {
                    field: "year".to_owned(),
                    value: value.to_owned(),
                    reason: "invalid year, expected number or range".to_owned(),
                });
            }
        }
    }

    /// Resolve `has:` / `missing:` presence fields.
    ///
    /// `is_missing` is true for the `missing:` field (inverted sense).
    /// `negated` is true when the clause is prefixed with `-`.
    fn resolve_presence(value: &str, negated: bool, is_missing: bool, result: &mut ResolvedQuery) {
        // `has:cover` → has_cover = true
        // `missing:cover` → has_cover = false
        // `-has:cover` → has_cover = false (negation flips)
        // `-missing:cover` → has_cover = true (negation flips)
        let positive = !(negated ^ is_missing);

        match value.to_lowercase().as_str() {
            "cover" => result.has_cover = Some(positive),
            "description" | "desc" => result.has_description = Some(positive),
            "identifiers" | "identifier" | "ids" => result.has_identifiers = Some(positive),
            _ => {
                let field_name = if is_missing { "missing" } else { "has" };
                result.warnings.push(QueryWarning::InvalidValue {
                    field: field_name.to_owned(),
                    value: value.to_owned(),
                    reason: format!(
                        "unknown presence check '{value}', expected: cover, description, identifiers"
                    ),
                });
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Parse a boolean-ish string.
fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

// ── Lightweight DB lookups (no pagination overhead) ─────────────────

/// Simple name/id row for lightweight relation searches.
#[derive(sqlx::FromRow)]
struct NameRow {
    id: String,
    name: String,
}

async fn find_author_by_name(pool: &SqlitePool, name: &str) -> Result<Option<NameRow>, DbError> {
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM authors WHERE name = ? COLLATE NOCASE",
        name,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn find_series_by_name(pool: &SqlitePool, name: &str) -> Result<Option<NameRow>, DbError> {
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM series WHERE name = ? COLLATE NOCASE",
        name,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn find_publisher_by_name(pool: &SqlitePool, name: &str) -> Result<Option<NameRow>, DbError> {
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM publishers WHERE name = ? COLLATE NOCASE",
        name,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn find_tags_by_name(pool: &SqlitePool, name: &str) -> Result<Vec<NameRow>, DbError> {
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM tags WHERE name = ? COLLATE NOCASE",
        name,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn search_authors_lightweight(
    pool: &SqlitePool,
    query: &str,
) -> Result<Vec<NameRow>, DbError> {
    let pattern = format!("%{query}%");
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM authors WHERE name LIKE ? COLLATE NOCASE LIMIT 5",
        pattern,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn search_series_lightweight(
    pool: &SqlitePool,
    query: &str,
) -> Result<Vec<NameRow>, DbError> {
    let pattern = format!("%{query}%");
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM series WHERE name LIKE ? COLLATE NOCASE LIMIT 5",
        pattern,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn search_publishers_lightweight(
    pool: &SqlitePool,
    query: &str,
) -> Result<Vec<NameRow>, DbError> {
    let pattern = format!("%{query}%");
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM publishers WHERE name LIKE ? COLLATE NOCASE LIMIT 5",
        pattern,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

async fn search_tags_lightweight(pool: &SqlitePool, query: &str) -> Result<Vec<NameRow>, DbError> {
    let pattern = format!("%{query}%");
    sqlx::query_as!(
        NameRow,
        "SELECT id, name FROM tags WHERE name LIKE ? COLLATE NOCASE LIMIT 5",
        pattern,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DbError::Query(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_bool ──────────────────────────────────────────────

    #[test]
    fn parse_bool_truthy() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("True"), Some(true));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
    }

    #[test]
    fn parse_bool_falsy() {
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("False"), Some(false));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
    }

    #[test]
    fn parse_bool_invalid() {
        assert_eq!(parse_bool("maybe"), None);
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("2"), None);
    }

    // ── Year parsing (via resolve_year on a scratch ResolvedQuery) ──

    fn year(input: &str) -> ResolvedQuery {
        let mut result = ResolvedQuery::default();
        SearchResolver::resolve_year(input, false, &mut result);
        result
    }

    #[test]
    fn year_exact() {
        let r = year("1965");
        assert_eq!(r.year_min, Some(1965));
        assert_eq!(r.year_max, Some(1965));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn year_range() {
        let r = year("1965..1970");
        assert_eq!(r.year_min, Some(1965));
        assert_eq!(r.year_max, Some(1970));
    }

    #[test]
    fn year_gt() {
        let r = year(">1965");
        assert_eq!(r.year_min, Some(1966));
        assert_eq!(r.year_max, None);
    }

    #[test]
    fn year_gte() {
        let r = year(">=1965");
        assert_eq!(r.year_min, Some(1965));
        assert_eq!(r.year_max, None);
    }

    #[test]
    fn year_lt() {
        let r = year("<1970");
        assert_eq!(r.year_min, None);
        assert_eq!(r.year_max, Some(1969));
    }

    #[test]
    fn year_lte() {
        let r = year("<=1970");
        assert_eq!(r.year_min, None);
        assert_eq!(r.year_max, Some(1970));
    }

    #[test]
    fn year_invalid() {
        let r = year("abc");
        assert!(r.year_min.is_none());
        assert!(r.year_max.is_none());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn year_negated_warns() {
        let mut result = ResolvedQuery::default();
        SearchResolver::resolve_year("1965", true, &mut result);
        assert_eq!(result.warnings.len(), 1);
    }

    // ── Presence fields ─────────────────────────────────────────

    #[test]
    fn has_cover() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("cover", false, false, &mut r);
        assert_eq!(r.has_cover, Some(true));
    }

    #[test]
    fn missing_cover() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("cover", false, true, &mut r);
        assert_eq!(r.has_cover, Some(false));
    }

    #[test]
    fn negated_has_cover() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("cover", true, false, &mut r);
        assert_eq!(r.has_cover, Some(false));
    }

    #[test]
    fn negated_missing_cover() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("cover", true, true, &mut r);
        assert_eq!(r.has_cover, Some(true));
    }

    #[test]
    fn has_description() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("description", false, false, &mut r);
        assert_eq!(r.has_description, Some(true));
    }

    #[test]
    fn has_identifiers() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("identifiers", false, false, &mut r);
        assert_eq!(r.has_identifiers, Some(true));
    }

    #[test]
    fn has_unknown_warns() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_presence("foobar", false, false, &mut r);
        assert_eq!(r.warnings.len(), 1);
    }

    // ── Text resolution ─────────────────────────────────────────

    #[test]
    fn text_plain() {
        let mut parts = Vec::new();
        SearchResolver::resolve_text("dune messiah", false, &mut parts);
        assert_eq!(parts, vec!["dune messiah"]);
    }

    #[test]
    fn text_negated() {
        let mut parts = Vec::new();
        SearchResolver::resolve_text("dune messiah", true, &mut parts);
        assert_eq!(parts, vec!["NOT dune NOT messiah"]);
    }

    #[test]
    fn phrase_plain() {
        let mut parts = Vec::new();
        SearchResolver::resolve_phrase("the final empire", false, &mut parts);
        assert_eq!(parts, vec!["\"the final empire\""]);
    }

    #[test]
    fn phrase_negated() {
        let mut parts = Vec::new();
        SearchResolver::resolve_phrase("bad book", true, &mut parts);
        assert_eq!(parts, vec!["NOT \"bad book\""]);
    }

    // ── Scalar enum parsing ─────────────────────────────────────

    #[test]
    fn format_epub() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_scalar_enum::<BookFormat>("format", "epub", false, &mut r);
        assert_eq!(r.format, Some(BookFormat::Epub));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn format_invalid() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_scalar_enum::<BookFormat>("format", "docx", false, &mut r);
        assert!(r.format.is_none());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn format_negated_warns() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_scalar_enum::<BookFormat>("format", "epub", true, &mut r);
        assert!(r.format.is_none());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn status_identified() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_scalar_enum::<MetadataStatus>(
            "status",
            "identified",
            false,
            &mut r,
        );
        assert_eq!(r.metadata_status, Some(MetadataStatus::Identified));
    }

    #[test]
    fn resolution_pending() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_scalar_enum::<ResolutionState>(
            "resolution",
            "pending",
            false,
            &mut r,
        );
        assert_eq!(r.resolution_state, Some(ResolutionState::Pending));
    }

    #[test]
    fn outcome_confirmed() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_scalar_enum::<ResolutionOutcome>(
            "outcome",
            "confirmed",
            false,
            &mut r,
        );
        assert_eq!(r.resolution_outcome, Some(ResolutionOutcome::Confirmed));
    }

    // ── Boolean fields ──────────────────────────────────────────

    #[test]
    fn trusted_true() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_bool_field("trusted", "true", false, &mut r);
        assert_eq!(r.trusted, Some(true));
    }

    #[test]
    fn locked_no() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_bool_field("locked", "no", false, &mut r);
        assert_eq!(r.locked, Some(false));
    }

    #[test]
    fn bool_invalid() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_bool_field("trusted", "maybe", false, &mut r);
        assert!(r.trusted.is_none());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn bool_negated_warns() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_bool_field("trusted", "true", true, &mut r);
        assert!(r.trusted.is_none());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn identifier_untyped() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_identifier(
            QueryField::Identifier,
            "9780451524935",
            false,
            &mut r,
        );
        assert_eq!(r.identifier_type, None);
        assert_eq!(r.identifier_value.as_deref(), Some("9780451524935"));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn identifier_typed_and_normalized() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_identifier(
            QueryField::Identifier,
            "isbn:978-0-451-52493-5",
            false,
            &mut r,
        );
        assert_eq!(r.identifier_type.as_deref(), Some("isbn"));
        assert_eq!(r.identifier_value.as_deref(), Some("9780451524935"));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn identifier_unknown_type_warns() {
        let mut r = ResolvedQuery::default();
        SearchResolver::resolve_identifier(
            QueryField::Identifier,
            "mystery:123",
            false,
            &mut r,
        );
        assert_eq!(r.identifier_value, None);
        assert!(matches!(
            &r.warnings[0],
            QueryWarning::InvalidValue { field, .. } if field == "identifier"
        ));
    }

    // ── OR group resolution ──────────────────────────────────────

    async fn or_pool() -> SqlitePool {
        SqlitePool::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn or_text_only_produces_fts_or() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
            QueryClause::Text {
                text: "dune".into(),
                negated: false,
            },
            QueryClause::Text {
                text: "foundation".into(),
                negated: false,
            },
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert_eq!(text_parts, vec!["dune OR foundation"]);
        assert!(result.warnings.is_empty());
    }

    #[tokio::test]
    async fn or_field_only_warns_and_drops() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
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
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert!(text_parts.is_empty(), "no FTS text should be produced");
        assert!(result.author_id.is_none(), "author_id should not be set");
        assert_eq!(result.warnings.len(), 2);
        assert!(matches!(
            &result.warnings[0],
            QueryWarning::UnsupportedOrField { field, value, negated }
                if field == "author" && value == "asimov" && !negated
        ));
        assert!(matches!(
            &result.warnings[1],
            QueryWarning::UnsupportedOrField { field, value, negated }
                if field == "author" && value == "clarke" && !negated
        ));
    }

    #[tokio::test]
    async fn or_mixed_keeps_text_warns_field() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
            QueryClause::Text {
                text: "dune".into(),
                negated: false,
            },
            QueryClause::Field {
                field: QueryField::Author,
                value: "asimov".into(),
                negated: false,
            },
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert_eq!(text_parts, vec!["dune"]);
        assert!(result.author_id.is_none());
        assert_eq!(result.warnings.len(), 1);
        assert!(matches!(
            &result.warnings[0],
            QueryWarning::UnsupportedOrField { field, .. } if field == "author"
        ));
    }

    #[tokio::test]
    async fn or_fts_column_filter_warns() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
            QueryClause::Field {
                field: QueryField::Title,
                value: "dune".into(),
                negated: false,
            },
            QueryClause::Field {
                field: QueryField::Title,
                value: "foundation".into(),
                negated: false,
            },
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert!(
            result.fts_column_filters.is_empty(),
            "no column filters should be pushed"
        );
        assert_eq!(result.warnings.len(), 2);
    }

    #[tokio::test]
    async fn or_phrases_produces_fts_or() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
            QueryClause::Phrase {
                phrase: "the hobbit".into(),
                negated: false,
            },
            QueryClause::Phrase {
                phrase: "the lord".into(),
                negated: false,
            },
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert_eq!(text_parts, vec!["\"the hobbit\" OR \"the lord\""]);
        assert!(result.warnings.is_empty());
    }

    #[tokio::test]
    async fn or_mixed_negated_field_warns() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
            QueryClause::Text {
                text: "dune".into(),
                negated: false,
            },
            QueryClause::Field {
                field: QueryField::Author,
                value: "asimov".into(),
                negated: true,
            },
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert_eq!(text_parts, vec!["dune"]);
        assert_eq!(result.warnings.len(), 1);
        assert!(matches!(
            &result.warnings[0],
            QueryWarning::UnsupportedOrField { field, value, negated }
                if field == "author" && value == "asimov" && *negated
        ));
    }

    #[tokio::test]
    async fn or_negated_and_positive_field_warns() {
        let pool = or_pool().await;
        let mut result = ResolvedQuery::default();
        let mut text_parts = Vec::new();
        let clauses = vec![
            QueryClause::Field {
                field: QueryField::Author,
                value: "asimov".into(),
                negated: false,
            },
            QueryClause::Field {
                field: QueryField::Author,
                value: "asimov".into(),
                negated: true,
            },
        ];
        SearchResolver::resolve_or(&pool, &clauses, &mut result, &mut text_parts)
            .await
            .unwrap();
        assert!(text_parts.is_empty());
        assert_eq!(result.warnings.len(), 2);
        assert!(matches!(
            &result.warnings[0],
            QueryWarning::UnsupportedOrField { negated, .. } if !negated
        ));
        assert!(matches!(
            &result.warnings[1],
            QueryWarning::UnsupportedOrField { negated, .. } if *negated
        ));
    }
}
