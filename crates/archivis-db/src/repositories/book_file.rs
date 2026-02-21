use archivis_core::errors::DbError;
use archivis_core::models::BookFile;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::book::BookFileRow;

pub struct BookFileRepository;

impl BookFileRepository {
    pub async fn create(pool: &SqlitePool, file: &BookFile) -> Result<(), DbError> {
        let id = file.id.to_string();
        let book_id = file.book_id.to_string();
        let format = serde_json::to_value(file.format)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unknown".into());
        let added_at = file.added_at.to_rfc3339();

        sqlx::query(
            "INSERT INTO book_files (id, book_id, format, storage_path, file_size, hash, added_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&book_id)
        .bind(&format)
        .bind(&file.storage_path)
        .bind(file.file_size)
        .bind(&file.hash)
        .bind(&added_at)
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                DbError::Constraint(format!("file with hash {} already exists", file.hash))
            } else {
                DbError::Query(e.to_string())
            }
        })?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<BookFile, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, BookFileRow>(
            "SELECT id, book_id, format, storage_path, file_size, hash, added_at FROM book_files WHERE id = ?",
        )
        .bind(&id_str)
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "book_file",
            id: id_str,
        })?;

        row.into_book_file()
    }

    pub async fn get_by_book_id(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<BookFile>, DbError> {
        let id_str = book_id.to_string();
        let rows = sqlx::query_as::<_, BookFileRow>(
            "SELECT id, book_id, format, storage_path, file_size, hash, added_at FROM book_files WHERE book_id = ?",
        )
        .bind(&id_str)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(BookFileRow::into_book_file).collect()
    }

    pub async fn get_by_hash(pool: &SqlitePool, hash: &str) -> Result<Option<BookFile>, DbError> {
        let row = sqlx::query_as::<_, BookFileRow>(
            "SELECT id, book_id, format, storage_path, file_size, hash, added_at FROM book_files WHERE hash = ?",
        )
        .bind(hash)
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(BookFileRow::into_book_file).transpose()
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query("DELETE FROM book_files WHERE id = ?")
            .bind(&id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book_file",
                id: id_str,
            });
        }

        Ok(())
    }
}
