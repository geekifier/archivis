use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use uuid::Uuid;

use archivis_core::models::TaskStatus;
use archivis_db::TaskRepository;

use crate::errors::ApiError;
use crate::state::AppState;

/// Build an SSE `Event` from a task status and JSON data.
fn progress_to_event(status: TaskStatus, data: serde_json::Value) -> Option<Event> {
    let event_name = match status {
        TaskStatus::Completed => "task:complete",
        TaskStatus::Failed => "task:error",
        TaskStatus::Cancelled => "task:cancelled",
        _ => "task:progress",
    };
    Event::default().event(event_name).json_data(data).ok()
}

/// GET /api/tasks/{id}/progress -- SSE stream for a specific task's progress.
///
/// If the task is already in a terminal state (completed/failed/cancelled), a single
/// final event is sent and the stream ends.  Otherwise, the stream subscribes
/// to progress updates and forwards events until the task reaches a terminal
/// state.
#[utoipa::path(
    get,
    path = "/api/tasks/{id}/progress",
    tag = "tasks",
    params(
        ("id" = uuid::Uuid, Path, description = "Task ID"),
    ),
    responses(
        (status = 200, description = "SSE stream of task progress events"),
        (status = 404, description = "Task not found"),
    )
)]
pub async fn task_progress_sse(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    // Verify the task exists and check its current status.
    let task = TaskRepository::get_by_id(state.db_pool(), task_id).await?;

    if task.status.is_terminal() {
        // Task already finished -- send one terminal event and close.
        let data = serde_json::to_value(&task).expect("serialization of Task should not fail");
        let event =
            progress_to_event(task.status, data).expect("event construction should not fail");

        let stream = tokio_stream::once(Ok::<_, Infallible>(event));
        return Ok(Sse::new(stream).into_response());
    }

    // Subscribe to the broadcast channel and filter for this task.
    let rx = state.task_queue().subscribe_all();

    // Track whether a terminal event has been emitted so the stream can end.
    let done = Arc::new(AtomicBool::new(false));

    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        // If we already sent a terminal event, stop producing items.
        if done.load(Ordering::Relaxed) {
            return None;
        }

        let progress = match result {
            Ok(p) if p.task_id == task_id => p,
            _ => return None, // Different task or lagged -- skip.
        };

        let is_terminal = progress.status.is_terminal();

        let data = serde_json::to_value(&progress).ok()?;
        let event = progress_to_event(progress.status, data)?;

        if is_terminal {
            done.store(true, Ordering::Relaxed);
        }

        Some(Ok::<_, Infallible>(event))
    });

    Ok(Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response())
}

/// GET /api/tasks/active -- SSE stream for all active task progress.
///
/// Broadcasts every progress / completion / failure / cancellation event from the task queue.
/// The stream stays open indefinitely until the client disconnects.
#[utoipa::path(
    get,
    path = "/api/tasks/active",
    tag = "tasks",
    responses(
        (status = 200, description = "SSE stream of all active task events"),
    )
)]
pub async fn active_tasks_sse(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.task_queue().subscribe_all();

    let stream = BroadcastStream::new(rx).filter_map(|result| {
        let progress = result.ok()?;
        let data = serde_json::to_value(&progress).ok()?;
        let event = progress_to_event(progress.status, data)?;
        Some(Ok(event))
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    )
}
