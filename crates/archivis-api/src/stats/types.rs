use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct StatsResponse {
    pub generated_at: DateTime<Utc>,
    pub library: LibraryStats,
    pub usage: UsageStats,
    pub db: Option<DbStats>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LibraryStats {
    pub books: i64,
    pub files: i64,
    pub total_file_size: i64,
    pub average_files_per_book: f64,
    pub files_by_format: Vec<FormatStat>,
    pub metadata_status: Vec<StatusCount>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FormatStat {
    pub format: String,
    pub file_count: i64,
    pub total_size: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UsageStats {
    pub tasks_total: i64,
    pub tasks_last_24h: i64,
    pub tasks_by_status: Vec<StatusCount>,
    pub tasks_by_type: Vec<TaskTypeCount>,
    pub pending_duplicates: i64,
    pub pending_candidates: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskTypeCount {
    pub task_type: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DbStats {
    pub files: DbFileStats,
    pub pages: DbPageStats,
    pub table_size_estimates_available: bool,
    pub objects: Vec<DbObjectStatResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DbFileStats {
    pub main_db_size: i64,
    pub wal_size: i64,
    pub shm_size: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DbPageStats {
    pub page_size: i64,
    pub page_count: i64,
    pub freelist_count: i64,
    pub used_pages: i64,
    pub used_bytes: i64,
    pub free_bytes: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DbObjectStatResponse {
    pub name: String,
    pub object_type: String,
    pub estimated_bytes: Option<i64>,
    pub row_count: Option<i64>,
}
