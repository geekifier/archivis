use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::{CandidateStatus, IdentificationCandidate, TaskType};
use archivis_db::{BookRepository, CandidateRepository};
use archivis_metadata::ProviderMetadata;

use crate::auth::AuthUser;
use crate::books::types::BookDetail;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    ApplyCandidateBody, BatchIdentifyRequest, CandidateResponse, IdentifyAllRequest,
    IdentifyAllResponse, IdentifyResponse, SeriesInfo,
};

/// POST /api/books/{id}/identify -- trigger identification for a single book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/identify",
    tag = "identify",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 202, description = "Identification task enqueued", body = IdentifyResponse),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn identify_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<IdentifyResponse>), ApiError> {
    // Verify book exists
    BookRepository::get_by_id(state.db_pool(), id).await?;

    let payload = serde_json::json!({
        "book_id": id.to_string(),
    });

    let task_id = state
        .task_queue()
        .enqueue(TaskType::IdentifyBook, payload)
        .await
        .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

    Ok((StatusCode::ACCEPTED, Json(IdentifyResponse { task_id })))
}

/// GET /api/books/{id}/candidates -- list identification candidates for a book.
#[utoipa::path(
    get,
    path = "/api/books/{id}/candidates",
    tag = "identify",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "List of identification candidates", body = Vec<CandidateResponse>),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_candidates(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<CandidateResponse>>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    let candidates = CandidateRepository::list_by_book(pool, id).await?;

    let responses: Vec<CandidateResponse> =
        candidates.into_iter().map(candidate_to_response).collect();

    Ok(Json(responses))
}

/// `POST /api/books/{id}/candidates/{candidate_id}/apply` -- apply a candidate to a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/candidates/{candidate_id}/apply",
    tag = "identify",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("candidate_id" = Uuid, Path, description = "Candidate ID"),
    ),
    request_body(content = ApplyCandidateBody, description = "Optional field exclusions", content_type = "application/json"),
    responses(
        (status = 200, description = "Candidate applied, book updated", body = BookDetail),
        (status = 404, description = "Book or candidate not found"),
        (status = 409, description = "Candidate already applied or rejected"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn apply_candidate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, candidate_id)): Path<(Uuid, Uuid)>,
    body: Option<Json<ApplyCandidateBody>>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Check candidate exists and status
    let candidate = CandidateRepository::get_by_id(pool, candidate_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("candidate not found: {candidate_id}")))?;

    if candidate.book_id != book_id {
        return Err(ApiError::NotFound(format!(
            "candidate {candidate_id} does not belong to book {book_id}"
        )));
    }

    if candidate.status != CandidateStatus::Pending {
        return Err(ApiError::Core(archivis_core::errors::ArchivisError::Db(
            archivis_core::errors::DbError::Constraint(format!(
                "candidate already {}",
                candidate.status
            )),
        )));
    }

    // Build exclusion set from optional body
    let exclude_fields: HashSet<String> = body
        .map(|b| b.0.exclude_fields.into_iter().collect())
        .unwrap_or_default();

    // Apply the candidate via the identification service
    state
        .identify_service()
        .apply_candidate(book_id, candidate_id, &exclude_fields)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("another candidate is already applied") {
                ApiError::Core(archivis_core::errors::ArchivisError::Db(
                    archivis_core::errors::DbError::Constraint(msg),
                ))
            } else {
                ApiError::Internal(format!("failed to apply candidate: {e}"))
            }
        })?;

    let bwr = BookRepository::get_with_relations(pool, book_id).await?;
    Ok(Json(bwr.into()))
}

/// `POST /api/books/{id}/candidates/{candidate_id}/reject` -- reject a candidate.
#[utoipa::path(
    post,
    path = "/api/books/{id}/candidates/{candidate_id}/reject",
    tag = "identify",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("candidate_id" = Uuid, Path, description = "Candidate ID"),
    ),
    responses(
        (status = 204, description = "Candidate rejected"),
        (status = 404, description = "Book or candidate not found"),
        (status = 409, description = "Candidate already applied or rejected"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn reject_candidate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, candidate_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Check candidate exists and status
    let candidate = CandidateRepository::get_by_id(pool, candidate_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("candidate not found: {candidate_id}")))?;

    if candidate.book_id != book_id {
        return Err(ApiError::NotFound(format!(
            "candidate {candidate_id} does not belong to book {book_id}"
        )));
    }

    if candidate.status != CandidateStatus::Pending {
        return Err(ApiError::Core(archivis_core::errors::ArchivisError::Db(
            archivis_core::errors::DbError::Constraint(format!(
                "candidate already {}",
                candidate.status
            )),
        )));
    }

    CandidateRepository::update_status(pool, candidate_id, CandidateStatus::Rejected).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/books/{id}/candidates/{candidate_id}/undo` -- undo an applied candidate.
#[utoipa::path(
    post,
    path = "/api/books/{id}/candidates/{candidate_id}/undo",
    tag = "identify",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("candidate_id" = Uuid, Path, description = "Candidate ID"),
    ),
    responses(
        (status = 200, description = "Candidate application undone, book updated", body = BookDetail),
        (status = 404, description = "Book or candidate not found"),
        (status = 409, description = "Candidate is not in applied state"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn undo_candidate(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, candidate_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Check candidate exists and belongs to this book
    let candidate = CandidateRepository::get_by_id(pool, candidate_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("candidate not found: {candidate_id}")))?;

    if candidate.book_id != book_id {
        return Err(ApiError::NotFound(format!(
            "candidate {candidate_id} does not belong to book {book_id}"
        )));
    }

    if candidate.status != CandidateStatus::Applied {
        return Err(ApiError::Core(archivis_core::errors::ArchivisError::Db(
            archivis_core::errors::DbError::Constraint(format!(
                "candidate is {}, can only undo applied candidates",
                candidate.status
            )),
        )));
    }

    // Undo the candidate via the identification service
    state
        .identify_service()
        .undo_candidate(book_id, candidate_id)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to undo candidate: {e}")))?;

    let bwr = BookRepository::get_with_relations(pool, book_id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/identify/batch -- trigger identification for multiple books.
#[utoipa::path(
    post,
    path = "/api/identify/batch",
    tag = "identify",
    request_body = BatchIdentifyRequest,
    responses(
        (status = 202, description = "Identification tasks enqueued", body = Vec<IdentifyResponse>),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_identify(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchIdentifyRequest>,
) -> Result<(StatusCode, Json<Vec<IdentifyResponse>>), ApiError> {
    let mut responses = Vec::with_capacity(body.book_ids.len());

    for book_id in &body.book_ids {
        let payload = serde_json::json!({
            "book_id": book_id.to_string(),
        });

        let task_id = state
            .task_queue()
            .enqueue(TaskType::IdentifyBook, payload)
            .await
            .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

        responses.push(IdentifyResponse { task_id });
    }

    Ok((StatusCode::ACCEPTED, Json(responses)))
}

/// POST /api/identify/all -- identify all unidentified/needs-review books.
#[utoipa::path(
    post,
    path = "/api/identify/all",
    tag = "identify",
    request_body = IdentifyAllRequest,
    responses(
        (status = 202, description = "Identification tasks enqueued", body = IdentifyAllResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn identify_all(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<IdentifyAllRequest>,
) -> Result<(StatusCode, Json<IdentifyAllResponse>), ApiError> {
    let pool = state.db_pool();
    let max_books = body.max_books.unwrap_or(100);

    let books = BookRepository::list_needing_identification(pool, 0.6, max_books).await?;

    let mut task_ids = Vec::with_capacity(books.len());

    for book in &books {
        let payload = serde_json::json!({
            "book_id": book.id.to_string(),
        });

        let task_id = state
            .task_queue()
            .enqueue(TaskType::IdentifyBook, payload)
            .await
            .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

        task_ids.push(task_id);
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(IdentifyAllResponse {
            count: books.len(),
            task_ids,
        }),
    ))
}

/// Convert an `IdentificationCandidate` to a `CandidateResponse`.
fn candidate_to_response(candidate: IdentificationCandidate) -> CandidateResponse {
    // Try to deserialize the stored JSON into ProviderMetadata for structured fields
    let provider_meta: Option<ProviderMetadata> = serde_json::from_value(candidate.metadata).ok();

    let (
        title,
        subtitle,
        authors,
        description,
        publisher,
        publication_date,
        isbn,
        series,
        cover_url,
    ) = provider_meta.as_ref().map_or_else(
        || (None, None, vec![], None, None, None, None, None, None),
        |meta| {
            (
                meta.title.clone(),
                meta.subtitle.clone(),
                meta.authors.iter().map(|a| a.name.clone()).collect(),
                meta.description.clone(),
                meta.publisher.clone(),
                meta.publication_date.clone(),
                meta.identifiers
                    .iter()
                    .find(|id| {
                        id.identifier_type == archivis_core::models::IdentifierType::Isbn13
                            || id.identifier_type == archivis_core::models::IdentifierType::Isbn10
                    })
                    .map(|id| id.value.clone()),
                meta.series.as_ref().map(|s| SeriesInfo {
                    name: s.name.clone(),
                    position: s.position,
                }),
                meta.cover_url.clone(),
            )
        },
    );

    CandidateResponse {
        id: candidate.id,
        provider_name: candidate.provider_name,
        score: candidate.score,
        title,
        subtitle,
        authors,
        description,
        publisher,
        publication_date,
        isbn,
        series,
        cover_url,
        match_reasons: candidate.match_reasons,
        status: candidate.status.to_string(),
    }
}
