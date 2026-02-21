use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use archivis_db::TaskRepository;

use crate::errors::ApiError;
use crate::state::AppState;

use super::types::TaskResponse;

/// GET /api/tasks -- list recent tasks (active and completed, up to 50).
#[utoipa::path(
    get,
    path = "/api/tasks",
    tag = "tasks",
    responses(
        (status = 200, description = "List of recent tasks", body = Vec<TaskResponse>),
    )
)]
pub async fn list_tasks(
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    let tasks = TaskRepository::list_recent(state.db_pool(), 50).await?;
    let responses: Vec<TaskResponse> = tasks.into_iter().map(Into::into).collect();
    Ok(Json(responses))
}

/// GET /api/tasks/{id} -- get a single task by ID.
#[utoipa::path(
    get,
    path = "/api/tasks/{id}",
    tag = "tasks",
    params(
        ("id" = Uuid, Path, description = "Task ID"),
    ),
    responses(
        (status = 200, description = "Task details", body = TaskResponse),
        (status = 404, description = "Task not found"),
    )
)]
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = TaskRepository::get_by_id(state.db_pool(), id).await?;
    Ok(Json(task.into()))
}
