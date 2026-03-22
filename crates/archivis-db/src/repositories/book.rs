use std::collections::{HashMap, HashSet};

use archivis_core::errors::DbError;
use archivis_core::models::{
    Author, Book, BookFile, Identifier, LibraryFilterState, MetadataProvenance, MetadataStatus,
    ResolutionOutcome, ResolutionState, Series, Tag, TagMatchMode,
};
use chrono::{DateTime, Utc};
use sqlx::{Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::types::{BookFilter, PaginatedResult, PaginationParams};

/// A book with all its related entities loaded.
#[derive(Debug, Clone)]
pub struct BookWithRelations {
    pub book: Book,
    pub authors: Vec<BookAuthorEntry>,
    pub series: Vec<BookSeriesEntry>,
    pub files: Vec<BookFile>,
    pub identifiers: Vec<Identifier>,
    pub tags: Vec<Tag>,
    pub publisher_name: Option<String>,
}

/// An author entry with role and position in a book.
#[derive(Debug, Clone)]
pub struct BookAuthorEntry {
    pub author: Author,
    pub role: String,
    pub position: i64,
}

/// A series entry with position.
#[derive(Debug, Clone)]
pub struct BookSeriesEntry {
    pub series: Series,
    pub position: Option<f64>,
}

/// A book with its author names pre-loaded (for duplicate detection).
#[derive(Debug, Clone)]
pub struct BookWithAuthors {
    pub book: Book,
    pub author_names: Vec<String>,
}

pub struct BookRepository;

fn serialize_json<T: serde::Serialize>(value: &T, context: &str) -> Result<String, DbError> {
    serde_json::to_string(value)
        .map_err(|e| DbError::Query(format!("failed to serialize {context}: {e}")))
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(value)
        .map(|d| d.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|ndt| ndt.and_utc())
        })
        .map_err(|e| DbError::Query(format!("invalid {field}: {e}")))
}

fn serialize_metadata_status(status: MetadataStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "unidentified".into())
}

fn serialize_resolution_state(state: ResolutionState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "pending".into())
}

fn serialize_resolution_outcome(outcome: Option<ResolutionOutcome>) -> Option<String> {
    outcome.and_then(|value| {
        serde_json::to_value(value)
            .ok()
            .and_then(|serialized| serialized.as_str().map(String::from))
    })
}

// ── Dynamic query builder helpers ─────────────────────────────

/// Column list for book SELECT queries (matches `BookRow` fields).
const BOOK_COLUMNS: &str = "\
    b.id, b.title, b.subtitle, b.sort_title, b.description, b.language, \
    b.publication_year, b.publisher_id, b.added_at, b.updated_at, b.rating, \
    b.page_count, b.metadata_status, b.review_baseline_metadata_status, \
    b.review_baseline_resolution_outcome, b.ingest_quality_score, \
    b.metadata_quality_score, b.resolution_state, b.resolution_outcome, \
    b.resolution_requested_at, b.resolution_requested_reason, b.last_resolved_at, \
    b.last_resolution_run_id, b.metadata_locked, b.metadata_user_trusted, \
    b.metadata_provenance, b.cover_path";

/// Token types for [`escape_fts_text`].
enum FtsTok {
    Quoted(String),    // "phrase" — passed through verbatim
    Operator(String),  // OR, NOT — intentional FTS5 operators
    EscapedOp(String), // AND, NEAR — must be wrapped in quotes
    Regular(String),   // plain search term — candidate for prefix `*`
}

/// Escape remaining plain-text query for FTS5 and append a prefix `*` to the
/// last eligible token so partial input matches indexed terms.
///
/// The input comes from the DSL resolver's `text_query` — it already contains
/// valid FTS5 syntax such as `NOT term` and `a OR b`.  We escape terms that
/// *accidentally* look like FTS5 operators (`AND`, `NEAR`) and append a
/// trailing `*` to the last unquoted, non-operator, non-negated token to
/// enable prefix matching via the FTS5 prefix indexes.
fn escape_fts_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // ── Pass 1: tokenize ────────────────────────────────────────
    let mut tokens: Vec<FtsTok> = Vec::new();
    let mut chars = trimmed.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c == '"' {
            let mut phrase = String::new();
            phrase.push(chars.next().unwrap()); // opening "
            loop {
                match chars.next() {
                    Some('"') | None => {
                        phrase.push('"');
                        break;
                    }
                    Some(ch) => phrase.push(ch),
                }
            }
            tokens.push(FtsTok::Quoted(phrase));
        } else if c.is_whitespace() {
            chars.next();
        } else {
            let mut term = String::new();
            while let Some(&tc) = chars.peek() {
                if tc.is_whitespace() || tc == '"' {
                    break;
                }
                term.push(chars.next().unwrap());
            }

            let upper = term.to_uppercase();
            if term == "OR" || term == "NOT" {
                tokens.push(FtsTok::Operator(term));
            } else if matches!(upper.as_str(), "AND" | "NEAR") {
                tokens.push(FtsTok::EscapedOp(term));
            } else {
                tokens.push(FtsTok::Regular(term));
            }
        }
    }

    // ── Pass 2: append `*` to the last eligible regular token ───
    let mut last_prefix_idx: Option<usize> = None;
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(tok, FtsTok::Regular(_)) {
            let preceded_by_not =
                i > 0 && matches!(&tokens[i - 1], FtsTok::Operator(op) if op == "NOT");
            if !preceded_by_not {
                last_prefix_idx = Some(i);
            }
        }
    }
    if let Some(idx) = last_prefix_idx {
        if let FtsTok::Regular(ref mut term) = tokens[idx] {
            if !term.ends_with('*') {
                term.push('*');
            }
        }
    }

    // ── Pass 3: build output ────────────────────────────────────
    let mut result = String::with_capacity(trimmed.len() + 8);
    for (i, tok) in tokens.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        match tok {
            FtsTok::Quoted(s) | FtsTok::Operator(s) | FtsTok::Regular(s) => {
                result.push_str(s);
            }
            FtsTok::EscapedOp(s) => {
                result.push('"');
                result.push_str(s);
                result.push('"');
            }
        }
    }

    result
}

/// Escape a single term for FTS5 column-filter use, appending a prefix `*`
/// to single-word terms so partial input matches indexed tokens.
fn escape_fts_term(term: &str) -> String {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Multi-word term: wrap in quotes (exact phrase match, no prefix).
    if trimmed.contains(' ') {
        return format!("\"{trimmed}\"");
    }
    // Single-word term: ensure trailing `*` for prefix matching.
    if trimmed.ends_with('*') {
        trimmed.to_owned()
    } else {
        format!("{trimmed}*")
    }
}

/// Compile the FTS5 MATCH expression from the resolved query components.
///
/// Combines:
/// 1. `filter.query` — remaining plain text/phrases (post-DSL extraction)
/// 2. `filter.fts_column_filters` — column-qualified terms from DSL
///
/// Returns `None` if there is nothing to match against.
fn compile_fts_match(filter: &BookFilter) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    // Add plain text query with FTS5 operator escaping
    if let Some(ref q) = filter.query {
        let escaped = escape_fts_text(q);
        if !escaped.is_empty() {
            parts.push(escaped);
        }
    }

    // Add column-qualified filters
    for (column, term, negated) in &filter.fts_column_filters {
        let escaped_term = escape_fts_term(term);
        if escaped_term.is_empty() {
            continue;
        }
        if *negated {
            parts.push(format!("NOT {column} : {escaped_term}"));
        } else {
            parts.push(format!("{column} : {escaped_term}"));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Serialize a serde-serializable enum to its DB string representation.
///
/// Uses serde's `rename_all` to produce the correct DB value (e.g. `"snake_case"`),
/// which may differ from the `Display` impl used for human-readable output.
fn serialize_enum_to_db<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// Append WHERE conditions to a `QueryBuilder` based on filter criteria.
///
/// Assumes the caller has already pushed `FROM books b` (and optional FTS JOIN).
#[allow(clippy::too_many_lines)]
fn build_where_clause(
    qb: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    filter: &BookFilter,
    fts_query: Option<String>,
) {
    qb.push(" WHERE 1=1");

    if let Some(fts_q) = fts_query {
        qb.push(" AND books_fts MATCH ").push_bind(fts_q);
    }

    if let Some(ref format) = filter.format {
        qb.push(" AND b.id IN (SELECT book_id FROM book_files WHERE format = ")
            .push_bind(serialize_enum_to_db(format))
            .push(")");
    }

    if let Some(ref status) = filter.status {
        qb.push(" AND b.metadata_status = ")
            .push_bind(serialize_enum_to_db(status));
    }

    if let Some(ref author_id) = filter.author_id {
        qb.push(" AND b.id IN (SELECT book_id FROM book_authors WHERE author_id = ")
            .push_bind(author_id.clone())
            .push(")");
    }

    if let Some(ref series_id) = filter.series_id {
        qb.push(" AND b.id IN (SELECT book_id FROM book_series WHERE series_id = ")
            .push_bind(series_id.clone())
            .push(")");
    }

    if let Some(ref tags) = filter.tags {
        if !tags.is_empty() {
            match filter.tag_match {
                TagMatchMode::Any => {
                    qb.push(" AND b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (");
                    let mut sep = qb.separated(", ");
                    for tag in tags {
                        sep.push_bind(tag.clone());
                    }
                    sep.push_unseparated("))");
                }
                TagMatchMode::All => {
                    qb.push(" AND b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (");
                    let mut sep = qb.separated(", ");
                    for tag in tags {
                        sep.push_bind(tag.clone());
                    }
                    sep.push_unseparated(") GROUP BY book_id HAVING COUNT(DISTINCT tag_id) = ");
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    qb.push_bind(tags.len() as i32).push(")");
                }
            }
        }
    }

    if let Some(ref neg_tags) = filter.neg_tag_ids {
        if !neg_tags.is_empty() {
            qb.push(" AND b.id NOT IN (SELECT book_id FROM book_tags WHERE tag_id IN (");
            let mut sep = qb.separated(", ");
            for tag in neg_tags {
                sep.push_bind(tag.clone());
            }
            sep.push_unseparated("))");
        }
    }

    if let Some(ref publisher_id) = filter.publisher_id {
        qb.push(" AND b.publisher_id = ")
            .push_bind(publisher_id.clone());
    }

    if let Some(trusted) = filter.trusted {
        qb.push(" AND b.metadata_user_trusted = ")
            .push_bind(i32::from(trusted));
    }

    if let Some(locked) = filter.locked {
        qb.push(" AND b.metadata_locked = ")
            .push_bind(i32::from(locked));
    }

    if let Some(ref state) = filter.resolution_state {
        qb.push(" AND b.resolution_state = ")
            .push_bind(serialize_enum_to_db(state));
    }

    if let Some(ref outcome) = filter.resolution_outcome {
        qb.push(" AND b.resolution_outcome = ")
            .push_bind(serialize_enum_to_db(outcome));
    }

    if let Some(ref lang) = filter.language {
        qb.push(" AND b.language = ").push_bind(lang.clone());
    }

    if let Some(year_min) = filter.year_min {
        qb.push(" AND b.publication_year >= ").push_bind(year_min);
    }
    if let Some(year_max) = filter.year_max {
        qb.push(" AND b.publication_year <= ").push_bind(year_max);
    }

    if let Some(has_cover) = filter.has_cover {
        if has_cover {
            qb.push(" AND b.cover_path IS NOT NULL AND b.cover_path != ''");
        } else {
            qb.push(" AND (b.cover_path IS NULL OR b.cover_path = '')");
        }
    }

    if let Some(has_desc) = filter.has_description {
        if has_desc {
            qb.push(" AND b.description IS NOT NULL AND b.description != ''");
        } else {
            qb.push(" AND (b.description IS NULL OR b.description = '')");
        }
    }

    if let Some(has_ids) = filter.has_identifiers {
        if has_ids {
            qb.push(" AND b.id IN (SELECT DISTINCT book_id FROM identifiers)");
        } else {
            qb.push(" AND b.id NOT IN (SELECT DISTINCT book_id FROM identifiers)");
        }
    }

    if let (Some(ref id_types), Some(ref id_value)) =
        (&filter.identifier_types, &filter.identifier_value)
    {
        if !id_types.is_empty() {
            qb.push(" AND b.id IN (SELECT book_id FROM identifiers WHERE identifier_type IN (");
            let mut sep = qb.separated(", ");
            for t in id_types {
                sep.push_bind(t.clone());
            }
            sep.push_unseparated(") AND value = ");
            qb.push_bind(id_value.clone()).push(")");
        }
    }
}

/// Append ORDER BY clause to a `QueryBuilder`.
///
/// `raw_query` is the original search text (before FTS compilation). When
/// present and sorting by relevance, the function blends `bm25()` with
/// exact-match and prefix-match boosts so that precise hits sort first.
fn append_order_by(
    qb: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    params: &PaginationParams,
    has_fts: bool,
    raw_query: Option<&str>,
) {
    let dir = params.sort_order.as_sql();

    match params.sort_by.as_str() {
        "relevance" => {
            if has_fts {
                // bm25 returns negative values; lower = more relevant, so always ASC.
                // Column 0 is `book_id UNINDEXED` (weight 0 — never matches).
                // Weights: title, description, author_names, series_names, publisher_name, tag_names
                //
                // Boost blending: subtract bonuses so boosted rows sort earlier.
                //   - Exact `norm_title` match:  -20
                //   - `norm_title` prefix match: -5
                //   - Exact identifier value:    -15
                qb.push(" ORDER BY bm25(books_fts, 0.0, 10.0, 1.0, 5.0, 3.0, 2.0, 2.0)");

                if let Some(q) = raw_query.filter(|s| !s.trim().is_empty()) {
                    let norm_query = archivis_core::models::normalize_title(q);
                    let norm_prefix = format!("{norm_query}%");

                    qb.push(" - CASE WHEN b.norm_title = ");
                    qb.push_bind(norm_query);
                    qb.push(" THEN 20.0 ELSE 0.0 END");

                    qb.push(" - CASE WHEN b.norm_title LIKE ");
                    qb.push_bind(norm_prefix);
                    qb.push(" THEN 5.0 ELSE 0.0 END");

                    qb.push(
                        " - CASE WHEN EXISTS(\
                            SELECT 1 FROM identifiers i \
                            WHERE i.book_id = b.id AND i.value = ",
                    );
                    qb.push_bind(q.trim().to_owned());
                    qb.push(") THEN 15.0 ELSE 0.0 END");
                }
            } else {
                qb.push(" ORDER BY b.sort_title ASC");
            }
        }
        "title" | "sort_title" => {
            qb.push(" ORDER BY b.sort_title ").push(dir);
        }
        "updated_at" => {
            qb.push(" ORDER BY b.updated_at ").push(dir);
        }
        "rating" => {
            qb.push(" ORDER BY b.rating ").push(dir);
        }
        "metadata_status" => {
            qb.push(" ORDER BY b.metadata_status ").push(dir);
        }
        "author" => {
            qb.push(
                " ORDER BY (SELECT MIN(a.sort_name) FROM book_authors ba \
                 JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id) ",
            )
            .push(dir);
        }
        "series" => {
            qb.push(
                " ORDER BY (SELECT MIN(s.name) FROM book_series bs \
                 JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id) ",
            )
            .push(dir);
            qb.push(", (SELECT MIN(bs.position) FROM book_series bs WHERE bs.book_id = b.id) ")
                .push(dir);
        }
        _ => {
            // Default: `added_at`
            qb.push(" ORDER BY b.added_at ").push(dir);
        }
    }
}

// ── LibraryFilterState → BookFilter ────────────────────────────

/// Resolve the active identifier filter from `LibraryFilterState` into
/// the DB-level type(s) and value. ISBN is special: the DB stores
/// `isbn10` and `isbn13` separately, so we expand to both.
fn resolve_identifier_filter(lfs: &LibraryFilterState) -> (Option<Vec<String>>, Option<String>) {
    let candidates: &[(&[&str], &Option<String>)] = &[
        (&["isbn10", "isbn13"], &lfs.isbn),
        (&["asin"], &lfs.asin),
        (&["open_library"], &lfs.open_library_id),
        (&["hardcover"], &lfs.hardcover_id),
    ];
    for &(types, val) in candidates {
        if let Some(v) = val {
            return (
                Some(types.iter().map(|s| (*s).to_string()).collect()),
                Some(v.clone()),
            );
        }
    }
    (None, None)
}

impl From<&LibraryFilterState> for BookFilter {
    fn from(lfs: &LibraryFilterState) -> Self {
        let (identifier_types, identifier_value) = resolve_identifier_filter(lfs);

        Self {
            query: lfs.text_query.clone(),
            format: lfs.format,
            status: lfs.metadata_status,
            tags: if lfs.tag_ids.is_empty() {
                None
            } else {
                Some(lfs.tag_ids.iter().map(ToString::to_string).collect())
            },
            tag_match: lfs.tag_match,
            author_id: lfs.author_id.map(|id| id.to_string()),
            series_id: lfs.series_id.map(|id| id.to_string()),
            publisher_id: lfs.publisher_id.map(|id| id.to_string()),
            trusted: lfs.trusted,
            locked: lfs.locked,
            resolution_state: lfs.resolution_state,
            resolution_outcome: lfs.resolution_outcome,
            language: lfs.language.clone(),
            year_min: lfs.year_min,
            year_max: lfs.year_max,
            has_cover: lfs.has_cover,
            has_description: lfs.has_description,
            has_identifiers: lfs.has_identifiers,
            identifier_types,
            identifier_value,
            neg_tag_ids: None,
            fts_column_filters: Vec::new(),
        }
    }
}

impl BookFilter {
    /// Create a filter from a merged [`LibraryFilterState`] plus the
    /// resolver's negation and FTS column data.
    pub fn from_resolved(
        lfs: &LibraryFilterState,
        resolved: &super::search_resolve::ResolvedQuery,
    ) -> Self {
        let mut filter = Self::from(lfs);

        // Override `text_query` with the resolver's cleaned-up version
        filter.query.clone_from(&resolved.text_query);

        // Add negated tag IDs
        if !resolved.neg_tag_ids.is_empty() {
            filter.neg_tag_ids = Some(
                resolved
                    .neg_tag_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            );
        }

        // Add FTS column filters
        filter.fts_column_filters = resolved
            .fts_column_filters
            .iter()
            .map(|f| (f.column.clone(), f.term.clone(), f.negated))
            .collect();

        filter
    }
}

// ── Batch relations loading ───────────────────────────────────

/// Pre-loaded relations for a batch of books.
#[derive(Debug, Clone, Default)]
pub struct RelationsBundle {
    pub authors: Option<Vec<BookAuthorEntry>>,
    pub series: Option<Vec<BookSeriesEntry>>,
    pub tags: Option<Vec<Tag>>,
    pub files: Option<Vec<BookFile>>,
}

impl BookRepository {
    pub async fn create(pool: &SqlitePool, book: &Book) -> Result<(), DbError> {
        let id = book.id.to_string();
        let publisher_id = book.publisher_id.map(|p| p.to_string());
        let pub_year = book.publication_year.map(i64::from);
        let added_at = book.added_at.to_rfc3339();
        let updated_at = book.updated_at.to_rfc3339();
        let status = serialize_metadata_status(book.metadata_status);
        let resolution_state = serialize_resolution_state(book.resolution_state);
        let resolution_outcome = serialize_resolution_outcome(book.resolution_outcome);
        let resolution_requested_at = book.resolution_requested_at.to_rfc3339();
        let last_resolved_at = book.last_resolved_at.map(|value| value.to_rfc3339());
        let last_resolution_run_id = book.last_resolution_run_id.map(|value| value.to_string());
        let metadata_locked = i64::from(book.metadata_locked);
        let metadata_user_trusted = i64::from(book.metadata_user_trusted);
        let metadata_provenance = serialize_json(&book.metadata_provenance, "metadata provenance")?;
        let norm_title = archivis_core::models::normalize_title(&book.title);
        let review_baseline = book
            .review_baseline_metadata_status
            .map(serialize_metadata_status);
        let review_baseline_outcome =
            serialize_resolution_outcome(book.review_baseline_resolution_outcome);

        let metadata_quality_score = book.metadata_quality_score.map(f64::from);

        sqlx::query!(
            "INSERT INTO books (id, title, subtitle, sort_title, description, language, publication_year, publisher_id, added_at, updated_at, rating, page_count, metadata_status, review_baseline_metadata_status, review_baseline_resolution_outcome, ingest_quality_score, metadata_quality_score, resolution_state, resolution_outcome, resolution_requested_at, resolution_requested_reason, last_resolved_at, last_resolution_run_id, metadata_locked, metadata_user_trusted, metadata_provenance, cover_path, norm_title)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            id,
            book.title,
            book.subtitle,
            book.sort_title,
            book.description,
            book.language,
            pub_year,
            publisher_id,
            added_at,
            updated_at,
            book.rating,
            book.page_count,
            status,
            review_baseline,
            review_baseline_outcome,
            book.ingest_quality_score,
            metadata_quality_score,
            resolution_state,
            resolution_outcome,
            resolution_requested_at,
            book.resolution_requested_reason,
            last_resolved_at,
            last_resolution_run_id,
            metadata_locked,
            metadata_user_trusted,
            metadata_provenance,
            book.cover_path,
            norm_title,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Book, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            BookRow,
            "SELECT id, title, subtitle, sort_title, description, language, publication_year, publisher_id, added_at, updated_at, rating, page_count, metadata_status, review_baseline_metadata_status, review_baseline_resolution_outcome, ingest_quality_score, metadata_quality_score, resolution_state, resolution_outcome, resolution_requested_at, resolution_requested_reason, last_resolved_at, last_resolution_run_id, metadata_locked, metadata_user_trusted, metadata_provenance, cover_path FROM books WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "book",
            id: id_str,
        })?;

        row.into_book()
    }

    pub async fn get_by_id_conn(conn: &mut SqliteConnection, id: Uuid) -> Result<Book, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            BookRow,
            "SELECT id, title, subtitle, sort_title, description, language, publication_year, publisher_id, added_at, updated_at, rating, page_count, metadata_status, review_baseline_metadata_status, review_baseline_resolution_outcome, ingest_quality_score, metadata_quality_score, resolution_state, resolution_outcome, resolution_requested_at, resolution_requested_reason, last_resolved_at, last_resolution_run_id, metadata_locked, metadata_user_trusted, metadata_provenance, cover_path FROM books WHERE id = ?",
            id_str,
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "book",
            id: id_str,
        })?;

        row.into_book()
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
        filter: &BookFilter,
    ) -> Result<PaginatedResult<Book>, DbError> {
        let fts_query = compile_fts_match(filter);
        let has_fts = fts_query.is_some();

        // ── COUNT ────────────────────────────────────────────
        let mut count_qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT COUNT(*) FROM books b");
        if has_fts {
            count_qb.push(" JOIN books_fts ON books_fts.book_id = b.id");
        }
        build_where_clause(&mut count_qb, filter, fts_query.clone());

        let count_row = count_qb
            .build()
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        let total: i32 = count_row.get(0);

        // ── SELECT ───────────────────────────────────────────
        let mut qb =
            sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!("SELECT {BOOK_COLUMNS} FROM books b"));
        if has_fts {
            qb.push(" JOIN books_fts ON books_fts.book_id = b.id");
        }
        build_where_clause(&mut qb, filter, fts_query);
        append_order_by(&mut qb, params, has_fts, filter.query.as_deref());
        #[allow(clippy::cast_possible_wrap)]
        {
            qb.push(" LIMIT ")
                .push_bind(params.per_page as i32)
                .push(" OFFSET ")
                .push_bind(params.offset() as i32);
        }

        let rows: Vec<BookRow> = qb
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let books = rows
            .into_iter()
            .map(BookRow::into_book)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss)]
        Ok(PaginatedResult::new(books, total as u32, params))
    }

    /// Return the number of books matching `filter` (no pagination).
    pub async fn count(pool: &SqlitePool, filter: &BookFilter) -> Result<u64, DbError> {
        let fts_query = compile_fts_match(filter);

        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT COUNT(*) FROM books b");
        if fts_query.is_some() {
            qb.push(" JOIN books_fts ON books_fts.book_id = b.id");
        }
        build_where_clause(&mut qb, filter, fts_query);

        let row = qb
            .build()
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        let total: i32 = row.get(0);

        #[allow(clippy::cast_sign_loss)]
        Ok(total as u64)
    }

    /// Return IDs of all books matching `filter` (no pagination).
    pub async fn list_ids(pool: &SqlitePool, filter: &BookFilter) -> Result<Vec<Uuid>, DbError> {
        let fts_query = compile_fts_match(filter);

        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT b.id FROM books b");
        if fts_query.is_some() {
            qb.push(" JOIN books_fts ON books_fts.book_id = b.id");
        }
        build_where_clause(&mut qb, filter, fts_query);

        let rows: Vec<(String,)> = qb
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(|(id,)| {
                Uuid::parse_str(&id).map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))
            })
            .collect()
    }

    /// Return IDs of all books matching `filter`, minus `excluded_ids`.
    ///
    /// Centralizes the `list_ids() - exclusions` pattern used by both the API
    /// sync path and the background bulk worker. Duplicates and out-of-scope
    /// entries in `excluded_ids` are silently ignored.
    pub async fn resolve_scope(
        pool: &SqlitePool,
        filter: &BookFilter,
        excluded_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, DbError> {
        let all_ids = Self::list_ids(pool, filter).await?;
        if excluded_ids.is_empty() {
            return Ok(all_ids);
        }
        let exclude_set: HashSet<Uuid> = excluded_ids.iter().copied().collect();
        Ok(all_ids
            .into_iter()
            .filter(|id| !exclude_set.contains(id))
            .collect())
    }

    /// Return the exact count of books matching `filter`, minus `excluded_ids`.
    ///
    /// Runs entirely in SQL — does **not** materialise the full ID set.
    /// Duplicates and out-of-scope entries in `excluded_ids` are silently
    /// ignored (deduped before binding, and the `NOT IN` naturally skips
    /// IDs that don't match the filter).
    pub async fn count_scope(
        pool: &SqlitePool,
        filter: &BookFilter,
        excluded_ids: &[Uuid],
    ) -> Result<u64, DbError> {
        if excluded_ids.is_empty() {
            return Self::count(pool, filter).await;
        }

        let fts_query = compile_fts_match(filter);

        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT COUNT(*) FROM books b");
        if fts_query.is_some() {
            qb.push(" JOIN books_fts ON books_fts.book_id = b.id");
        }
        build_where_clause(&mut qb, filter, fts_query);

        // Dedup exclusions before binding to keep the query compact.
        let unique_excluded: HashSet<String> =
            excluded_ids.iter().map(ToString::to_string).collect();

        qb.push(" AND b.id NOT IN (");
        let mut sep = qb.separated(", ");
        for id in &unique_excluded {
            sep.push_bind(id.clone());
        }
        sep.push_unseparated(")");

        let row = qb
            .build()
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        let total: i32 = row.get(0);

        #[allow(clippy::cast_sign_loss)]
        Ok(total as u64)
    }

    /// Batch-load requested relations for a set of book IDs.
    ///
    /// Runs one query per requested relation type with `WHERE book_id IN (...)`.
    /// Returns a map from book ID to its relations bundle. Books with no matches
    /// for a given relation get an empty vec.
    #[allow(clippy::too_many_lines)]
    pub async fn batch_load_relations(
        pool: &SqlitePool,
        book_ids: &[Uuid],
        includes: &HashSet<&str>,
    ) -> Result<HashMap<Uuid, RelationsBundle>, DbError> {
        if book_ids.is_empty() || includes.is_empty() {
            return Ok(HashMap::new());
        }

        let id_strings: Vec<String> = book_ids.iter().map(ToString::to_string).collect();

        // Initialize bundles for every book ID.
        let mut bundles: HashMap<Uuid, RelationsBundle> = book_ids
            .iter()
            .map(|id| (*id, RelationsBundle::default()))
            .collect();

        if includes.contains("authors") {
            let placeholders = vec!["?"; id_strings.len()].join(", ");
            let sql = format!(
                "SELECT a.id, a.name, a.sort_name, ba.role, ba.position, ba.book_id \
                 FROM authors a \
                 JOIN book_authors ba ON ba.author_id = a.id \
                 WHERE ba.book_id IN ({placeholders}) \
                 ORDER BY ba.position"
            );
            let mut query = sqlx::query_as::<_, BatchAuthorRow>(&sql);
            for id in &id_strings {
                query = query.bind(id);
            }
            let rows = query
                .fetch_all(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

            for row in rows {
                let book_id = Uuid::parse_str(&row.book_id)
                    .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
                let entry = BookAuthorEntry {
                    author: Author {
                        id: Uuid::parse_str(&row.id)
                            .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?,
                        name: row.name,
                        sort_name: row.sort_name,
                    },
                    role: row.role,
                    position: row.position,
                };
                if let Some(bundle) = bundles.get_mut(&book_id) {
                    bundle.authors.get_or_insert_with(Vec::new).push(entry);
                }
            }
            // Ensure books with no authors get an empty vec.
            for bundle in bundles.values_mut() {
                if bundle.authors.is_none() {
                    bundle.authors = Some(Vec::new());
                }
            }
        }

        if includes.contains("series") {
            let placeholders = vec!["?"; id_strings.len()].join(", ");
            let sql = format!(
                "SELECT s.id, s.name, s.description, bs.position, bs.book_id \
                 FROM series s \
                 JOIN book_series bs ON bs.series_id = s.id \
                 WHERE bs.book_id IN ({placeholders}) \
                 ORDER BY bs.position"
            );
            let mut query = sqlx::query_as::<_, BatchSeriesRow>(&sql);
            for id in &id_strings {
                query = query.bind(id);
            }
            let rows = query
                .fetch_all(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

            for row in rows {
                let book_id = Uuid::parse_str(&row.book_id)
                    .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
                let entry = BookSeriesEntry {
                    series: Series {
                        id: Uuid::parse_str(&row.id)
                            .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?,
                        name: row.name,
                        description: row.description,
                    },
                    position: row.position,
                };
                if let Some(bundle) = bundles.get_mut(&book_id) {
                    bundle.series.get_or_insert_with(Vec::new).push(entry);
                }
            }
            for bundle in bundles.values_mut() {
                if bundle.series.is_none() {
                    bundle.series = Some(Vec::new());
                }
            }
        }

        if includes.contains("tags") {
            let placeholders = vec!["?"; id_strings.len()].join(", ");
            let sql = format!(
                "SELECT t.id, t.name, t.category, bt.book_id \
                 FROM tags t \
                 JOIN book_tags bt ON bt.tag_id = t.id \
                 WHERE bt.book_id IN ({placeholders})"
            );
            let mut query = sqlx::query_as::<_, BatchTagRow>(&sql);
            for id in &id_strings {
                query = query.bind(id);
            }
            let rows = query
                .fetch_all(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

            for row in rows {
                let book_id = Uuid::parse_str(&row.book_id)
                    .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
                let tag = Tag {
                    id: Uuid::parse_str(&row.id)
                        .map_err(|e| DbError::Query(format!("invalid tag UUID: {e}")))?,
                    name: row.name,
                    category: row.category,
                };
                if let Some(bundle) = bundles.get_mut(&book_id) {
                    bundle.tags.get_or_insert_with(Vec::new).push(tag);
                }
            }
            for bundle in bundles.values_mut() {
                if bundle.tags.is_none() {
                    bundle.tags = Some(Vec::new());
                }
            }
        }

        if includes.contains("files") {
            let placeholders = vec!["?"; id_strings.len()].join(", ");
            let sql = format!(
                "SELECT id, book_id, format, format_version, storage_path, file_size, hash, added_at \
                 FROM book_files WHERE book_id IN ({placeholders})"
            );
            let mut query = sqlx::query_as::<_, BookFileRow>(&sql);
            for id in &id_strings {
                query = query.bind(id);
            }
            let rows = query
                .fetch_all(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

            for row in rows {
                let book_id_str = row.book_id.clone();
                let book_id = Uuid::parse_str(&book_id_str)
                    .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
                let file = row.into_book_file()?;
                if let Some(bundle) = bundles.get_mut(&book_id) {
                    bundle.files.get_or_insert_with(Vec::new).push(file);
                }
            }
            for bundle in bundles.values_mut() {
                if bundle.files.is_none() {
                    bundle.files = Some(Vec::new());
                }
            }
        }

        Ok(bundles)
    }

    pub async fn update(pool: &SqlitePool, book: &Book) -> Result<(), DbError> {
        let id = book.id.to_string();
        let publisher_id = book.publisher_id.map(|p| p.to_string());
        let pub_year = book.publication_year.map(i64::from);
        let updated_at = Utc::now().to_rfc3339();
        let status = serialize_metadata_status(book.metadata_status);
        let review_baseline = book
            .review_baseline_metadata_status
            .map(serialize_metadata_status);
        let review_baseline_outcome =
            serialize_resolution_outcome(book.review_baseline_resolution_outcome);
        let resolution_state = serialize_resolution_state(book.resolution_state);
        let resolution_outcome = serialize_resolution_outcome(book.resolution_outcome);
        let resolution_requested_at = book.resolution_requested_at.to_rfc3339();
        let last_resolved_at = book.last_resolved_at.map(|value| value.to_rfc3339());
        let last_resolution_run_id = book.last_resolution_run_id.map(|value| value.to_string());
        let metadata_locked = i64::from(book.metadata_locked);
        let metadata_user_trusted = i64::from(book.metadata_user_trusted);
        let metadata_provenance = serialize_json(&book.metadata_provenance, "metadata provenance")?;
        let norm_title = archivis_core::models::normalize_title(&book.title);

        let result = sqlx::query!(
            "UPDATE books SET title = ?, subtitle = ?, sort_title = ?, description = ?, language = ?, publication_year = ?, publisher_id = ?, updated_at = ?, rating = ?, page_count = ?, metadata_status = ?, review_baseline_metadata_status = ?, review_baseline_resolution_outcome = ?, ingest_quality_score = ?, resolution_state = ?, resolution_outcome = ?, resolution_requested_at = ?, resolution_requested_reason = ?, last_resolved_at = ?, last_resolution_run_id = ?, metadata_locked = ?, metadata_user_trusted = ?, metadata_provenance = ?, cover_path = ?, norm_title = ? WHERE id = ?",
            book.title,
            book.subtitle,
            book.sort_title,
            book.description,
            book.language,
            pub_year,
            publisher_id,
            updated_at,
            book.rating,
            book.page_count,
            status,
            review_baseline,
            review_baseline_outcome,
            book.ingest_quality_score,
            resolution_state,
            resolution_outcome,
            resolution_requested_at,
            book.resolution_requested_reason,
            last_resolved_at,
            last_resolution_run_id,
            metadata_locked,
            metadata_user_trusted,
            metadata_provenance,
            book.cover_path,
            norm_title,
            id,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "book", id });
        }

        Ok(())
    }

    pub async fn update_conn(conn: &mut SqliteConnection, book: &Book) -> Result<(), DbError> {
        let id = book.id.to_string();
        let publisher_id = book.publisher_id.map(|p| p.to_string());
        let pub_year = book.publication_year.map(i64::from);
        let updated_at = Utc::now().to_rfc3339();
        let status = serialize_metadata_status(book.metadata_status);
        let review_baseline = book
            .review_baseline_metadata_status
            .map(serialize_metadata_status);
        let review_baseline_outcome =
            serialize_resolution_outcome(book.review_baseline_resolution_outcome);
        let resolution_state = serialize_resolution_state(book.resolution_state);
        let resolution_outcome = serialize_resolution_outcome(book.resolution_outcome);
        let resolution_requested_at = book.resolution_requested_at.to_rfc3339();
        let last_resolved_at = book.last_resolved_at.map(|value| value.to_rfc3339());
        let last_resolution_run_id = book.last_resolution_run_id.map(|value| value.to_string());
        let metadata_locked = i64::from(book.metadata_locked);
        let metadata_user_trusted = i64::from(book.metadata_user_trusted);
        let metadata_provenance = serialize_json(&book.metadata_provenance, "metadata provenance")?;
        let norm_title = archivis_core::models::normalize_title(&book.title);

        let result = sqlx::query!(
            "UPDATE books SET title = ?, subtitle = ?, sort_title = ?, description = ?, language = ?, publication_year = ?, publisher_id = ?, updated_at = ?, rating = ?, page_count = ?, metadata_status = ?, review_baseline_metadata_status = ?, review_baseline_resolution_outcome = ?, ingest_quality_score = ?, resolution_state = ?, resolution_outcome = ?, resolution_requested_at = ?, resolution_requested_reason = ?, last_resolved_at = ?, last_resolution_run_id = ?, metadata_locked = ?, metadata_user_trusted = ?, metadata_provenance = ?, cover_path = ?, norm_title = ? WHERE id = ?",
            book.title,
            book.subtitle,
            book.sort_title,
            book.description,
            book.language,
            pub_year,
            publisher_id,
            updated_at,
            book.rating,
            book.page_count,
            status,
            review_baseline,
            review_baseline_outcome,
            book.ingest_quality_score,
            resolution_state,
            resolution_outcome,
            resolution_requested_at,
            book.resolution_requested_reason,
            last_resolved_at,
            last_resolution_run_id,
            metadata_locked,
            metadata_user_trusted,
            metadata_provenance,
            book.cover_path,
            norm_title,
            id,
        )
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "book", id });
        }

        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM books WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Book>, DbError> {
        let filter = BookFilter {
            query: Some(query.into()),
            trusted: None,
            ..BookFilter::default()
        };
        // Default to relevance sort for search queries (the caller can still
        // override by setting `sort_by` to something other than the default).
        let default_sort = PaginationParams::default().sort_by;
        let explicit = (params.sort_by != default_sort).then(|| params.sort_by.clone());
        let mut params = params.clone();
        params.sort_by = PaginationParams::resolve_default_sort(explicit, true);
        Self::list(pool, &params, &filter).await
    }

    #[allow(clippy::too_many_lines)] // multiple sub-queries for related entities
    pub async fn get_with_relations(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<BookWithRelations, DbError> {
        let book = Self::get_by_id(pool, id).await?;
        let id_str = id.to_string();

        // Fetch authors
        let author_rows = sqlx::query_as!(
            BookAuthorRow,
            "SELECT a.id, a.name, a.sort_name, ba.role, ba.position FROM authors a JOIN book_authors ba ON ba.author_id = a.id WHERE ba.book_id = ? ORDER BY ba.position",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let authors = author_rows
            .into_iter()
            .map(|r| {
                Ok(BookAuthorEntry {
                    author: Author {
                        id: Uuid::parse_str(&r.id)
                            .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?,
                        name: r.name,
                        sort_name: r.sort_name,
                    },
                    role: r.role,
                    position: r.position,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        // Fetch series
        let series_rows = sqlx::query_as!(
            BookSeriesRow,
            "SELECT s.id, s.name, s.description, bs.position FROM series s JOIN book_series bs ON bs.series_id = s.id WHERE bs.book_id = ? ORDER BY bs.position",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let series = series_rows
            .into_iter()
            .map(|r| {
                Ok(BookSeriesEntry {
                    series: Series {
                        id: Uuid::parse_str(&r.id)
                            .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?,
                        name: r.name,
                        description: r.description,
                    },
                    position: r.position,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        // Fetch files
        let file_rows = sqlx::query_as!(
            BookFileRow,
            "SELECT id, book_id, format, format_version, storage_path, file_size, hash, added_at FROM book_files WHERE book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let files = file_rows
            .into_iter()
            .map(BookFileRow::into_book_file)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch identifiers
        let ident_rows = sqlx::query_as!(
            IdentifierRow,
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let identifiers = ident_rows
            .into_iter()
            .map(IdentifierRow::into_identifier)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch tags
        let tag_rows = sqlx::query_as!(
            TagRow,
            "SELECT t.id, t.name, t.category FROM tags t JOIN book_tags bt ON bt.tag_id = t.id WHERE bt.book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let tags = tag_rows
            .into_iter()
            .map(TagRow::into_tag)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch publisher name
        let publisher_name: Option<String> = if let Some(pid) = book.publisher_id {
            let pid_str = pid.to_string();
            sqlx::query_scalar!("SELECT name FROM publishers WHERE id = ?", pid_str)
                .fetch_optional(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?
        } else {
            None
        };

        Ok(BookWithRelations {
            book,
            authors,
            series,
            files,
            identifiers,
            tags,
            publisher_name,
        })
    }

    #[allow(clippy::too_many_lines)] // mirrors `get_with_relations` but uses a connection
    pub async fn get_with_relations_conn(
        conn: &mut SqliteConnection,
        id: Uuid,
    ) -> Result<BookWithRelations, DbError> {
        let book = Self::get_by_id_conn(&mut *conn, id).await?;
        let id_str = id.to_string();

        let author_rows = sqlx::query_as!(
            BookAuthorRow,
            "SELECT a.id, a.name, a.sort_name, ba.role, ba.position FROM authors a JOIN book_authors ba ON ba.author_id = a.id WHERE ba.book_id = ? ORDER BY ba.position",
            id_str,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let authors = author_rows
            .into_iter()
            .map(|r| {
                Ok(BookAuthorEntry {
                    author: Author {
                        id: Uuid::parse_str(&r.id)
                            .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?,
                        name: r.name,
                        sort_name: r.sort_name,
                    },
                    role: r.role,
                    position: r.position,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        let series_rows = sqlx::query_as!(
            BookSeriesRow,
            "SELECT s.id, s.name, s.description, bs.position FROM series s JOIN book_series bs ON bs.series_id = s.id WHERE bs.book_id = ? ORDER BY bs.position",
            id_str,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let series = series_rows
            .into_iter()
            .map(|r| {
                Ok(BookSeriesEntry {
                    series: Series {
                        id: Uuid::parse_str(&r.id)
                            .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?,
                        name: r.name,
                        description: r.description,
                    },
                    position: r.position,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        let file_rows = sqlx::query_as!(
            BookFileRow,
            "SELECT id, book_id, format, format_version, storage_path, file_size, hash, added_at FROM book_files WHERE book_id = ?",
            id_str,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let files = file_rows
            .into_iter()
            .map(BookFileRow::into_book_file)
            .collect::<Result<Vec<_>, _>>()?;

        let ident_rows = sqlx::query_as!(
            IdentifierRow,
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE book_id = ?",
            id_str,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let identifiers = ident_rows
            .into_iter()
            .map(IdentifierRow::into_identifier)
            .collect::<Result<Vec<_>, _>>()?;

        let tag_rows = sqlx::query_as!(
            TagRow,
            "SELECT t.id, t.name, t.category FROM tags t JOIN book_tags bt ON bt.tag_id = t.id WHERE bt.book_id = ?",
            id_str,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let tags = tag_rows
            .into_iter()
            .map(TagRow::into_tag)
            .collect::<Result<Vec<_>, _>>()?;

        let publisher_name: Option<String> = if let Some(pid) = book.publisher_id {
            let pid_str = pid.to_string();
            sqlx::query_scalar!("SELECT name FROM publishers WHERE id = ?", pid_str)
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?
        } else {
            None
        };

        Ok(BookWithRelations {
            book,
            authors,
            series,
            files,
            identifiers,
            tags,
            publisher_name,
        })
    }

    /// Remove all author links for a book.
    pub async fn clear_authors(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_authors WHERE book_id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    pub async fn clear_authors_conn(
        conn: &mut SqliteConnection,
        book_id: Uuid,
    ) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_authors WHERE book_id = ?", id_str)
            .execute(conn)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Remove all series links for a book.
    pub async fn clear_series(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_series WHERE book_id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    pub async fn clear_series_conn(
        conn: &mut SqliteConnection,
        book_id: Uuid,
    ) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_series WHERE book_id = ?", id_str)
            .execute(conn)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Remove all tag links for a book.
    pub async fn clear_tags(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_tags WHERE book_id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Link a book to an author.
    pub async fn add_author(
        pool: &SqlitePool,
        book_id: Uuid,
        author_id: Uuid,
        role: &str,
        position: i32,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let author_id_str = author_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position) VALUES (?, ?, ?, ?)",
            book_id_str,
            author_id_str,
            role,
            position,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn add_author_conn(
        conn: &mut SqliteConnection,
        book_id: Uuid,
        author_id: Uuid,
        role: &str,
        position: i32,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let author_id_str = author_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position) VALUES (?, ?, ?, ?)",
            book_id_str,
            author_id_str,
            role,
            position,
        )
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Link a book to a series.
    pub async fn add_series(
        pool: &SqlitePool,
        book_id: Uuid,
        series_id: Uuid,
        position: Option<f64>,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let series_id_str = series_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_series (book_id, series_id, position) VALUES (?, ?, ?)",
            book_id_str,
            series_id_str,
            position,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn add_series_conn(
        conn: &mut SqliteConnection,
        book_id: Uuid,
        series_id: Uuid,
        position: Option<f64>,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let series_id_str = series_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_series (book_id, series_id, position) VALUES (?, ?, ?)",
            book_id_str,
            series_id_str,
            position,
        )
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Update the position of a book within a series.
    pub async fn update_series_position(
        pool: &SqlitePool,
        book_id: Uuid,
        series_id: Uuid,
        position: Option<f64>,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let series_id_str = series_id.to_string();
        sqlx::query!(
            "UPDATE book_series SET position = ? WHERE book_id = ? AND series_id = ?",
            position,
            book_id_str,
            series_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn update_series_position_conn(
        conn: &mut SqliteConnection,
        book_id: Uuid,
        series_id: Uuid,
        position: Option<f64>,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let series_id_str = series_id.to_string();
        sqlx::query!(
            "UPDATE book_series SET position = ? WHERE book_id = ? AND series_id = ?",
            position,
            book_id_str,
            series_id_str,
        )
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Link a book to a tag.
    pub async fn add_tag(pool: &SqlitePool, book_id: Uuid, tag_id: Uuid) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let tag_id_str = tag_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?, ?)",
            book_id_str,
            tag_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Find potential duplicate books by title similarity.
    ///
    /// Returns books whose `norm_title` starts with the same 3-char prefix,
    /// enabling efficient DB-level pre-filtering before expensive fuzzy
    /// matching in Rust.
    pub async fn find_potential_duplicates(
        pool: &SqlitePool,
        title: &str,
        limit: i64,
    ) -> Result<Vec<BookWithAuthors>, DbError> {
        let norm = archivis_core::models::normalize_title(title);
        let prefix: String = norm.chars().take(3).collect();

        if prefix.len() < 3 {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as!(
            DuplicateCandidateRow,
            r#"SELECT b.id, b.title, b.subtitle, b.sort_title, b.description, b.language,
                      b.publication_year, b.publisher_id, b.added_at, b.updated_at,
                      b.rating, b.page_count, b.metadata_status,
                      b.review_baseline_metadata_status, b.review_baseline_resolution_outcome, b.ingest_quality_score,
                      b.metadata_quality_score,
                      b.resolution_state, b.resolution_outcome, b.resolution_requested_at,
                      b.resolution_requested_reason, b.last_resolved_at, b.last_resolution_run_id,
                      b.metadata_locked, b.metadata_user_trusted, b.metadata_provenance,
                      b.cover_path,
                      GROUP_CONCAT(a.name, '||') as "author_names: String"
               FROM books b
               LEFT JOIN book_authors ba ON ba.book_id = b.id
               LEFT JOIN authors a ON a.id = ba.author_id
               WHERE SUBSTR(b.norm_title, 1, 3) = ?
               GROUP BY b.id
               LIMIT ?"#,
            prefix,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(|r| {
                let book = BookRow {
                    id: r.id,
                    title: r.title,
                    subtitle: r.subtitle,
                    sort_title: r.sort_title,
                    description: r.description,
                    language: r.language,
                    publication_year: r.publication_year,
                    publisher_id: r.publisher_id,
                    added_at: r.added_at,
                    updated_at: r.updated_at,
                    rating: r.rating,
                    page_count: r.page_count,
                    metadata_status: r.metadata_status,
                    review_baseline_metadata_status: r.review_baseline_metadata_status,
                    review_baseline_resolution_outcome: r.review_baseline_resolution_outcome,
                    ingest_quality_score: r.ingest_quality_score,
                    metadata_quality_score: r.metadata_quality_score,
                    resolution_state: r.resolution_state,
                    resolution_outcome: r.resolution_outcome,
                    resolution_requested_at: r.resolution_requested_at,
                    resolution_requested_reason: r.resolution_requested_reason,
                    last_resolved_at: r.last_resolved_at,
                    last_resolution_run_id: r.last_resolution_run_id,
                    metadata_locked: r.metadata_locked,
                    metadata_user_trusted: r.metadata_user_trusted,
                    metadata_provenance: r.metadata_provenance,
                    cover_path: r.cover_path,
                }
                .into_book()?;

                let author_names = r
                    .author_names
                    .map(|names| {
                        names
                            .split("||")
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();

                Ok(BookWithAuthors { book, author_names })
            })
            .collect()
    }

    /// Backfill `norm_title` for rows that still have the empty default.
    ///
    /// Called after migrations; idempotent (no-op when all rows are filled).
    pub async fn backfill_norm_titles(pool: &SqlitePool) -> Result<(), DbError> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, title FROM books WHERE norm_title = ''")
                .fetch_all(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

        if rows.is_empty() {
            return Ok(());
        }

        tracing::info!(count = rows.len(), "backfilling norm_title");

        for (id, title) in &rows {
            let norm = archivis_core::models::normalize_title(title);
            sqlx::query("UPDATE books SET norm_title = ? WHERE id = ?")
                .bind(&norm)
                .bind(id)
                .execute(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;
        }

        Ok(())
    }

    /// Targeted setter for review baseline fields to avoid
    /// full-row update races with concurrent resolution runs.
    pub async fn set_review_baseline(
        pool: &SqlitePool,
        book_id: Uuid,
        baseline: Option<MetadataStatus>,
        outcome: Option<ResolutionOutcome>,
    ) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        let baseline_str = baseline.map(serialize_metadata_status);
        let outcome_str = serialize_resolution_outcome(outcome);
        let updated_at = Utc::now().to_rfc3339();

        let result = sqlx::query!(
            "UPDATE books SET review_baseline_metadata_status = ?, review_baseline_resolution_outcome = ?, updated_at = ? WHERE id = ?",
            baseline_str,
            outcome_str,
            updated_at,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Lightweight single-column update for the live quality score.
    ///
    /// Intentionally does NOT bump `updated_at` — that column reflects
    /// user-visible mutations, and a backfill/refresh would falsely make
    /// books appear recently edited.
    pub async fn update_metadata_quality_score(
        pool: &SqlitePool,
        book_id: Uuid,
        score: f32,
    ) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        let score_f64 = f64::from(score);
        let result = sqlx::query!(
            "UPDATE books SET metadata_quality_score = ? WHERE id = ?",
            score_f64,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Return book IDs that have no computed `metadata_quality_score` yet.
    pub async fn list_ids_without_quality_score(
        pool: &SqlitePool,
        limit: i64,
    ) -> Result<Vec<Uuid>, DbError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM books WHERE metadata_quality_score IS NULL LIMIT ?")
                .bind(limit)
                .fetch_all(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(|(id,)| {
                Uuid::parse_str(&id).map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))
            })
            .collect()
    }
}

// ── Row types for sqlx mapping ──────────────────────────────────

/// Row type for the `find_potential_duplicates` query (book + grouped author names).
#[derive(sqlx::FromRow)]
struct DuplicateCandidateRow {
    id: String,
    title: String,
    subtitle: Option<String>,
    sort_title: String,
    description: Option<String>,
    language: Option<String>,
    publication_year: Option<i64>,
    publisher_id: Option<String>,
    added_at: String,
    updated_at: String,
    rating: Option<f64>,
    page_count: Option<i64>,
    metadata_status: String,
    review_baseline_metadata_status: Option<String>,
    review_baseline_resolution_outcome: Option<String>,
    ingest_quality_score: f64,
    metadata_quality_score: Option<f64>,
    resolution_state: String,
    resolution_outcome: Option<String>,
    resolution_requested_at: String,
    resolution_requested_reason: Option<String>,
    last_resolved_at: Option<String>,
    last_resolution_run_id: Option<String>,
    metadata_locked: i64,
    metadata_user_trusted: i64,
    metadata_provenance: String,
    cover_path: Option<String>,
    author_names: Option<String>,
}

#[derive(sqlx::FromRow)]
struct BookRow {
    id: String,
    title: String,
    subtitle: Option<String>,
    sort_title: String,
    description: Option<String>,
    language: Option<String>,
    publication_year: Option<i64>,
    publisher_id: Option<String>,
    added_at: String,
    updated_at: String,
    rating: Option<f64>,
    page_count: Option<i64>,
    metadata_status: String,
    review_baseline_metadata_status: Option<String>,
    review_baseline_resolution_outcome: Option<String>,
    ingest_quality_score: f64,
    metadata_quality_score: Option<f64>,
    resolution_state: String,
    resolution_outcome: Option<String>,
    resolution_requested_at: String,
    resolution_requested_reason: Option<String>,
    last_resolved_at: Option<String>,
    last_resolution_run_id: Option<String>,
    metadata_locked: i64,
    metadata_user_trusted: i64,
    metadata_provenance: String,
    cover_path: Option<String>,
}

impl BookRow {
    fn into_book(self) -> Result<Book, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let publisher_id = self
            .publisher_id
            .map(|p| Uuid::parse_str(&p))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid publisher UUID: {e}")))?;
        #[allow(clippy::cast_possible_truncation)]
        let publication_year = self.publication_year.map(|y| y as i32);
        let added_at = parse_timestamp(&self.added_at, "added_at")?;
        let updated_at = parse_timestamp(&self.updated_at, "updated_at")?;
        let metadata_status: MetadataStatus = self
            .metadata_status
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let review_baseline_metadata_status = self
            .review_baseline_metadata_status
            .map(|value| value.parse::<MetadataStatus>())
            .transpose()
            .map_err(|e: String| DbError::Query(e))?;
        let review_baseline_resolution_outcome = self
            .review_baseline_resolution_outcome
            .map(|value| value.parse::<ResolutionOutcome>())
            .transpose()
            .map_err(|e: String| DbError::Query(e))?;
        let resolution_state: ResolutionState = self
            .resolution_state
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let resolution_outcome = self
            .resolution_outcome
            .map(|value| value.parse())
            .transpose()
            .map_err(|e: String| DbError::Query(e))?;
        let resolution_requested_at =
            parse_timestamp(&self.resolution_requested_at, "resolution_requested_at")?;
        let last_resolved_at = self
            .last_resolved_at
            .as_deref()
            .map(|value| parse_timestamp(value, "last_resolved_at"))
            .transpose()?;
        let last_resolution_run_id = self
            .last_resolution_run_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid resolution run UUID: {e}")))?;
        let metadata_provenance: MetadataProvenance =
            serde_json::from_str(&self.metadata_provenance)
                .map_err(|e| DbError::Query(format!("invalid metadata_provenance JSON: {e}")))?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(Book {
            id,
            title: self.title,
            subtitle: self.subtitle,
            sort_title: self.sort_title,
            description: self.description,
            language: self.language,
            publication_year,
            publisher_id,
            added_at,
            updated_at,
            rating: self.rating.map(|r| r as f32),
            page_count: self.page_count.map(|p| p as i32),
            metadata_status,
            review_baseline_metadata_status,
            review_baseline_resolution_outcome,
            ingest_quality_score: self.ingest_quality_score as f32,
            metadata_quality_score: self.metadata_quality_score.map(|s| s as f32),
            resolution_state,
            resolution_outcome,
            resolution_requested_at,
            resolution_requested_reason: self.resolution_requested_reason,
            last_resolved_at,
            last_resolution_run_id,
            metadata_locked: self.metadata_locked != 0,
            metadata_user_trusted: self.metadata_user_trusted != 0,
            metadata_provenance,
            cover_path: self.cover_path,
        })
    }
}

#[derive(sqlx::FromRow)]
struct BookAuthorRow {
    id: String,
    name: String,
    sort_name: String,
    role: String,
    position: i64,
}

#[derive(sqlx::FromRow)]
struct BookSeriesRow {
    id: String,
    name: String,
    description: Option<String>,
    position: Option<f64>,
}

#[derive(sqlx::FromRow)]
pub struct BookFileRow {
    pub id: String,
    pub book_id: String,
    pub format: String,
    pub format_version: Option<String>,
    pub storage_path: String,
    pub file_size: i64,
    pub hash: String,
    pub added_at: String,
}

impl BookFileRow {
    pub fn into_book_file(self) -> Result<BookFile, DbError> {
        use archivis_core::models::BookFormat;

        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid book_file UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let format: BookFormat = self.format.parse().map_err(|e: String| DbError::Query(e))?;
        let added_at = DateTime::parse_from_rfc3339(&self.added_at)
            .map(|d| d.with_timezone(&Utc))
            .or_else(|_| {
                // Handle SQLite default timestamp format (with microseconds)
                chrono::NaiveDateTime::parse_from_str(&self.added_at, "%Y-%m-%dT%H:%M:%S%.fZ")
                    .map(|ndt| ndt.and_utc())
            })
            .map_err(|e| DbError::Query(format!("invalid added_at: {e}")))?;

        Ok(BookFile {
            id,
            book_id,
            format,
            format_version: self.format_version,
            storage_path: self.storage_path,
            file_size: self.file_size,
            hash: self.hash,
            added_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub struct IdentifierRow {
    pub id: String,
    pub book_id: String,
    pub identifier_type: String,
    pub value: String,
    pub source_type: String,
    pub source_name: Option<String>,
    pub confidence: f64,
}

impl IdentifierRow {
    pub fn into_identifier(self) -> Result<Identifier, DbError> {
        use archivis_core::models::{IdentifierType, MetadataSource};

        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid identifier UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let identifier_type: IdentifierType = self
            .identifier_type
            .parse()
            .map_err(|e: String| DbError::Query(e))?;

        let source = match self.source_type.as_str() {
            "embedded" => MetadataSource::Embedded,
            "filename" => MetadataSource::Filename,
            "user" => MetadataSource::User,
            "provider" => MetadataSource::Provider(self.source_name.unwrap_or_default()),
            "content_scan" => MetadataSource::ContentScan,
            other => {
                return Err(DbError::Query(format!("unknown source_type: {other}")));
            }
        };

        #[allow(clippy::cast_possible_truncation)]
        Ok(Identifier {
            id,
            book_id,
            identifier_type,
            value: self.value,
            source,
            confidence: self.confidence as f32,
        })
    }
}

#[derive(sqlx::FromRow)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
}

impl TagRow {
    pub fn into_tag(self) -> Result<Tag, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid tag UUID: {e}")))?;
        Ok(Tag {
            id,
            name: self.name,
            category: self.category,
        })
    }
}

// ── Batch row types for `batch_load_relations` ───────────────

#[derive(sqlx::FromRow)]
struct BatchAuthorRow {
    id: String,
    name: String,
    sort_name: String,
    role: String,
    position: i64,
    book_id: String,
}

#[derive(sqlx::FromRow)]
struct BatchSeriesRow {
    id: String,
    name: String,
    description: Option<String>,
    position: Option<f64>,
    book_id: String,
}

#[derive(sqlx::FromRow)]
struct BatchTagRow {
    id: String,
    name: String,
    category: Option<String>,
    book_id: String,
}

impl BookRepository {
    pub async fn mark_resolution_pending(
        pool: &SqlitePool,
        book_id: Uuid,
        trigger: &str,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET updated_at = ?,
                 resolution_requested_at = ?,
                 resolution_requested_reason = ?,
                 resolution_state = CASE
                     WHEN resolution_state = ? THEN resolution_state
                     ELSE ?
                 END
             WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(trigger)
        .bind(&running)
        .bind(&pending)
        .bind(&id_str)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Normalize `resolution_state` for a trusted book after a core-identity edit.
    ///
    /// - If the book is `Running` (manual refresh in flight): stamp
    ///   `resolution_requested_at` so the resolver self-supersedes, keep
    ///   state as `Running`.  The auto-resolver will later skip (trusted) → Done.
    /// - Otherwise: set `resolution_state = Done` directly.  A trusted book
    ///   should not re-enter the automatic resolution pipeline.
    ///
    /// Does NOT touch `last_resolved_at` (no resolution actually ran).
    pub async fn normalize_trusted_resolution_state(
        pool: &SqlitePool,
        book_id: Uuid,
        trigger: &str,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let running = serialize_resolution_state(ResolutionState::Running);
        let done = serialize_resolution_state(ResolutionState::Done);

        sqlx::query(
            "UPDATE books
             SET resolution_state = CASE
                     WHEN resolution_state = ? THEN resolution_state
                     ELSE ?
                 END,
                 resolution_requested_at = CASE
                     WHEN resolution_state = ? THEN ?
                     ELSE resolution_requested_at
                 END,
                 resolution_requested_reason = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(&running) // CASE 1: keep Running
        .bind(&done) // CASE 1: else → Done
        .bind(&running) // CASE 2: stamp only if Running
        .bind(&now) // CASE 2: new timestamp
        .bind(trigger)
        .bind(&now)
        .bind(book_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn list_pending_resolution(
        pool: &SqlitePool,
        limit: i64,
    ) -> Result<Vec<Book>, DbError> {
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let rows = sqlx::query_as::<_, BookRow>(
            "SELECT id, title, subtitle, sort_title, description, language, publication_year, publisher_id, added_at, updated_at, rating, page_count, metadata_status, review_baseline_metadata_status, review_baseline_resolution_outcome, ingest_quality_score, metadata_quality_score, resolution_state, resolution_outcome, resolution_requested_at, resolution_requested_reason, last_resolved_at, last_resolution_run_id, metadata_locked, metadata_user_trusted, metadata_provenance, cover_path
             FROM books
             WHERE resolution_state = ?
               AND (metadata_locked = 0 OR metadata_user_trusted = 1)
             ORDER BY resolution_requested_at ASC
             LIMIT ?",
        )
        .bind(pending)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(BookRow::into_book).collect()
    }

    pub async fn claim_pending_resolution(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<bool, DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 updated_at = ?
             WHERE id = ?
               AND resolution_state = ?
               AND (metadata_locked = 0 OR metadata_user_trusted = 1)",
        )
        .bind(&running)
        .bind(&now)
        .bind(&id_str)
        .bind(&pending)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn claim_manual_resolution(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<bool, DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 updated_at = ?
             WHERE id = ?
               AND resolution_state = ?",
        )
        .bind(&running)
        .bind(&now)
        .bind(&id_str)
        .bind(&pending)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_running_resolution(pool: &SqlitePool) -> Result<Vec<Book>, DbError> {
        let running = serialize_resolution_state(ResolutionState::Running);
        let rows = sqlx::query_as::<_, BookRow>(
            "SELECT id, title, subtitle, sort_title, description, language, publication_year, publisher_id, added_at, updated_at, rating, page_count, metadata_status, review_baseline_metadata_status, review_baseline_resolution_outcome, ingest_quality_score, metadata_quality_score, resolution_state, resolution_outcome, resolution_requested_at, resolution_requested_reason, last_resolved_at, last_resolution_run_id, metadata_locked, metadata_user_trusted, metadata_provenance, cover_path
             FROM books
             WHERE resolution_state = ?",
        )
        .bind(running)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(BookRow::into_book).collect()
    }

    pub async fn reset_resolution_to_pending(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<bool, DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 updated_at = ?
             WHERE id = ?
               AND resolution_state = ?",
        )
        .bind(&pending)
        .bind(&now)
        .bind(&id_str)
        .bind(&running)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_resolution_done(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let done = serialize_resolution_state(ResolutionState::Done);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 updated_at = ?,
                 last_resolved_at = ?
             WHERE id = ?",
        )
        .bind(&done)
        .bind(&now)
        .bind(&now)
        .bind(&id_str)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn mark_resolution_superseded(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 updated_at = ?
             WHERE id = ?
               AND resolution_state = ?",
        )
        .bind(&pending)
        .bind(&now)
        .bind(&id_str)
        .bind(&running)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn mark_resolution_failed(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let failed = serialize_resolution_state(ResolutionState::Failed);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(&failed)
        .bind(&now)
        .bind(&id_str)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Mark a book's resolution as skipped (trusted publisher or other rule).
    ///
    /// Sets `resolution_state = 'done'` and `last_resolved_at = now`, records
    /// the skip reason, but does **not** touch `resolution_outcome` or
    /// `metadata_status` (preserving the import-time values).
    pub async fn mark_resolution_skipped(
        pool: &SqlitePool,
        book_id: Uuid,
        reason: &str,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let done = serialize_resolution_state(ResolutionState::Done);

        let result = sqlx::query(
            "UPDATE books
             SET resolution_state = ?,
                 last_resolved_at = ?,
                 resolution_requested_reason = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(&done)
        .bind(&now)
        .bind(reason)
        .bind(&now)
        .bind(&id_str)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Atomically set `metadata_user_trusted = true` and transition to
    /// Identified/Confirmed/Done, clearing review baselines.
    /// Returns `Ok(true)` if the row was updated, `Ok(false)` if the book
    /// is currently Running (0 rows affected).
    pub async fn set_trusted_atomic(
        conn: &mut SqliteConnection,
        book_id: Uuid,
    ) -> Result<bool, DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let identified = serialize_metadata_status(MetadataStatus::Identified);
        let confirmed =
            serialize_resolution_outcome(Some(archivis_core::models::ResolutionOutcome::Confirmed));
        let done = serialize_resolution_state(ResolutionState::Done);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET metadata_user_trusted = 1,
                 metadata_status = ?,
                 resolution_outcome = ?,
                 resolution_state = ?,
                 review_baseline_metadata_status = NULL,
                 review_baseline_resolution_outcome = NULL,
                 updated_at = ?
             WHERE id = ? AND resolution_state != ?",
        )
        .bind(&identified)
        .bind(&confirmed)
        .bind(&done)
        .bind(&now)
        .bind(&id_str)
        .bind(&running)
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Atomically set `metadata_user_trusted = false`, clear
    /// `resolution_outcome` to NULL (re-evaluate, not undo), clear review
    /// baselines, and cancel any pending resolution (Pending → Done).
    /// Returns `Ok(true)` if the row was updated, `Ok(false)` if the book
    /// is currently Running (0 rows affected).
    pub async fn set_untrusted_atomic(
        conn: &mut SqliteConnection,
        book_id: Uuid,
    ) -> Result<bool, DbError> {
        let now = Utc::now().to_rfc3339();
        let id_str = book_id.to_string();
        let done = serialize_resolution_state(ResolutionState::Done);
        let pending = serialize_resolution_state(ResolutionState::Pending);
        let running = serialize_resolution_state(ResolutionState::Running);

        let result = sqlx::query(
            "UPDATE books
             SET metadata_user_trusted = 0,
                 resolution_outcome = NULL,
                 review_baseline_metadata_status = NULL,
                 review_baseline_resolution_outcome = NULL,
                 resolution_state = CASE
                     WHEN resolution_state = ? THEN ?
                     ELSE resolution_state
                 END,
                 updated_at = ?
             WHERE id = ? AND resolution_state != ?",
        )
        .bind(&pending)
        .bind(&done)
        .bind(&now)
        .bind(&id_str)
        .bind(&running)
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Pool-based version of [`set_untrusted_atomic`](Self::set_untrusted_atomic)
    /// for callers that don't need a transaction.
    pub async fn set_untrusted_atomic_pool(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<bool, DbError> {
        let mut conn = pool
            .acquire()
            .await
            .map_err(|e| DbError::Connection(e.to_string()))?;
        Self::set_untrusted_atomic(&mut conn, book_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::{compile_fts_match, escape_fts_term, escape_fts_text, BookFilter};

    // ── escape_fts_text ──────────────────────────────────────────

    #[test]
    fn escape_text_plain_terms() {
        assert_eq!(escape_fts_text("hello world"), "hello world*");
    }

    #[test]
    fn escape_text_preserves_quoted_phrase() {
        assert_eq!(escape_fts_text(r#""hello world""#), r#""hello world""#);
    }

    #[test]
    fn escape_text_passes_or_through() {
        assert_eq!(escape_fts_text("hello OR world"), "hello OR world*");
    }

    #[test]
    fn escape_text_passes_not_through() {
        assert_eq!(escape_fts_text("NOT dune"), "NOT dune");
    }

    #[test]
    fn escape_text_escapes_and() {
        assert_eq!(escape_fts_text("hello AND world"), r#"hello "AND" world*"#);
    }

    #[test]
    fn escape_text_escapes_near() {
        assert_eq!(
            escape_fts_text("hello NEAR world"),
            r#"hello "NEAR" world*"#
        );
    }

    #[test]
    fn escape_text_preserves_prefix_wildcard() {
        assert_eq!(escape_fts_text("hel*"), "hel*");
    }

    #[test]
    fn escape_text_empty_input() {
        assert_eq!(escape_fts_text(""), "");
        assert_eq!(escape_fts_text("   "), "");
    }

    #[test]
    fn escape_text_unclosed_quote() {
        assert_eq!(escape_fts_text(r#""unclosed"#), r#""unclosed""#);
    }

    #[test]
    fn escape_text_case_insensitive_and() {
        // `and` / `And` are still the AND operator in FTS5
        assert_eq!(escape_fts_text("and"), r#""and""#);
        assert_eq!(escape_fts_text("And"), r#""And""#);
    }

    #[test]
    fn escape_text_or_case_insensitive_passthrough() {
        assert_eq!(escape_fts_text("or"), "or*");
        assert_eq!(escape_fts_text("Or"), "Or*");
    }

    #[test]
    fn escape_text_mixed_input() {
        assert_eq!(
            escape_fts_text(r#"brandon "the final empire" OR sanderson"#),
            r#"brandon "the final empire" OR sanderson*"#,
        );
    }

    #[test]
    fn escape_text_not_with_or() {
        assert_eq!(
            escape_fts_text("dune OR NOT foundation"),
            "dune* OR NOT foundation"
        );
    }

    // ── escape_fts_term ──────────────────────────────────────────

    #[test]
    fn escape_term_simple() {
        assert_eq!(escape_fts_term("dune"), "dune*");
    }

    #[test]
    fn escape_term_with_spaces() {
        assert_eq!(escape_fts_term("dune messiah"), r#""dune messiah""#);
    }

    #[test]
    fn escape_term_empty() {
        assert_eq!(escape_fts_term(""), "");
        assert_eq!(escape_fts_term("   "), "");
    }

    #[test]
    fn escape_term_preserves_wildcard() {
        assert_eq!(escape_fts_term("hel*"), "hel*");
    }

    // ── compile_fts_match ────────────────────────────────────────

    #[test]
    fn match_text_only() {
        let filter = BookFilter {
            query: Some("dune".into()),
            ..Default::default()
        };
        assert_eq!(compile_fts_match(&filter), Some("dune*".into()));
    }

    #[test]
    fn match_column_filters_only() {
        let filter = BookFilter {
            fts_column_filters: vec![("title".into(), "dune".into(), false)],
            ..Default::default()
        };
        assert_eq!(compile_fts_match(&filter), Some("title : dune*".into()));
    }

    #[test]
    fn match_combined_text_and_column() {
        let filter = BookFilter {
            query: Some("herbert".into()),
            fts_column_filters: vec![("title".into(), "dune".into(), false)],
            ..Default::default()
        };
        assert_eq!(
            compile_fts_match(&filter),
            Some("herbert* title : dune*".into())
        );
    }

    #[test]
    fn match_negated_column_filter() {
        let filter = BookFilter {
            fts_column_filters: vec![("title".into(), "dune".into(), true)],
            ..Default::default()
        };
        assert_eq!(compile_fts_match(&filter), Some("NOT title : dune*".into()));
    }

    #[test]
    fn match_empty_filter() {
        let filter = BookFilter::default();
        assert_eq!(compile_fts_match(&filter), None);
    }

    #[test]
    fn match_blank_query_only() {
        let filter = BookFilter {
            query: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(compile_fts_match(&filter), None);
    }

    #[test]
    fn match_multiple_column_filters() {
        let filter = BookFilter {
            fts_column_filters: vec![
                ("title".into(), "dune".into(), false),
                ("description".into(), "spice".into(), true),
            ],
            ..Default::default()
        };
        assert_eq!(
            compile_fts_match(&filter),
            Some("title : dune* NOT description : spice*".into())
        );
    }

    #[test]
    fn match_column_filter_with_spaces() {
        let filter = BookFilter {
            fts_column_filters: vec![("title".into(), "dune messiah".into(), false)],
            ..Default::default()
        };
        assert_eq!(
            compile_fts_match(&filter),
            Some(r#"title : "dune messiah""#.into())
        );
    }

    #[test]
    fn match_skips_empty_column_term() {
        let filter = BookFilter {
            query: Some("dune".into()),
            fts_column_filters: vec![("title".into(), "   ".into(), false)],
            ..Default::default()
        };
        assert_eq!(compile_fts_match(&filter), Some("dune*".into()));
    }

    // ── prefix-specific ────────────────────────────────────────────

    #[test]
    fn escape_text_single_term_gets_prefix() {
        assert_eq!(escape_fts_text("sapkow"), "sapkow*");
    }

    #[test]
    fn escape_text_quoted_then_regular() {
        assert_eq!(
            escape_fts_text(r#""exact phrase" sapkow"#),
            r#""exact phrase" sapkow*"#,
        );
    }

    #[test]
    fn escape_text_all_negated_no_prefix() {
        assert_eq!(
            escape_fts_text("NOT dune NOT foundation"),
            "NOT dune NOT foundation",
        );
    }
}
