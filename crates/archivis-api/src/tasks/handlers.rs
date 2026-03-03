use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use archivis_core::models::TaskStatus;
use archivis_db::TaskRepository;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::TaskResponse;

/// GET /api/tasks -- list recent top-level tasks (with children summary).
#[utoipa::path(
    get,
    path = "/api/tasks",
    tag = "tasks",
    responses(
        (status = 200, description = "List of recent top-level tasks", body = Vec<TaskResponse>),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    let tasks = TaskRepository::list_recent(state.db_pool(), 50).await?;
    let mut responses = Vec::with_capacity(tasks.len());
    for task in tasks {
        let task_id = task.id;
        let mut resp: TaskResponse = task.into();
        // Attach children summary for tasks that have children
        let summary = TaskRepository::child_summary(state.db_pool(), task_id).await?;
        if summary.total > 0 {
            resp.children_summary = Some(summary.into());
        }
        responses.push(resp);
    }
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
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Task not found"),
    ),
    security(("bearer" = []))
)]
pub async fn get_task(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = TaskRepository::get_by_id(state.db_pool(), id).await?;
    let mut resp: TaskResponse = task.into();
    let summary = TaskRepository::child_summary(state.db_pool(), id).await?;
    if summary.total > 0 {
        resp.children_summary = Some(summary.into());
    }
    Ok(Json(resp))
}

/// GET /api/tasks/{id}/children -- list child tasks of a parent.
#[utoipa::path(
    get,
    path = "/api/tasks/{id}/children",
    tag = "tasks",
    params(
        ("id" = Uuid, Path, description = "Parent task ID"),
    ),
    responses(
        (status = 200, description = "List of child tasks", body = Vec<TaskResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Task not found"),
    ),
    security(("bearer" = []))
)]
pub async fn list_children(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    // Verify parent exists
    let _parent = TaskRepository::get_by_id(state.db_pool(), id).await?;
    let children = TaskRepository::list_children(state.db_pool(), id).await?;
    let responses: Vec<TaskResponse> = children.into_iter().map(Into::into).collect();
    Ok(Json(responses))
}

/// POST /api/tasks/{id}/cancel -- cancel a running or pending task.
#[utoipa::path(
    post,
    path = "/api/tasks/{id}/cancel",
    tag = "tasks",
    params(
        ("id" = Uuid, Path, description = "Task ID to cancel"),
    ),
    responses(
        (status = 200, description = "Task cancelled", body = TaskResponse),
        (status = 400, description = "Task cannot be cancelled"),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Task not found"),
    ),
    security(("bearer" = []))
)]
pub async fn cancel_task(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = TaskRepository::get_by_id(state.db_pool(), id).await?;

    if task.status.is_terminal() {
        return Err(ApiError::Validation(format!(
            "task is already in terminal state: {}",
            task.status
        )));
    }

    match task.status {
        TaskStatus::Pending => {
            // Immediately cancel pending task and its pending children
            TaskRepository::update_status(
                state.db_pool(),
                id,
                TaskStatus::Cancelled,
                None,
                Some(chrono::Utc::now()),
                None,
                None,
            )
            .await?;
            TaskRepository::cancel_pending_children(state.db_pool(), id).await?;

            // Broadcast cancellation event
            let update = archivis_core::models::TaskProgress {
                task_id: id,
                status: TaskStatus::Cancelled,
                progress: 0,
                message: Some("Task cancelled".into()),
                result: None,
                error: None,
                parent_task_id: task.parent_task_id,
                data: None,
            };
            // Broadcast via the progress sender
            let _ = state.task_queue().subscribe_all();
            let tx = state.task_queue().progress_sender();
            let _ = tx.broadcast_sender().send(update);
        }
        TaskStatus::Running => {
            // Signal cancellation via token; the worker will handle status transition
            state.task_queue().cancellation_registry().cancel(id);
            // Cancel pending children immediately
            TaskRepository::cancel_pending_children(state.db_pool(), id).await?;
        }
        _ => {
            // Completed/Failed/Cancelled — already handled above
        }
    }

    // Return refreshed task state
    let updated_task = TaskRepository::get_by_id(state.db_pool(), id).await?;
    let mut resp: TaskResponse = updated_task.into();
    let summary = TaskRepository::child_summary(state.db_pool(), id).await?;
    if summary.total > 0 {
        resp.children_summary = Some(summary.into());
    }
    Ok(Json(resp))
}
