use archivis_core::errors::DbError;
use archivis_core::models::{Identifier, MetadataSource};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::book::IdentifierRow;

pub struct IdentifierRepository;

impl IdentifierRepository {
    pub async fn create(pool: &SqlitePool, identifier: &Identifier) -> Result<(), DbError> {
        let id = identifier.id.to_string();
        let book_id = identifier.book_id.to_string();
        let identifier_type = serde_json::to_value(identifier.identifier_type)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();

        let (source_type, source_name) = match &identifier.source {
            MetadataSource::Embedded => ("embedded", None),
            MetadataSource::Filename => ("filename", None),
            MetadataSource::User => ("user", None),
            MetadataSource::Provider(name) => ("provider", Some(name.as_str())),
        };

        sqlx::query!(
            "INSERT INTO identifiers (id, book_id, identifier_type, value, source_type, source_name, confidence) VALUES (?, ?, ?, ?, ?, ?, ?)",
            id,
            book_id,
            identifier_type,
            identifier.value,
            source_type,
            source_name,
            identifier.confidence,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_book_id(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<Identifier>, DbError> {
        let id_str = book_id.to_string();
        let rows = sqlx::query_as!(
            IdentifierRow,
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(IdentifierRow::into_identifier)
            .collect()
    }

    /// Delete all identifiers for a book that came from a specific provider.
    pub async fn delete_by_provider(
        pool: &SqlitePool,
        book_id: Uuid,
        provider_name: &str,
    ) -> Result<u64, DbError> {
        let book_id_str = book_id.to_string();
        let result = sqlx::query!(
            "DELETE FROM identifiers WHERE book_id = ? AND source_type = 'provider' AND source_name = ?",
            book_id_str,
            provider_name,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }

    pub async fn find_by_value(
        pool: &SqlitePool,
        identifier_type: &str,
        value: &str,
    ) -> Result<Vec<Identifier>, DbError> {
        let rows = sqlx::query_as!(
            IdentifierRow,
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE identifier_type = ? AND value = ?",
            identifier_type,
            value,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(IdentifierRow::into_identifier)
            .collect()
    }

    /// Get a single identifier by its ID.
    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Identifier, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            IdentifierRow,
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "identifier",
            id: id_str,
        })?;

        row.into_identifier()
    }

    /// Update an existing identifier's value and/or type.
    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        value: &str,
        identifier_type: &str,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!(
            "UPDATE identifiers SET value = ?, identifier_type = ? WHERE id = ?",
            value,
            identifier_type,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "identifier",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Delete a single identifier by ID.
    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM identifiers WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "identifier",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Check if an identifier value already exists for a book (to prevent duplicates).
    pub async fn exists_for_book(
        pool: &SqlitePool,
        book_id: Uuid,
        identifier_type: &str,
        value: &str,
    ) -> Result<bool, DbError> {
        let book_id_str = book_id.to_string();
        let row = sqlx::query_scalar!(
            "SELECT COUNT(*) as count FROM identifiers WHERE book_id = ? AND identifier_type = ? AND value = ?",
            book_id_str,
            identifier_type,
            value,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(row > 0)
    }
}
