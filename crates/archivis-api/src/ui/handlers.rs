use axum::extract::State;
use axum::Json;

use archivis_db::StatsRepository;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::SidebarCountsResponse;

/// GET /api/ui/sidebar-counts -- atomic snapshot of sidebar badge counts.
#[utoipa::path(
    get,
    path = "/api/ui/sidebar-counts",
    tag = "ui",
    responses(
        (status = 200, description = "Sidebar badge counts", body = SidebarCountsResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn sidebar_counts(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<SidebarCountsResponse>, ApiError> {
    let counts = StatsRepository::sidebar_counts(state.db_pool()).await?;

    Ok(Json(SidebarCountsResponse {
        duplicates: counts.duplicates,
        needs_review: counts.needs_review,
        unidentified: counts.unidentified,
        active_tasks: counts.active_tasks,
    }))
}
