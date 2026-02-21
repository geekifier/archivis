use archivis_core::errors::DbError;
use archivis_core::models::Tag;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::book::TagRow;
use super::types::{PaginatedResult, PaginationParams};

pub struct TagRepository;

impl TagRepository {
    pub async fn create(pool: &SqlitePool, tag: &Tag) -> Result<(), DbError> {
        let id = tag.id.to_string();
        sqlx::query("INSERT INTO tags (id, name, category) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&tag.name)
            .bind(&tag.category)
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
        let row = sqlx::query_as::<_, TagRow>("SELECT id, name, category FROM tags WHERE id = ?")
            .bind(&id_str)
            .fetch_optional(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
            .ok_or(DbError::NotFound {
                entity: "tag",
                id: id_str,
            })?;

        row.into_tag()
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Tag>, DbError> {
        let sort_col = match params.sort_by.as_str() {
            "category" => "category",
            _ => "name",
        };
        let sort_dir = params.sort_order.as_sql();
        let limit = params.per_page;
        let offset = params.offset();

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let sql = format!(
            "SELECT id, name, category FROM tags ORDER BY {sort_col} {sort_dir} LIMIT {limit} OFFSET {offset}"
        );

        let rows = sqlx::query_as::<_, TagRow>(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let items = rows
            .into_iter()
            .map(TagRow::into_tag)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    /// Search tags by name and/or filter by category.
    pub async fn search(
        pool: &SqlitePool,
        query: Option<&str>,
        category: Option<&str>,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Tag>, DbError> {
        let sort_col = match params.sort_by.as_str() {
            "category" => "category",
            _ => "name",
        };
        let sort_dir = params.sort_order.as_sql();
        let limit = params.per_page;
        let offset = params.offset();

        let mut where_clauses = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if let Some(q) = query {
            if !q.is_empty() {
                where_clauses.push("name LIKE ? COLLATE NOCASE");
                binds.push(format!("%{q}%"));
            }
        }

        if let Some(cat) = category {
            if !cat.is_empty() {
                where_clauses.push("category = ? COLLATE NOCASE");
                binds.push(cat.to_string());
            }
        }

        let where_clause = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM tags{where_clause}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        for b in &binds {
            count_query = count_query.bind(b);
        }
        let total = count_query
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let sql = format!(
            "SELECT id, name, category FROM tags{where_clause} ORDER BY {sort_col} {sort_dir} LIMIT {limit} OFFSET {offset}"
        );
        let mut row_query = sqlx::query_as::<_, TagRow>(&sql);
        for b in &binds {
            row_query = row_query.bind(b);
        }
        let rows = row_query
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let items = rows
            .into_iter()
            .map(TagRow::into_tag)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, tag: &Tag) -> Result<(), DbError> {
        let id = tag.id.to_string();
        let result = sqlx::query("UPDATE tags SET name = ?, category = ? WHERE id = ?")
            .bind(&tag.name)
            .bind(&tag.category)
            .bind(&id)
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
        let result = sqlx::query("DELETE FROM tags WHERE id = ?")
            .bind(&id_str)
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
            sqlx::query_as::<_, TagRow>(
                "SELECT id, name, category FROM tags WHERE name = ? COLLATE NOCASE AND category = ? COLLATE NOCASE",
            )
            .bind(name)
            .bind(cat)
            .fetch_optional(pool)
            .await
        } else {
            sqlx::query_as::<_, TagRow>(
                "SELECT id, name, category FROM tags WHERE name = ? COLLATE NOCASE AND category IS NULL",
            )
            .bind(name)
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
