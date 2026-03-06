use std::collections::HashMap;

use archivis_core::errors::DbError;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct LibraryOverview {
    pub books: i64,
    pub files: i64,
    pub total_file_size: i64,
}

#[derive(Debug, Clone)]
pub struct FormatFileStat {
    pub format: String,
    pub file_count: i64,
    pub total_size: i64,
}

#[derive(Debug, Clone)]
pub struct BucketCount {
    pub key: String,
    pub count: i64,
}

#[derive(Debug, Clone)]
pub struct TaskOverview {
    pub total: i64,
    pub last_24h: i64,
}

#[derive(Debug, Clone)]
pub struct DbPragmaStats {
    pub page_size: i64,
    pub page_count: i64,
    pub freelist_count: i64,
}

#[derive(Debug, Clone)]
pub struct DbObjectStat {
    pub name: String,
    pub object_type: String,
    pub estimated_bytes: Option<i64>,
    pub row_count: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct DbObjectStats {
    pub table_size_estimates_available: bool,
    pub objects: Vec<DbObjectStat>,
}

#[derive(Debug, Clone)]
pub struct SidebarCounts {
    pub duplicates: i64,
    pub needs_review: i64,
    pub unidentified: i64,
}

pub struct StatsRepository;

impl StatsRepository {
    pub async fn library_overview(pool: &SqlitePool) -> Result<LibraryOverview, DbError> {
        let row = sqlx::query(
            r"
            SELECT
                (SELECT COUNT(*) FROM books) AS books,
                (SELECT COUNT(*) FROM book_files) AS files,
                COALESCE((SELECT SUM(file_size) FROM book_files), 0) AS total_file_size
            ",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(LibraryOverview {
            books: row
                .try_get("books")
                .map_err(|e| DbError::Query(e.to_string()))?,
            files: row
                .try_get("files")
                .map_err(|e| DbError::Query(e.to_string()))?,
            total_file_size: row
                .try_get("total_file_size")
                .map_err(|e| DbError::Query(e.to_string()))?,
        })
    }

    pub async fn files_by_format(pool: &SqlitePool) -> Result<Vec<FormatFileStat>, DbError> {
        let rows = sqlx::query(
            r"
            SELECT
                LOWER(format) AS format,
                COUNT(*) AS file_count,
                COALESCE(SUM(file_size), 0) AS total_size
            FROM book_files
            GROUP BY format
            ORDER BY file_count DESC, format ASC
            ",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                Ok(FormatFileStat {
                    format: row
                        .try_get::<String, _>("format")
                        .map_err(|e| DbError::Query(e.to_string()))?,
                    file_count: row
                        .try_get("file_count")
                        .map_err(|e| DbError::Query(e.to_string()))?,
                    total_size: row
                        .try_get("total_size")
                        .map_err(|e| DbError::Query(e.to_string()))?,
                })
            })
            .collect()
    }

    pub async fn metadata_status_counts(pool: &SqlitePool) -> Result<Vec<BucketCount>, DbError> {
        Self::bucket_counts(
            pool,
            r"
            SELECT metadata_status AS key, COUNT(*) AS count
            FROM books
            GROUP BY metadata_status
            ORDER BY count DESC, key ASC
            ",
        )
        .await
    }

    pub async fn task_status_counts(pool: &SqlitePool) -> Result<Vec<BucketCount>, DbError> {
        Self::bucket_counts(
            pool,
            r"
            SELECT status AS key, COUNT(*) AS count
            FROM tasks
            GROUP BY status
            ORDER BY count DESC, key ASC
            ",
        )
        .await
    }

    pub async fn task_type_counts(pool: &SqlitePool) -> Result<Vec<BucketCount>, DbError> {
        Self::bucket_counts(
            pool,
            r"
            SELECT task_type AS key, COUNT(*) AS count
            FROM tasks
            GROUP BY task_type
            ORDER BY count DESC, key ASC
            ",
        )
        .await
    }

    pub async fn task_overview(pool: &SqlitePool) -> Result<TaskOverview, DbError> {
        let row = sqlx::query(
            r"
            SELECT
                COUNT(*) AS total,
                COALESCE(
                    SUM(
                        CASE
                            WHEN datetime(created_at) >= datetime('now', '-1 day') THEN 1
                            ELSE 0
                        END
                    ),
                    0
                ) AS last_24h
            FROM tasks
            ",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(TaskOverview {
            total: row
                .try_get("total")
                .map_err(|e| DbError::Query(e.to_string()))?,
            last_24h: row
                .try_get("last_24h")
                .map_err(|e| DbError::Query(e.to_string()))?,
        })
    }

    pub async fn pending_duplicate_count(pool: &SqlitePool) -> Result<i64, DbError> {
        Self::count_scalar(
            pool,
            "SELECT COUNT(*) AS count FROM duplicate_links WHERE status = 'pending'",
        )
        .await
    }

    pub async fn sidebar_counts(pool: &SqlitePool) -> Result<SidebarCounts, DbError> {
        let row = sqlx::query(
            r"SELECT
                (SELECT COUNT(*) FROM duplicate_links WHERE status = 'pending') AS duplicates,
                (SELECT COUNT(*) FROM books WHERE metadata_status = 'needs_review') AS needs_review,
                (SELECT COUNT(*) FROM books WHERE metadata_status = 'unidentified') AS unidentified",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(SidebarCounts {
            duplicates: row
                .try_get("duplicates")
                .map_err(|e| DbError::Query(e.to_string()))?,
            needs_review: row
                .try_get("needs_review")
                .map_err(|e| DbError::Query(e.to_string()))?,
            unidentified: row
                .try_get("unidentified")
                .map_err(|e| DbError::Query(e.to_string()))?,
        })
    }

    pub async fn pending_candidate_count(pool: &SqlitePool) -> Result<i64, DbError> {
        Self::count_scalar(
            pool,
            "SELECT COUNT(*) AS count
             FROM identification_candidates candidates
             LEFT JOIN resolution_runs runs ON runs.id = candidates.run_id
             WHERE candidates.status = 'pending'
               AND (
                   (
                       candidates.run_id IS NULL
                       AND NOT EXISTS (
                           SELECT 1
                           FROM resolution_runs current_runs
                           WHERE current_runs.book_id = candidates.book_id
                             AND current_runs.state != 'superseded'
                       )
                   )
                   OR (
                       runs.state != 'superseded'
                       AND (
                           runs.state = 'running'
                           OR runs.outcome IN ('ambiguous', 'disputed')
                       )
                       AND runs.id = (
                           SELECT current_runs.id
                           FROM resolution_runs current_runs
                           WHERE current_runs.book_id = candidates.book_id
                             AND current_runs.state != 'superseded'
                             AND (
                                 current_runs.state = 'running'
                                 OR current_runs.outcome IN ('ambiguous', 'disputed')
                             )
                           ORDER BY current_runs.started_at DESC
                           LIMIT 1
                       )
                   )
               )",
        )
        .await
    }

    pub async fn db_pragma_stats(pool: &SqlitePool) -> Result<DbPragmaStats, DbError> {
        let page_size = Self::pragma_i64(pool, "page_size").await?;
        let page_count = Self::pragma_i64(pool, "page_count").await?;
        let freelist_count = Self::pragma_i64(pool, "freelist_count").await?;

        Ok(DbPragmaStats {
            page_size,
            page_count,
            freelist_count,
        })
    }

    pub async fn db_object_stats(pool: &SqlitePool) -> Result<DbObjectStats, DbError> {
        let schema_rows = sqlx::query(
            r"
            SELECT name, type
            FROM sqlite_master
            WHERE type IN ('table', 'index')
              AND name NOT LIKE 'sqlite_%'
            ORDER BY type, name
            ",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let mut schema_objects = HashMap::new();
        for row in schema_rows {
            let name: String = row
                .try_get("name")
                .map_err(|e| DbError::Query(e.to_string()))?;
            let object_type: String = row
                .try_get("type")
                .map_err(|e| DbError::Query(e.to_string()))?;
            schema_objects.insert(name, object_type);
        }

        let dbstat_sizes = Self::load_dbstat_sizes(pool).await?;
        let table_size_estimates_available = dbstat_sizes.is_some();
        let size_map = dbstat_sizes.unwrap_or_default();

        let mut objects = Vec::with_capacity(schema_objects.len());
        for (name, object_type) in schema_objects {
            let row_count = if object_type == "table" {
                Some(Self::table_row_count(pool, &name).await?)
            } else {
                None
            };

            objects.push(DbObjectStat {
                estimated_bytes: size_map.get(&name).copied(),
                name,
                object_type,
                row_count,
            });
        }

        objects.sort_by(|a, b| {
            let a_bytes = a.estimated_bytes.unwrap_or(-1);
            let b_bytes = b.estimated_bytes.unwrap_or(-1);
            b_bytes
                .cmp(&a_bytes)
                .then_with(|| a.object_type.cmp(&b.object_type))
                .then_with(|| a.name.cmp(&b.name))
        });

        Ok(DbObjectStats {
            table_size_estimates_available,
            objects,
        })
    }

    async fn bucket_counts(pool: &SqlitePool, sql: &str) -> Result<Vec<BucketCount>, DbError> {
        let rows = sqlx::query(sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                Ok(BucketCount {
                    key: row
                        .try_get("key")
                        .map_err(|e| DbError::Query(e.to_string()))?,
                    count: row
                        .try_get("count")
                        .map_err(|e| DbError::Query(e.to_string()))?,
                })
            })
            .collect()
    }

    async fn count_scalar(pool: &SqlitePool, sql: &str) -> Result<i64, DbError> {
        let row = sqlx::query(sql)
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        row.try_get("count")
            .map_err(|e| DbError::Query(e.to_string()))
    }

    async fn pragma_i64(pool: &SqlitePool, pragma_name: &str) -> Result<i64, DbError> {
        let sql = format!("PRAGMA {pragma_name}");
        let row = sqlx::query(&sql)
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        row.try_get(0).map_err(|e| DbError::Query(e.to_string()))
    }

    async fn load_dbstat_sizes(pool: &SqlitePool) -> Result<Option<HashMap<String, i64>>, DbError> {
        let result = sqlx::query(
            r"
            SELECT name, SUM(pgsize) AS estimated_bytes
            FROM dbstat
            WHERE name NOT LIKE 'sqlite_%'
            GROUP BY name
            ",
        )
        .fetch_all(pool)
        .await;

        let rows = match result {
            Ok(rows) => rows,
            Err(err) => {
                let msg = err.to_string().to_lowercase();
                if msg.contains("no such table: dbstat") || msg.contains("no such module: dbstat") {
                    return Ok(None);
                }
                return Err(DbError::Query(err.to_string()));
            }
        };

        let mut sizes = HashMap::new();
        for row in rows {
            let name: String = row
                .try_get("name")
                .map_err(|e| DbError::Query(e.to_string()))?;
            let estimated_bytes: i64 = row
                .try_get("estimated_bytes")
                .map_err(|e| DbError::Query(e.to_string()))?;
            sizes.insert(name, estimated_bytes);
        }

        Ok(Some(sizes))
    }

    async fn table_row_count(pool: &SqlitePool, table_name: &str) -> Result<i64, DbError> {
        let escaped = table_name.replace('"', "\"\"");
        let sql = format!("SELECT COUNT(*) AS count FROM \"{escaped}\"");
        let row = sqlx::query(&sql)
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        row.try_get("count")
            .map_err(|e| DbError::Query(e.to_string()))
    }
}
