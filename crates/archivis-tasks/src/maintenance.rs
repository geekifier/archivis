use std::time::Duration;

use archivis_db::DbPool;
use chrono::Utc;

const TASK_RETENTION_DAYS: i64 = 30;
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const INITIAL_DELAY: Duration = Duration::from_secs(60);

/// Run periodic DB maintenance (expired sessions + old terminal tasks).
///
/// Waits an initial delay, then loops on a fixed interval. Cancel-safe —
/// just drop the future / abort the spawned task.
pub async fn run_maintenance_loop(db_pool: DbPool) {
    tokio::time::sleep(INITIAL_DELAY).await;
    tracing::debug!("DB maintenance loop started");

    let mut interval = tokio::time::interval(MAINTENANCE_INTERVAL);
    // First tick fires immediately (we already waited the initial delay).
    interval.tick().await;

    loop {
        run_maintenance_cycle(&db_pool).await;
        interval.tick().await;
    }
}

async fn run_maintenance_cycle(db_pool: &DbPool) {
    // 1. Expired sessions
    match archivis_db::SessionRepository::delete_expired(db_pool).await {
        Ok(0) => tracing::debug!("maintenance: no expired sessions to clean up"),
        Ok(n) => tracing::info!(deleted = n, "maintenance: cleaned up expired sessions"),
        Err(e) => tracing::warn!(error = %e, "maintenance: failed to delete expired sessions"),
    }

    // 2. Old terminal tasks
    let cutoff = Utc::now() - chrono::Duration::days(TASK_RETENTION_DAYS);
    match archivis_db::TaskRepository::delete_terminal_older_than(db_pool, cutoff).await {
        Ok(0) => tracing::debug!("maintenance: no old tasks to clean up"),
        Ok(n) => tracing::info!(deleted = n, "maintenance: cleaned up old terminal tasks"),
        Err(e) => tracing::warn!(error = %e, "maintenance: failed to delete old tasks"),
    }
}
