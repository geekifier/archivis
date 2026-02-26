use std::path::Path;

use axum::extract::State;
use axum::Json;
use chrono::Utc;

use archivis_core::models::UserRole;
use archivis_db::StatsRepository;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    DbFileStats, DbObjectStatResponse, DbPageStats, DbStats, FormatStat, LibraryStats,
    StatsResponse, StatusCount, TaskTypeCount, UsageStats,
};

/// GET /api/stats -- aggregated statistics for library, usage, and (admin-only) DB diagnostics.
#[utoipa::path(
    get,
    path = "/api/stats",
    tag = "stats",
    responses(
        (status = 200, description = "Aggregated statistics", body = StatsResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_stats(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<StatsResponse>, ApiError> {
    let pool = state.db_pool();

    let library_overview = StatsRepository::library_overview(pool).await?;
    let files_by_format = StatsRepository::files_by_format(pool).await?;
    let metadata_status = StatsRepository::metadata_status_counts(pool).await?;

    let task_overview = StatsRepository::task_overview(pool).await?;
    let task_status = StatsRepository::task_status_counts(pool).await?;
    let task_types = StatsRepository::task_type_counts(pool).await?;
    let pending_duplicates = StatsRepository::pending_duplicate_count(pool).await?;
    let pending_candidates = StatsRepository::pending_candidate_count(pool).await?;

    #[allow(clippy::cast_precision_loss)]
    let average_files_per_book = if library_overview.books > 0 {
        library_overview.files as f64 / library_overview.books as f64
    } else {
        0.0
    };

    let db = if user.role == UserRole::Admin {
        let pragma = StatsRepository::db_pragma_stats(pool).await?;
        let object_stats = StatsRepository::db_object_stats(pool).await?;

        let db_path = state.config().data_dir.join("archivis.db");
        let wal_path = state.config().data_dir.join("archivis.db-wal");
        let shm_path = state.config().data_dir.join("archivis.db-shm");

        let used_pages = (pragma.page_count - pragma.freelist_count).max(0);
        let used_bytes = used_pages.saturating_mul(pragma.page_size);
        let free_bytes = pragma.freelist_count.saturating_mul(pragma.page_size);

        Some(DbStats {
            files: DbFileStats {
                main_db_size: file_size_or_zero(&db_path).await,
                wal_size: file_size_or_zero(&wal_path).await,
                shm_size: file_size_or_zero(&shm_path).await,
            },
            pages: DbPageStats {
                page_size: pragma.page_size,
                page_count: pragma.page_count,
                freelist_count: pragma.freelist_count,
                used_pages,
                used_bytes,
                free_bytes,
            },
            table_size_estimates_available: object_stats.table_size_estimates_available,
            objects: object_stats
                .objects
                .into_iter()
                .map(|entry| DbObjectStatResponse {
                    name: entry.name,
                    object_type: entry.object_type,
                    estimated_bytes: entry.estimated_bytes,
                    row_count: entry.row_count,
                })
                .collect(),
        })
    } else {
        None
    };

    Ok(Json(StatsResponse {
        generated_at: Utc::now(),
        library: LibraryStats {
            books: library_overview.books,
            files: library_overview.files,
            total_file_size: library_overview.total_file_size,
            average_files_per_book,
            files_by_format: files_by_format
                .into_iter()
                .map(|entry| FormatStat {
                    format: entry.format,
                    file_count: entry.file_count,
                    total_size: entry.total_size,
                })
                .collect(),
            metadata_status: metadata_status
                .into_iter()
                .map(|entry| StatusCount {
                    status: entry.key,
                    count: entry.count,
                })
                .collect(),
        },
        usage: UsageStats {
            tasks_total: task_overview.total,
            tasks_last_24h: task_overview.last_24h,
            tasks_by_status: task_status
                .into_iter()
                .map(|entry| StatusCount {
                    status: entry.key,
                    count: entry.count,
                })
                .collect(),
            tasks_by_type: task_types
                .into_iter()
                .map(|entry| TaskTypeCount {
                    task_type: entry.key,
                    count: entry.count,
                })
                .collect(),
            pending_duplicates,
            pending_candidates,
        },
        db,
    }))
}

async fn file_size_or_zero(path: &Path) -> i64 {
    tokio::fs::metadata(path).await.map_or(0, |metadata| {
        i64::try_from(metadata.len()).unwrap_or(i64::MAX)
    })
}
