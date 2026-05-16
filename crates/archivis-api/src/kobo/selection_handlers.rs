use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::{BookFile, BookFormat, KoboSyncSelection};
use archivis_db::{BookFileRepository, BookRepository, KoboSyncSelectionRepository};

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::kobo::types::{KoboSyncStateResponse, UpsertSelectionRequest};
use crate::state::AppState;

/// `PUT /api/books/{book_id}/kobo-sync` — opt the book into the caller's
/// Kobo Sync, optionally selecting a specific EPUB file.
#[utoipa::path(
    put,
    path = "/api/books/{book_id}/kobo-sync",
    tag = "kobo",
    params(("book_id" = Uuid, Path, description = "Book ID")),
    request_body = UpsertSelectionRequest,
    responses(
        (status = 200, description = "Selection updated", body = KoboSyncStateResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn upsert_selection(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(book_id): Path<Uuid>,
    Json(body): Json<UpsertSelectionRequest>,
) -> Result<Json<KoboSyncStateResponse>, ApiError> {
    if !crate::kobo::sync_enabled(&state) {
        return Err(crate::kobo::sync_disabled_error());
    }

    if !body.enabled {
        // The first UI never sends enabled=false; reject it explicitly so a
        // future "disable via PUT" path is added by intent, not by accident.
        return Err(ApiError::Validation(
            "use DELETE to disable Kobo Sync for this book".into(),
        ));
    }

    let pool = state.db_pool();
    // Confirm the book exists.
    BookRepository::get_by_id(pool, book_id).await?;

    let files = BookFileRepository::get_by_book_id(pool, book_id).await?;
    let mut epubs: Vec<BookFile> = files
        .into_iter()
        .filter(|f| f.format == BookFormat::Epub)
        .collect();
    if epubs.is_empty() {
        return Err(ApiError::Validation(
            "book has no EPUB file; cannot enable Kobo Sync".into(),
        ));
    }

    // Deterministic order: oldest first, then by id.
    epubs.sort_by(|a, b| a.added_at.cmp(&b.added_at).then_with(|| a.id.cmp(&b.id)));

    let selected_id = if let Some(explicit) = body.book_file_id {
        let found = epubs.iter().find(|f| f.id == explicit).ok_or_else(|| {
            ApiError::Validation("requested file is not an EPUB on this book".into())
        })?;
        found.id
    } else {
        epubs[0].id
    };

    let selection =
        KoboSyncSelectionRepository::upsert(pool, user.id, book_id, Some(selected_id)).await?;

    Ok(Json(build_state_response(&epubs, Some(&selection))))
}

/// `DELETE /api/books/{book_id}/kobo-sync` — disable Kobo Sync for the book.
#[utoipa::path(
    delete,
    path = "/api/books/{book_id}/kobo-sync",
    tag = "kobo",
    params(("book_id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 204, description = "Selection removed"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_selection(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(book_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !crate::kobo::sync_enabled(&state) {
        return Err(crate::kobo::sync_disabled_error());
    }

    KoboSyncSelectionRepository::delete(state.db_pool(), user.id, book_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Build the per-user Kobo Sync state block embedded in the book detail
/// response.
pub async fn current_state_for_book(
    state: &AppState,
    user_id: Uuid,
    book_id: Uuid,
) -> Result<KoboSyncStateResponse, ApiError> {
    let pool = state.db_pool();
    let files = BookFileRepository::get_by_book_id(pool, book_id).await?;
    let mut epubs: Vec<BookFile> = files
        .into_iter()
        .filter(|f| f.format == BookFormat::Epub)
        .collect();
    epubs.sort_by(|a, b| a.added_at.cmp(&b.added_at).then_with(|| a.id.cmp(&b.id)));

    let selection = KoboSyncSelectionRepository::find(pool, user_id, book_id).await?;
    Ok(build_state_response(&epubs, selection.as_ref()))
}

fn build_state_response(
    epubs: &[BookFile],
    selection: Option<&KoboSyncSelection>,
) -> KoboSyncStateResponse {
    let eligible_file_ids: Vec<Uuid> = epubs.iter().map(|f| f.id).collect();

    let Some(sel) = selection else {
        return KoboSyncStateResponse {
            enabled: false,
            selected_book_file_id: None,
            eligible_file_ids,
            stale: false,
            reason: None,
        };
    };

    let selected_book_file_id = sel.selected_book_file_id;
    let stale = selected_book_file_id.is_none_or(|id| !eligible_file_ids.contains(&id));
    let reason = if stale {
        Some(if selected_book_file_id.is_none() {
            "selected file was deleted".into()
        } else {
            "selected file is no longer an EPUB on this book".into()
        })
    } else {
        None
    };

    KoboSyncStateResponse {
        enabled: true,
        selected_book_file_id,
        eligible_file_ids,
        stale,
        reason,
    }
}
