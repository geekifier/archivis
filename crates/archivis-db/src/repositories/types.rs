use serde::{Deserialize, Serialize};

use archivis_core::models::{BookFormat, MetadataStatus};

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
    /// Filter by book format.
    pub format: Option<BookFormat>,
    /// Filter by metadata status.
    pub status: Option<MetadataStatus>,
    /// Filter by tag IDs.
    pub tags: Option<Vec<String>>,
    /// Filter by author ID.
    pub author_id: Option<String>,
    /// Filter by series ID.
    pub series_id: Option<String>,
    /// Filter by publisher ID (direct column on books table).
    pub publisher_id: Option<String>,
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
