use archivis_core::errors::DbError;
use archivis_core::models::Tag;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::book::TagRow;
use super::types::{PaginatedResult, PaginationParams, SortOrder};

/// A tag together with a pre-computed book count (from list/search queries).
pub struct TagWithBookCount {
    pub tag: Tag,
    pub book_count: u32,
}

pub struct TagRepository;

/// Helper to fetch a page of tag-with-count rows with a given ORDER BY clause.
macro_rules! fetch_tag_count_rows {
    ($sql:literal, $pool:expr $(, $bind:expr)*) => {
        sqlx::query_as!(TagWithCountRow, $sql $(, $bind)*)
            .fetch_all($pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
    };
}

impl TagRepository {
    pub async fn create(pool: &SqlitePool, tag: &Tag) -> Result<(), DbError> {
        let id = tag.id.to_string();
        sqlx::query!(
            "INSERT INTO tags (id, name, category) VALUES (?, ?, ?)",
            id,
            tag.name,
            tag.category,
        )
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                DbError::Constraint(format!(
                    "tag '{}' already exists in category {:?}",
                    tag.name, tag.category
                ))
            } else {
                DbError::Query(e.to_string())
            }
        })?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Tag, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            TagRow,
            "SELECT id, name, category FROM tags WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "tag",
            id: id_str,
        })?;

        row.into_tag()
    }

    /// Get a tag by ID with its book count.
    pub async fn get_by_id_with_count(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<TagWithBookCount, DbError> {
        let tag = Self::get_by_id(pool, id).await?;
        let id_str = id.to_string();
        let count = sqlx::query_scalar!("SELECT COUNT(*) FROM book_tags WHERE tag_id = ?", id_str,)
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(TagWithBookCount {
            tag,
            book_count: count as u32,
        })
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<TagWithBookCount>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        let total = sqlx::query_scalar!("SELECT COUNT(*) FROM tags")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match (params.sort_by.as_str(), params.sort_order) {
            ("category", SortOrder::Asc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t ORDER BY t.category ASC, t.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("category", SortOrder::Desc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t ORDER BY t.category DESC, t.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("book_count", SortOrder::Asc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t ORDER BY book_count ASC, t.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("book_count", SortOrder::Desc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t ORDER BY book_count DESC, t.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t ORDER BY t.name DESC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            // Default: name ASC
            _ => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t ORDER BY t.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
        };

        let items = rows
            .into_iter()
            .map(TagWithCountRow::into_tag_with_count)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    /// Search tags by name and/or filter by category.
    /// Uses `IS NULL OR` pattern for optional filters — `SQLite` short-circuits unused conditions.
    pub async fn search(
        pool: &SqlitePool,
        query: Option<&str>,
        category: Option<&str>,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<TagWithBookCount>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        // Convert query to LIKE pattern; NULL disables the filter via IS NULL OR
        let pattern = query.filter(|q| !q.is_empty()).map(|q| format!("%{q}%"));
        // Empty category string is treated as "no filter"
        let cat_filter = category.filter(|c| !c.is_empty());

        let total = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM tags WHERE (? IS NULL OR name LIKE ? COLLATE NOCASE) AND (? IS NULL OR category = ? COLLATE NOCASE)",
            pattern,
            pattern,
            cat_filter,
            cat_filter,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match (params.sort_by.as_str(), params.sort_order) {
            ("category", SortOrder::Asc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t WHERE (? IS NULL OR t.name LIKE ? COLLATE NOCASE) AND (? IS NULL OR t.category = ? COLLATE NOCASE) ORDER BY t.category ASC, t.name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, cat_filter, cat_filter, limit, offset
            ),
            ("category", SortOrder::Desc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t WHERE (? IS NULL OR t.name LIKE ? COLLATE NOCASE) AND (? IS NULL OR t.category = ? COLLATE NOCASE) ORDER BY t.category DESC, t.name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, cat_filter, cat_filter, limit, offset
            ),
            ("book_count", SortOrder::Asc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t WHERE (? IS NULL OR t.name LIKE ? COLLATE NOCASE) AND (? IS NULL OR t.category = ? COLLATE NOCASE) ORDER BY book_count ASC, t.name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, cat_filter, cat_filter, limit, offset
            ),
            ("book_count", SortOrder::Desc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t WHERE (? IS NULL OR t.name LIKE ? COLLATE NOCASE) AND (? IS NULL OR t.category = ? COLLATE NOCASE) ORDER BY book_count DESC, t.name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, cat_filter, cat_filter, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t WHERE (? IS NULL OR t.name LIKE ? COLLATE NOCASE) AND (? IS NULL OR t.category = ? COLLATE NOCASE) ORDER BY t.name DESC LIMIT ? OFFSET ?",
                pool, pattern, pattern, cat_filter, cat_filter, limit, offset
            ),
            _ => fetch_tag_count_rows!(
                "SELECT t.id, t.name, t.category, (SELECT COUNT(*) FROM book_tags bt WHERE bt.tag_id = t.id) AS book_count FROM tags t WHERE (? IS NULL OR t.name LIKE ? COLLATE NOCASE) AND (? IS NULL OR t.category = ? COLLATE NOCASE) ORDER BY t.name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, cat_filter, cat_filter, limit, offset
            ),
        };

        let items = rows
            .into_iter()
            .map(TagWithCountRow::into_tag_with_count)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    /// Return all distinct non-null, non-empty category values.
    pub async fn list_categories(pool: &SqlitePool) -> Result<Vec<String>, DbError> {
        let rows = sqlx::query_scalar!(
            "SELECT DISTINCT category FROM tags WHERE category IS NOT NULL AND category != '' ORDER BY category ASC"
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(rows.into_iter().flatten().collect())
    }

    /// Find tags by name (case-insensitive exact match).
    ///
    /// Returns a `Vec` because tags can share names across different categories.
    pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Vec<Tag>, DbError> {
        let rows = sqlx::query_as!(
            TagRow,
            "SELECT id, name, category FROM tags WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(TagRow::into_tag).collect()
    }

    pub async fn update(pool: &SqlitePool, tag: &Tag) -> Result<(), DbError> {
        let id = tag.id.to_string();
        let result = sqlx::query!(
            "UPDATE tags SET name = ?, category = ? WHERE id = ?",
            tag.name,
            tag.category,
            id,
        )
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                DbError::Constraint(format!(
                    "tag '{}' already exists in category {:?}",
                    tag.name, tag.category
                ))
            } else {
                DbError::Query(e.to_string())
            }
        })?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "tag", id });
        }

        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM tags WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "tag",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Find a tag by name and category, or create it if it doesn't exist.
    pub async fn find_or_create(
        pool: &SqlitePool,
        name: &str,
        category: Option<&str>,
    ) -> Result<Tag, DbError> {
        // Try to find existing
        let row = if let Some(cat) = category {
            sqlx::query_as!(
                TagRow,
                "SELECT id, name, category FROM tags WHERE name = ? COLLATE NOCASE AND category = ? COLLATE NOCASE",
                name,
                cat,
            )
            .fetch_optional(pool)
            .await
        } else {
            sqlx::query_as!(
                TagRow,
                "SELECT id, name, category FROM tags WHERE name = ? COLLATE NOCASE AND category IS NULL",
                name,
            )
            .fetch_optional(pool)
            .await
        }
        .map_err(|e| DbError::Query(e.to_string()))?;

        if let Some(row) = row {
            return row.into_tag();
        }

        // Create new
        let tag = category.map_or_else(|| Tag::new(name), |cat| Tag::with_category(name, cat));

        Self::create(pool, &tag).await?;
        Ok(tag)
    }
}

// ── Private row type for tag-with-count queries ─────────────────

struct TagWithCountRow {
    id: String,
    name: String,
    category: Option<String>,
    book_count: i64,
}

impl TagWithCountRow {
    fn into_tag_with_count(self) -> Result<TagWithBookCount, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid tag UUID: {e}")))?;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let book_count = self.book_count as u32;
        Ok(TagWithBookCount {
            tag: Tag {
                id,
                name: self.name,
                category: self.category,
            },
            book_count,
        })
    }
}
