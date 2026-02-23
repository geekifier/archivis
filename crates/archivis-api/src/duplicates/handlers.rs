use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::DuplicateLink;
use archivis_db::{BookRepository, DuplicateRepository};
use archivis_tasks::merge::{MergeOptions, MergePreference};

use crate::auth::AuthUser;
use crate::books::types::BookSummary;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    DuplicateCountResponse, DuplicateLinkResponse, DuplicateListParams, FlagDuplicateRequest,
    MergeRequest, PaginatedDuplicates,
};

/// Build a `DuplicateLinkResponse` by enriching a `DuplicateLink` with book summaries.
async fn enrich_link(
    pool: &archivis_db::DbPool,
    link: &DuplicateLink,
) -> Result<DuplicateLinkResponse, ApiError> {
    let book_a = BookRepository::get_with_relations(pool, link.book_id_a).await?;
    let book_b = BookRepository::get_with_relations(pool, link.book_id_b).await?;

    let mut summary_a = BookSummary::from(book_a.book);
    summary_a.authors = Some(book_a.authors.into_iter().map(Into::into).collect());

    let mut summary_b = BookSummary::from(book_b.book);
    summary_b.authors = Some(book_b.authors.into_iter().map(Into::into).collect());

    Ok(DuplicateLinkResponse {
        id: link.id,
        book_a: summary_a,
        book_b: summary_b,
        detection_method: link.detection_method.clone(),
        confidence: link.confidence,
        status: link.status.to_string(),
        created_at: link.created_at,
    })
}

/// GET /api/duplicates -- list pending duplicate pairs (paginated).
#[utoipa::path(
    get,
    path = "/api/duplicates",
    tag = "duplicates",
    params(DuplicateListParams),
    responses(
        (status = 200, description = "Paginated duplicate links", body = PaginatedDuplicates),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_duplicates(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<DuplicateListParams>,
) -> Result<Json<PaginatedDuplicates>, ApiError> {
    let pool = state.db_pool();
    let per_page = params.per_page.unwrap_or(25).min(100);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let total = DuplicateRepository::count_pending(pool).await?;
    let links =
        DuplicateRepository::list_pending(pool, i64::from(per_page), i64::from(offset)).await?;

    let mut items = Vec::with_capacity(links.len());
    for link in &links {
        items.push(enrich_link(pool, link).await?);
    }

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let total_u32 = total as u32;
    let total_pages = total_u32.div_ceil(per_page);

    Ok(Json(PaginatedDuplicates {
        items,
        total: total_u32,
        page,
        per_page,
        total_pages,
    }))
}

/// GET /api/duplicates/count -- count of pending duplicates.
#[utoipa::path(
    get,
    path = "/api/duplicates/count",
    tag = "duplicates",
    responses(
        (status = 200, description = "Pending duplicate count", body = DuplicateCountResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn count_duplicates(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<DuplicateCountResponse>, ApiError> {
    let count = DuplicateRepository::count_pending(state.db_pool()).await?;
    Ok(Json(DuplicateCountResponse { count }))
}

/// GET /api/duplicates/{id} -- get duplicate link detail.
#[utoipa::path(
    get,
    path = "/api/duplicates/{id}",
    tag = "duplicates",
    params(("id" = Uuid, Path, description = "Duplicate link ID")),
    responses(
        (status = 200, description = "Duplicate link detail", body = DuplicateLinkResponse),
        (status = 404, description = "Duplicate link not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_duplicate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<DuplicateLinkResponse>, ApiError> {
    let pool = state.db_pool();
    let link = DuplicateRepository::get_by_id(pool, id).await?;
    let response = enrich_link(pool, &link).await?;
    Ok(Json(response))
}

/// POST /api/duplicates/{id}/merge -- merge the duplicate pair.
#[utoipa::path(
    post,
    path = "/api/duplicates/{id}/merge",
    tag = "duplicates",
    params(("id" = Uuid, Path, description = "Duplicate link ID")),
    request_body = MergeRequest,
    responses(
        (status = 200, description = "Merged book detail", body = crate::books::types::BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Duplicate link or book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn merge_duplicate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<MergeRequest>,
) -> Result<Json<crate::books::types::BookDetail>, ApiError> {
    let pool = state.db_pool();
    let link = DuplicateRepository::get_by_id(pool, id).await?;

    // Validate that both book IDs match the link's books (in either direction)
    let valid_pair = (body.primary_id == link.book_id_a && body.secondary_id == link.book_id_b)
        || (body.primary_id == link.book_id_b && body.secondary_id == link.book_id_a);

    if !valid_pair {
        return Err(ApiError::Validation(
            "primary_id and secondary_id must match the duplicate link's book IDs".into(),
        ));
    }

    let preference = MergePreference::from_str_or_default(body.prefer_metadata_from.as_deref());
    let options = MergeOptions {
        prefer_metadata_from: preference,
    };

    let merge_service = state.merge_service();
    let result = merge_service
        .merge_books(body.primary_id, body.secondary_id, options)
        .await
        .map_err(|e| match e {
            archivis_tasks::merge::MergeError::BookNotFound(id) => {
                ApiError::NotFound(format!("book not found: {id}"))
            }
            archivis_tasks::merge::MergeError::SameBook => {
                ApiError::Validation("cannot merge a book with itself".into())
            }
            archivis_tasks::merge::MergeError::Database(db_err) => ApiError::from(db_err),
            archivis_tasks::merge::MergeError::Storage(storage_err) => ApiError::from(storage_err),
            archivis_tasks::merge::MergeError::Io(io_err) => {
                ApiError::Internal(format!("I/O error during merge: {io_err}"))
            }
        })?;

    Ok(Json(result.into()))
}

/// POST /api/duplicates/{id}/dismiss -- dismiss a duplicate link.
#[utoipa::path(
    post,
    path = "/api/duplicates/{id}/dismiss",
    tag = "duplicates",
    params(("id" = Uuid, Path, description = "Duplicate link ID")),
    responses(
        (status = 204, description = "Duplicate dismissed"),
        (status = 404, description = "Duplicate link not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn dismiss_duplicate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();
    // Verify the link exists
    DuplicateRepository::get_by_id(pool, id).await?;
    DuplicateRepository::update_status(pool, id, "dismissed").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/books/{id}/duplicates -- manually flag a duplicate.
#[utoipa::path(
    post,
    path = "/api/books/{id}/duplicates",
    tag = "duplicates",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = FlagDuplicateRequest,
    responses(
        (status = 201, description = "Duplicate link created", body = DuplicateLinkResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 409, description = "Duplicate link already exists"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn flag_duplicate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(book_id): Path<Uuid>,
    Json(body): Json<FlagDuplicateRequest>,
) -> Result<(StatusCode, Json<DuplicateLinkResponse>), ApiError> {
    let pool = state.db_pool();

    if book_id == body.other_book_id {
        return Err(ApiError::Validation(
            "cannot flag a book as a duplicate of itself".into(),
        ));
    }

    // Verify both books exist
    BookRepository::get_by_id(pool, book_id).await?;
    BookRepository::get_by_id(pool, body.other_book_id).await?;

    // Check if link already exists
    if DuplicateRepository::exists(pool, book_id, body.other_book_id).await? {
        return Err(ApiError::Validation(
            "a duplicate link already exists between these books".into(),
        ));
    }

    // Create the link
    let link = DuplicateLink::new(book_id, body.other_book_id, "user", 1.0);
    DuplicateRepository::create(pool, &link).await?;

    let response = enrich_link(pool, &link).await?;
    Ok((StatusCode::CREATED, Json(response)))
}
