use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::state::AppState;

use super::types::{HealthChecks, HealthResponse};

/// Kubernetes liveness probe.
///
/// Returns 200 if the process is running. No dependency checks — a failure
/// (no response) tells the orchestrator the process is dead or deadlocked.
pub async fn liveness() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        checks: None,
    })
}

/// Kubernetes readiness probe.
///
/// Returns 200 when the application can serve traffic (DB reachable).
/// Returns 503 when a critical dependency is unavailable, causing the
/// orchestrator to remove the instance from load-balancer endpoints.
pub async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let db_ok = archivis_db::ping(state.db_pool()).await.is_ok();

    let (status_code, status_str, db_str) = if db_ok {
        (StatusCode::OK, "ok", "ok")
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "unavailable",
        )
    };

    (
        status_code,
        Json(HealthResponse {
            status: status_str,
            checks: Some(HealthChecks { database: db_str }),
        }),
    )
}
