use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::{CandidateStatus, IdentificationCandidate, TaskType};
use archivis_db::{BookRepository, CandidateRepository};
use archivis_metadata::ProviderMetadata;

use crate::auth::AuthUser;
use crate::books::handlers::stamp_quality_score;
use crate::books::types::BookDetail;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    ApplyCandidateBody, BatchRefreshMetadataRequest, BatchRejectRequest, CandidateAuthor,
    CandidateResponse, RefreshAllMetadataRequest, RefreshAllMetadataResponse,
    RefreshMetadataResponse, SeriesInfo,
};

const VALID_EXCLUDE_FIELDS: &[&str] = &[
    "title",
    "subtitle",
    "description",
    "publication_year",
    "authors",
    "identifiers",
    "series",
    "cover",
    "publisher",
    "language",
    "page_count",
];

fn parse_exclude_fields(body: Option<ApplyCandidateBody>) -> Result<HashSet<String>, ApiError> {
    let fields = body.map_or_else(Vec::new, |b| b.exclude_fields);
    let mut invalid = Vec::new();

    for field in &fields {
        if !VALID_EXCLUDE_FIELDS.contains(&field.as_str()) {
            invalid.push(field.clone());
        }
    }

    if !invalid.is_empty() {
        invalid.sort();
        invalid.dedup();
        return Err(ApiError::Validation(format!(
            "invalid exclude_fields values: {}",
            invalid.join(", ")
        )));
    }

    Ok(fields.into_iter().collect())
}

async fn enqueue_manual_refresh(
    state: &AppState,
    id: Uuid,
) -> Result<(StatusCode, Json<RefreshMetadataResponse>), ApiError> {
    BookRepository::mark_resolution_pending(state.db_pool(), id, "manual_refresh").await?;

    let payload = serde_json::json!({
        "book_id": id.to_string(),
        "manual_refresh": true,
    });

    let task_id = state
        .task_queue()
        .enqueue(TaskType::ResolveBook, payload)
        .await
        .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(RefreshMetadataResponse { task_id }),
    ))
}

/// POST /api/books/{id}/refresh-metadata -- trigger metadata refresh for a single book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/refresh-metadata",
    tag = "resolution",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 202, description = "Metadata refresh task enqueued", body = RefreshMetadataResponse),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn refresh_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<RefreshMetadataResponse>), ApiError> {
    enqueue_manual_refresh(&state, id).await
}

/// GET /api/books/{id}/candidates -- list candidates for the current reviewable run.
#[utoipa::path(
    get,
    path = "/api/books/{id}/candidates",
    tag = "resolution",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "List of metadata resolution candidates", body = Vec<CandidateResponse>),
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
    tag = "resolution",
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
    let exclude_fields = parse_exclude_fields(body.map(|b| b.0))?;

    // Apply the candidate via the resolution service
    state
        .resolve_service()
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
    tag = "resolution",
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

    state
        .resolve_service()
        .reject_candidate(book_id, candidate_id)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to reject candidate: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/books/{id}/candidates/reject` -- batch-reject multiple candidates.
#[utoipa::path(
    post,
    path = "/api/books/{id}/candidates/reject",
    tag = "resolution",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = BatchRejectRequest,
    responses(
        (status = 200, description = "Candidates rejected, book updated", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 409, description = "One or more candidates not in pending state"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_reject_candidates(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(book_id): Path<Uuid>,
    Json(body): Json<BatchRejectRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    state
        .resolve_service()
        .reject_candidates(book_id, &body.candidate_ids)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to reject candidates: {e}")))?;

    let bwr = BookRepository::get_with_relations(pool, book_id).await?;
    Ok(Json(bwr.into()))
}

/// `POST /api/books/{id}/trust-metadata` -- trust current metadata as correct.
#[utoipa::path(
    post,
    path = "/api/books/{id}/trust-metadata",
    tag = "resolution",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Metadata trusted", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 409, description = "Book has a metadata refresh in progress"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn trust_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(book_id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    let ok = state
        .resolve_service()
        .trust_metadata(book_id)
        .await
        .map_err(|e| ApiError::Internal(format!("trust failed: {e}")))?;
    if !ok {
        return Err(ApiError::Conflict(
            "Cannot trust metadata while a refresh is in progress".into(),
        ));
    }

    let mut bwr = BookRepository::get_with_relations(pool, book_id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// `POST /api/books/{id}/untrust-metadata` -- remove trust from current metadata.
#[utoipa::path(
    post,
    path = "/api/books/{id}/untrust-metadata",
    tag = "resolution",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Metadata untrusted", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 409, description = "Book has a metadata refresh in progress"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn untrust_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(book_id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    let result = state
        .resolve_service()
        .untrust_metadata(book_id)
        .await
        .map_err(|e| ApiError::Internal(format!("untrust failed: {e}")))?;
    if result.is_none() {
        return Err(ApiError::Conflict(
            "Cannot untrust metadata while a refresh is in progress".into(),
        ));
    }

    let mut bwr = BookRepository::get_with_relations(pool, book_id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// `POST /api/books/{id}/candidates/{candidate_id}/undo` -- undo an applied candidate.
#[utoipa::path(
    post,
    path = "/api/books/{id}/candidates/{candidate_id}/undo",
    tag = "resolution",
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

    // Undo the candidate via the resolution service
    state
        .resolve_service()
        .undo_candidate(book_id, candidate_id)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to undo candidate: {e}")))?;

    let bwr = BookRepository::get_with_relations(pool, book_id).await?;
    Ok(Json(bwr.into()))
}

async fn enqueue_batch_manual_refresh(
    state: &AppState,
    body: BatchRefreshMetadataRequest,
) -> Result<(StatusCode, Json<Vec<RefreshMetadataResponse>>), ApiError> {
    let mut responses = Vec::with_capacity(body.book_ids.len());

    for book_id in &body.book_ids {
        BookRepository::mark_resolution_pending(state.db_pool(), *book_id, "manual_refresh")
            .await?;

        let payload = serde_json::json!({
            "book_id": book_id.to_string(),
            "manual_refresh": true,
        });

        let task_id = state
            .task_queue()
            .enqueue(TaskType::ResolveBook, payload)
            .await
            .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

        responses.push(RefreshMetadataResponse { task_id });
    }

    Ok((StatusCode::ACCEPTED, Json(responses)))
}

/// POST /api/books/refresh-metadata/batch -- trigger metadata refresh for multiple books.
#[utoipa::path(
    post,
    path = "/api/books/refresh-metadata/batch",
    tag = "resolution",
    request_body = BatchRefreshMetadataRequest,
    responses(
        (status = 202, description = "Metadata refresh tasks enqueued", body = Vec<RefreshMetadataResponse>),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_refresh_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchRefreshMetadataRequest>,
) -> Result<(StatusCode, Json<Vec<RefreshMetadataResponse>>), ApiError> {
    enqueue_batch_manual_refresh(&state, body).await
}

/// POST /api/books/refresh-metadata/all -- enqueue refresh for pending resolution backlog.
#[utoipa::path(
    post,
    path = "/api/books/refresh-metadata/all",
    tag = "resolution",
    request_body = RefreshAllMetadataRequest,
    responses(
        (status = 202, description = "Metadata refresh tasks enqueued", body = RefreshAllMetadataResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn refresh_all_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<RefreshAllMetadataRequest>,
) -> Result<(StatusCode, Json<RefreshAllMetadataResponse>), ApiError> {
    let max_books = body.max_books.unwrap_or(100);
    let response = enqueue_resolution_backlog(&state, max_books).await?;

    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn enqueue_resolution_backlog(
    state: &AppState,
    max_books: i64,
) -> Result<RefreshAllMetadataResponse, ApiError> {
    let books = BookRepository::list_pending_resolution(state.db_pool(), max_books).await?;
    let mut task_ids = Vec::with_capacity(books.len());

    for book in &books {
        let payload = serde_json::json!({
            "book_id": book.id.to_string(),
        });

        let task_id = state
            .task_queue()
            .enqueue(TaskType::ResolveBook, payload)
            .await
            .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;
        task_ids.push(task_id);
    }

    Ok(RefreshAllMetadataResponse {
        count: books.len(),
        task_ids,
    })
}

/// Convert a persisted resolution candidate into an API response.
fn candidate_to_response(candidate: IdentificationCandidate) -> CandidateResponse {
    let provider_meta: Option<ProviderMetadata> = serde_json::from_value(candidate.metadata).ok();
    let meta = provider_meta.as_ref();

    CandidateResponse {
        id: candidate.id,
        run_id: candidate.run_id,
        provider_name: candidate.provider_name,
        score: candidate.score,
        title: meta.and_then(|m| m.title.clone()),
        subtitle: meta.and_then(|m| m.subtitle.clone()),
        authors: meta.map_or_else(Vec::new, |m| {
            m.authors
                .iter()
                .map(|a| CandidateAuthor {
                    name: a.name.clone(),
                    role: a.role.clone(),
                })
                .collect()
        }),
        description: meta.and_then(|m| m.description.clone()),
        publisher: meta.and_then(|m| m.publisher.clone()),
        publication_year: meta.and_then(|m| m.publication_year),
        language: meta.and_then(|m| m.language.clone()),
        language_label: meta
            .and_then(|m| m.language.as_deref())
            .and_then(archivis_core::language::language_label)
            .map(String::from),
        page_count: meta.and_then(|m| m.page_count),
        isbn: meta.and_then(|m| {
            m.identifiers
                .iter()
                .find(|id| {
                    id.identifier_type == archivis_core::models::IdentifierType::Isbn13
                        || id.identifier_type == archivis_core::models::IdentifierType::Isbn10
                })
                .map(|id| id.value.clone())
        }),
        series: meta.and_then(|m| {
            m.series.as_ref().map(|s| SeriesInfo {
                name: s.name.clone(),
                position: s.position,
            })
        }),
        cover_url: meta.and_then(|m| m.cover_url.clone()),
        match_reasons: candidate.match_reasons,
        disputes: candidate.disputes,
        status: candidate.status.to_string(),
        tier: candidate.tier,
        is_composite: meta.is_some_and(|m| !m.merged_from.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use archivis_auth::{AuthService, LocalAuthAdapter};
    use archivis_core::models::{
        Book, IdentificationCandidate, MetadataStatus, ResolutionOutcome, ResolutionRun,
        ResolutionState, TaskType, User, UserRole,
    };
    use archivis_db::{
        create_pool, run_migrations, CandidateRepository, ResolutionRunRepository, TaskRepository,
    };
    use archivis_metadata::{MetadataResolver, ProviderRegistry};
    use archivis_storage::local::LocalStorage;
    use archivis_tasks::merge::MergeService;
    use archivis_tasks::queue::TaskQueue;
    use archivis_tasks::resolve::ResolutionService;
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    use crate::settings::service::ConfigService;
    use crate::state::ApiConfig;

    struct TestSettings;

    impl archivis_core::settings::SettingsReader for TestSettings {
        fn get_setting(&self, _key: &str) -> Option<serde_json::Value> {
            None
        }
    }

    async fn test_state(tmp: &TempDir) -> AppState {
        let db_path = tmp.path().join("test.db");
        let storage_dir = tmp.path().join("books");
        let db_pool = create_pool(&db_path).await.unwrap();
        run_migrations(&db_pool).await.unwrap();

        let storage = LocalStorage::new(&storage_dir).await.unwrap();
        let auth_adapter = LocalAuthAdapter::new(db_pool.clone());
        let auth_service = AuthService::new(db_pool.clone(), auth_adapter);
        let (task_queue, mut rx) = TaskQueue::new(db_pool.clone());
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let provider_registry = Arc::new(ProviderRegistry::new());
        let resolver = Arc::new(MetadataResolver::new(
            Arc::clone(&provider_registry),
            Arc::new(TestSettings),
        ));
        let resolve_service = Arc::new(ResolutionService::new(
            db_pool.clone(),
            resolver,
            storage.clone(),
            tmp.path().to_path_buf(),
        ));
        let merge_service = Arc::new(MergeService::new(
            db_pool.clone(),
            storage.clone(),
            tmp.path().to_path_buf(),
        ));
        let config_service = Arc::new(ConfigService::new(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            db_pool.clone(),
        ));

        AppState::new(
            db_pool,
            Arc::new(task_queue),
            auth_service,
            storage,
            provider_registry,
            resolve_service,
            merge_service,
            ApiConfig {
                data_dir: tmp.path().to_path_buf(),
                frontend_dir: None,
            },
            config_service,
            None,
            None,
            [0u8; 32],
        )
    }

    fn auth_user() -> AuthUser {
        AuthUser(User::new(
            "tester".into(),
            "password-hash".into(),
            UserRole::Admin,
        ))
    }

    #[tokio::test]
    async fn enqueue_resolution_backlog_uses_resolution_lifecycle_queue() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let base = Utc::now();

        let mut earliest = Book::new("Earliest");
        earliest.metadata_status = MetadataStatus::Identified;
        earliest.ingest_quality_score = 1.0;
        earliest.resolution_requested_at = base - Duration::hours(4);
        BookRepository::create(state.db_pool(), &earliest)
            .await
            .unwrap();

        let mut locked = Book::new("Locked");
        locked.resolution_requested_at = base - Duration::hours(5);
        locked.metadata_locked = true;
        BookRepository::create(state.db_pool(), &locked)
            .await
            .unwrap();

        let mut done = Book::new("Done");
        done.resolution_requested_at = base - Duration::hours(6);
        done.resolution_state = ResolutionState::Done;
        done.metadata_status = MetadataStatus::Unidentified;
        done.ingest_quality_score = 0.0;
        BookRepository::create(state.db_pool(), &done)
            .await
            .unwrap();

        let mut latest = Book::new("Latest");
        latest.resolution_requested_at = base - Duration::hours(1);
        BookRepository::create(state.db_pool(), &latest)
            .await
            .unwrap();

        let response = enqueue_resolution_backlog(&state, 10).await.unwrap();

        assert_eq!(response.count, 2);

        let first_task = TaskRepository::get_by_id(state.db_pool(), response.task_ids[0])
            .await
            .unwrap();
        let second_task = TaskRepository::get_by_id(state.db_pool(), response.task_ids[1])
            .await
            .unwrap();

        assert_eq!(first_task.task_type, TaskType::ResolveBook);
        assert_eq!(second_task.task_type, TaskType::ResolveBook);
        assert_eq!(first_task.payload["book_id"], earliest.id.to_string());
        assert_eq!(second_task.payload["book_id"], latest.id.to_string());
    }

    #[tokio::test]
    async fn refresh_metadata_allows_manual_refresh_for_locked_books() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut locked = Book::new("Locked");
        locked.metadata_locked = true;
        BookRepository::create(state.db_pool(), &locked)
            .await
            .unwrap();

        let (status, Json(response)) =
            refresh_metadata(State(state.clone()), auth_user(), Path(locked.id))
                .await
                .unwrap();

        assert_eq!(status, StatusCode::ACCEPTED);

        let queued = TaskRepository::get_by_id(state.db_pool(), response.task_id)
            .await
            .unwrap();
        assert_eq!(queued.task_type, TaskType::ResolveBook);
        assert_eq!(queued.payload["book_id"], locked.id.to_string());
        assert_eq!(queued.payload["manual_refresh"], true);

        let refreshed = BookRepository::get_by_id(state.db_pool(), locked.id)
            .await
            .unwrap();
        assert_eq!(refreshed.resolution_state, ResolutionState::Pending);
        assert_eq!(
            refreshed.resolution_requested_reason.as_deref(),
            Some("manual_refresh")
        );
    }

    #[test]
    fn parse_exclude_fields_accepts_valid_values() {
        let parsed = parse_exclude_fields(Some(ApplyCandidateBody {
            exclude_fields: vec!["cover".into(), "title".into(), "series".into()],
        }))
        .unwrap();

        assert!(parsed.contains("cover"));
        assert!(parsed.contains("title"));
        assert!(parsed.contains("series"));
    }

    #[test]
    fn candidate_to_response_includes_disputes() {
        let mut candidate = IdentificationCandidate::new(
            Uuid::new_v4(),
            "test_provider",
            0.91,
            serde_json::json!({
                "provider_name": "test_provider",
                "title": "Dune",
                "authors": [],
                "identifiers": [],
                "subjects": [],
                "confidence": 0.91
            }),
            vec!["title_match".into()],
        );
        candidate.disputes = vec!["title_conflict".into(), "author_conflict".into()];

        let response = candidate_to_response(candidate);

        assert_eq!(
            response.disputes,
            vec!["title_conflict".to_string(), "author_conflict".to_string()]
        );
    }

    #[test]
    fn parse_exclude_fields_rejects_invalid_values() {
        let err = parse_exclude_fields(Some(ApplyCandidateBody {
            exclude_fields: vec!["cover".into(), "covre".into(), "unknown".into()],
        }))
        .unwrap_err();

        match err {
            ApiError::Validation(msg) => {
                assert!(msg.contains("covre"));
                assert!(msg.contains("unknown"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_candidates_returns_only_latest_run_candidates() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let book = Book::new("Candidate History");
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let run1 = ResolutionRun::new(book.id, "first", serde_json::json!({"title":"old"}));
        ResolutionRunRepository::create(state.db_pool(), &run1)
            .await
            .unwrap();
        let mut older = IdentificationCandidate::new(
            book.id,
            "provider_old",
            0.4,
            serde_json::json!({"provider_name":"provider_old","title":"Old"}),
            vec![],
        );
        older.run_id = Some(run1.id);
        CandidateRepository::create(state.db_pool(), &older)
            .await
            .unwrap();

        let run2 = ResolutionRun::new(book.id, "second", serde_json::json!({"title":"new"}));
        ResolutionRunRepository::create(state.db_pool(), &run2)
            .await
            .unwrap();
        let mut latest = IdentificationCandidate::new(
            book.id,
            "provider_new",
            0.9,
            serde_json::json!({"provider_name":"provider_new","title":"New"}),
            vec!["Exact".into()],
        );
        latest.run_id = Some(run2.id);
        CandidateRepository::create(state.db_pool(), &latest)
            .await
            .unwrap();

        let Json(candidates) = list_candidates(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, latest.id);
        assert_eq!(candidates[0].provider_name, "provider_new");
    }

    #[tokio::test]
    async fn list_candidates_surfaces_applied_for_confirmed_run() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let book = Book::new("Auto-applied");
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let mut run = ResolutionRun::new(
            book.id,
            "import",
            serde_json::json!({"title":"Auto-applied"}),
        );
        run.state = archivis_core::models::ResolutionRunState::Done;
        run.outcome = Some(ResolutionOutcome::Confirmed);
        run.finished_at = Some(Utc::now());
        ResolutionRunRepository::create(state.db_pool(), &run)
            .await
            .unwrap();

        let mut applied = IdentificationCandidate::new(
            book.id,
            "provider",
            0.95,
            serde_json::json!({"provider_name":"provider","title":"Auto-applied"}),
            vec!["isbn".into()],
        );
        applied.run_id = Some(run.id);
        applied.status = CandidateStatus::Applied;
        CandidateRepository::create(state.db_pool(), &applied)
            .await
            .unwrap();

        let mut rejected = IdentificationCandidate::new(
            book.id,
            "provider_b",
            0.6,
            serde_json::json!({"provider_name":"provider_b","title":"Auto-applied"}),
            vec![],
        );
        rejected.run_id = Some(run.id);
        rejected.status = CandidateStatus::Rejected;
        CandidateRepository::create(state.db_pool(), &rejected)
            .await
            .unwrap();

        let Json(candidates) = list_candidates(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();

        // Applied candidate is surfaced (for undo); rejected is not
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, applied.id);
        assert_eq!(candidates[0].status, "applied");
    }

    #[test]
    fn valid_exclude_fields_covers_all_merge_fields() {
        // If a field is handled by `merge_book_fields()`, it must appear in
        // `VALID_EXCLUDE_FIELDS` so the user can exclude it via the API.
        let expected: HashSet<&str> = [
            "title",
            "subtitle",
            "description",
            "publication_year",
            "authors",
            "identifiers",
            "series",
            "cover",
            "publisher",
            "language",
            "page_count",
        ]
        .into_iter()
        .collect();
        let actual: HashSet<&str> = VALID_EXCLUDE_FIELDS.iter().copied().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn candidate_to_response_extracts_language_and_page_count() {
        let candidate = IdentificationCandidate::new(
            Uuid::new_v4(),
            "test_provider",
            0.88,
            serde_json::json!({
                "provider_name": "test_provider",
                "title": "Test Book",
                "authors": [],
                "identifiers": [],
                "subjects": [],
                "confidence": 0.88,
                "language": "en",
                "page_count": 320
            }),
            vec!["isbn_match".into()],
        );

        let response = candidate_to_response(candidate);

        assert_eq!(response.language.as_deref(), Some("en"));
        assert_eq!(
            response.language_label.as_deref(),
            Some("English"),
            "language_label should be computed from the ISO code"
        );
        assert_eq!(response.page_count, Some(320));
    }
}
