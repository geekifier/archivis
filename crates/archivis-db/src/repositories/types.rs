use serde::{Deserialize, Serialize};

use archivis_core::models::{
    BookFormat, MetadataStatus, ResolutionOutcome, ResolutionState, TagMatchMode,
};

/// Pagination parameters for list queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    pub page: u32,
    pub per_page: u32,
    pub sort_by: String,
    pub sort_order: SortOrder,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 25,
            sort_by: "added_at".into(),
            sort_order: SortOrder::Desc,
        }
    }
}

impl PaginationParams {
    pub fn offset(&self) -> u32 {
        (self.page.saturating_sub(1)) * self.per_page
    }

    /// Resolve the effective `sort_by` given an optional caller-supplied value
    /// and whether a text search query is active.
    ///
    /// - Explicit `sort_by` is always honored.
    /// - When omitted: `"relevance"` if `has_query`, else `"added_at"`.
    pub fn resolve_default_sort(explicit: Option<String>, has_query: bool) -> String {
        explicit.unwrap_or_else(|| {
            if has_query {
                "relevance".into()
            } else {
                "added_at".into()
            }
        })
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// Filter criteria for book listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookFilter {
    /// Full-text search query.
    pub query: Option<String>,
    /// Fail closed when the parsed query had clauses but none remain executable.
    pub matches_nothing: bool,
    /// Filter by book format.
    pub format: Option<BookFormat>,
    /// Filter by metadata status.
    pub status: Option<MetadataStatus>,
    /// Filter by tag IDs.
    pub tags: Option<Vec<String>>,
    /// How multiple tags should be matched (`Any` = OR, `All` = AND).
    pub tag_match: TagMatchMode,
    /// Filter by author ID.
    pub author_id: Option<String>,
    /// Filter by series ID.
    pub series_id: Option<String>,
    /// Filter by publisher ID (direct column on books table).
    pub publisher_id: Option<String>,
    /// Filter by `metadata_user_trusted` flag.
    pub trusted: Option<bool>,
    /// Filter by `metadata_locked` flag.
    pub locked: Option<bool>,
    /// Filter by resolution state.
    pub resolution_state: Option<ResolutionState>,
    /// Filter by resolution outcome.
    pub resolution_outcome: Option<ResolutionOutcome>,
    /// Filter by language code (exact match).
    pub language: Option<String>,
    /// Minimum publication year (inclusive).
    pub year_min: Option<i32>,
    /// Maximum publication year (inclusive).
    pub year_max: Option<i32>,
    /// Filter by cover presence.
    pub has_cover: Option<bool>,
    /// Filter by description presence.
    pub has_description: Option<bool>,
    /// Filter by having at least one identifier.
    pub has_identifiers: Option<bool>,
    /// Identifier type(s) for lookup (e.g. `["isbn13", "isbn10"]` or `["asin"]`).
    /// When multiple types are provided, the query matches any of them (OR).
    pub identifier_types: Option<Vec<String>>,
    /// Identifier value for lookup (used with `identifier_types`).
    pub identifier_value: Option<String>,
    /// Tag IDs to exclude (negated tag filter from DSL).
    pub neg_tag_ids: Option<Vec<String>>,
    /// Author ID to exclude (negated author filter from DSL).
    pub neg_author_id: Option<String>,
    /// Series ID to exclude (negated series filter from DSL).
    pub neg_series_id: Option<String>,
    /// Publisher ID to exclude (negated publisher filter from DSL).
    pub neg_publisher_id: Option<String>,
    /// FTS5 column-qualified filters from DSL resolution.
    /// Each tuple: (`column_name`, `search_term`, `is_negated`).
    pub fts_column_filters: Vec<(String, String, bool)>,
}

/// Paginated query result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResult<T> {
    pub items: Vec<T>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

impl<T> PaginatedResult<T> {
    pub fn new(items: Vec<T>, total: u32, params: &PaginationParams) -> Self {
        let total_pages = if params.per_page == 0 {
            0
        } else {
            total.div_ceil(params.per_page)
        };
        Self {
            items,
            total,
            page: params.page,
            per_page: params.per_page,
            total_pages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_default_sort_search_without_explicit() {
        assert_eq!(
            PaginationParams::resolve_default_sort(None, true),
            "relevance"
        );
    }

    #[test]
    fn resolve_default_sort_search_with_explicit_title() {
        assert_eq!(
            PaginationParams::resolve_default_sort(Some("title".into()), true),
            "title"
        );
    }

    #[test]
    fn resolve_default_sort_no_search_without_explicit() {
        assert_eq!(
            PaginationParams::resolve_default_sort(None, false),
            "added_at"
        );
    }

    #[test]
    fn resolve_default_sort_no_search_with_explicit() {
        assert_eq!(
            PaginationParams::resolve_default_sort(Some("rating".into()), false),
            "rating"
        );
    }
}
