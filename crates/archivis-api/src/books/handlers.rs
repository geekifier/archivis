use std::collections::HashSet;

use axum::extract::{Multipart, Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use archivis_formats::sanitize::{sanitize_text, SanitizeOptions};
use archivis_formats::CoverData;

use archivis_core::isbn::validate_isbn;
use archivis_core::models::filter::is_supported_identifier_type;
use archivis_core::models::{
    Book, BulkOperation, BulkTagEntry, BulkTagMode, BulkTaskPayload, BulkUpdateFields,
    FieldProvenance, Identifier, IdentifierType, LibraryFilterState, MetadataSource, TaskType,
};
use archivis_core::search_query::{parse_search_query, QueryClause};

use archivis_db::{
    AuthorRepository, BookFileRepository, BookFilter, BookRepository, CandidateRepository,
    IdentifierRepository, PaginationParams, RelationsBundle, SearchResolver, SeriesRepository,
    SortOrder, TagRepository,
};
use archivis_storage::StorageBackend;
use archivis_tasks::resolve::{
    compute_and_persist_quality_score, persist_recomputed_status, refresh_quality_score_best_effort,
};
use archivis_tasks::workers::{apply_bulk_update_to_book, BulkFieldError};

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    AddIdentifierRequest, BatchAsyncResponse, BatchBookFields, BatchSetTagsRequest,
    BatchSyncResponse, BatchTagMode, BatchUpdateBooksRequest, BatchUpdateError, BookDetail,
    BookListParams, BookSummary, CoverParams, FieldProtectionRequest, IssueSelectionScopeRequest,
    IssueSelectionScopeResponse, OverrideStatusRequest, PaginatedBooks, QueryWarningResponse,
    SelectionSpec, SetBookAuthorsRequest, SetBookSeriesRequest, SetBookTagsRequest,
    UpdateBookRequest, UpdateIdentifierRequest,
};

const USER_EDIT_TRIGGER: &str = "user_edit";
const UNLOCK_TRIGGER: &str = "metadata_unlock";
const PROTECT_FIELDS_TRIGGER: &str = "protect_fields";
const UNPROTECT_FIELDS_TRIGGER: &str = "unprotect_fields";
const PROTECTABLE_FIELDS: &[&str] = &[
    "title",
    "subtitle",
    "description",
    "authors",
    "series",
    "publisher",
    "publication_year",
    "language",
    "page_count",
    "cover",
];

/// Stamp the live quality score into an already-loaded `BookWithRelations`.
pub async fn stamp_quality_score(
    pool: &archivis_db::DbPool,
    bwr: &mut archivis_db::BookWithRelations,
) {
    match compute_and_persist_quality_score(pool, bwr).await {
        Ok(score) => bwr.book.metadata_quality_score = Some(score),
        Err(e) => {
            tracing::warn!(book_id = %bwr.book.id, error = %e, "metadata quality score refresh failed");
        }
    }
}

fn user_field_provenance() -> FieldProvenance {
    FieldProvenance {
        origin: MetadataSource::User,
        protected: true,
        applied_candidate_id: None,
    }
}

fn validate_protection_fields(fields: &[String]) -> Result<(), ApiError> {
    let mut invalid = Vec::new();

    for field in fields {
        if !PROTECTABLE_FIELDS.contains(&field.as_str()) {
            invalid.push(field.clone());
        }
    }

    if invalid.is_empty() {
        return Ok(());
    }

    invalid.sort();
    invalid.dedup();

    Err(ApiError::Validation(format!(
        "invalid fields values: {}",
        invalid.join(", ")
    )))
}

fn provenance_for_field_mut<'a>(
    book: &'a mut Book,
    field: &str,
) -> Result<&'a mut Option<FieldProvenance>, ApiError> {
    match field {
        "title" => Ok(&mut book.metadata_provenance.title),
        "subtitle" => Ok(&mut book.metadata_provenance.subtitle),
        "description" => Ok(&mut book.metadata_provenance.description),
        "authors" => Ok(&mut book.metadata_provenance.authors),
        "series" => Ok(&mut book.metadata_provenance.series),
        "publisher" => Ok(&mut book.metadata_provenance.publisher),
        "publication_year" => Ok(&mut book.metadata_provenance.publication_year),
        "language" => Ok(&mut book.metadata_provenance.language),
        "page_count" => Ok(&mut book.metadata_provenance.page_count),
        "cover" => Ok(&mut book.metadata_provenance.cover),
        _ => Err(ApiError::Validation(format!("invalid field: {field}"))),
    }
}

fn set_field_protection(
    book: &mut Book,
    fields: &[String],
    protected: bool,
) -> Result<bool, ApiError> {
    validate_protection_fields(fields)?;

    let mut changed = false;
    for field in fields {
        let provenance = provenance_for_field_mut(book, field)?;
        match provenance {
            Some(existing) if existing.protected != protected => {
                existing.protected = protected;
                changed = true;
            }
            None if protected => {
                *provenance = Some(user_field_provenance());
                changed = true;
            }
            Some(_) | None => {}
        }
    }

    Ok(changed)
}

async fn invalidate_resolution_for_user_edit(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
) -> Result<(), ApiError> {
    // 1. Stamp resolution_requested_at FIRST so any in-flight resolver
    //    sees the supersession signal before we touch artifacts.
    BookRepository::mark_resolution_pending(pool, book_id, USER_EDIT_TRIGGER).await?;

    // 2. Supersede the active reviewable run and its candidates
    supersede_active_review(pool, book_id).await?;

    // 3. Clear the review baseline (targeted write, no full-row update)
    BookRepository::set_review_baseline(pool, book_id, None, None).await?;

    // 4. Recompute status from the user's new data
    persist_recomputed_status(pool, book_id).await?;
    Ok(())
}

/// Trust-aware invalidation for core identity edits on a trusted book.
///
/// Unlike `invalidate_resolution_for_user_edit`, this does NOT queue the
/// book for automatic resolution.  Instead it normalizes `resolution_state`
/// back to `Done` (or stamps supersession if a manual refresh is in flight).
async fn invalidate_resolution_for_trusted_edit(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
) -> Result<(), ApiError> {
    // 1. Normalize resolution_state (Done, or keep Running + supersede)
    BookRepository::normalize_trusted_resolution_state(pool, book_id, USER_EDIT_TRIGGER).await?;

    // 2. Supersede any active review (defensive — trust should have
    //    already cleared this, but edits can arrive in any order)
    supersede_active_review(pool, book_id).await?;

    // 3. Clear review baselines (defensive)
    BookRepository::set_review_baseline(pool, book_id, None, None).await?;

    // 4. Recompute status (Identified for trusted, corrects stale outcomes)
    persist_recomputed_status(pool, book_id).await?;
    Ok(())
}

/// Shared dispatch: checks `metadata_user_trusted` and calls the appropriate
/// invalidation path.  All backend core-identity edit endpoints use this.
async fn invalidate_resolution_for_core_edit(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
) -> Result<(), ApiError> {
    let book = BookRepository::get_by_id(pool, book_id).await?;
    if book.metadata_user_trusted {
        invalidate_resolution_for_trusted_edit(pool, book_id).await
    } else {
        invalidate_resolution_for_user_edit(pool, book_id).await
    }
}

/// Supersede the currently active reviewable run and its candidates so
/// `persist_recomputed_status` doesn't see stale pending candidates.
async fn supersede_active_review(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
) -> Result<(), ApiError> {
    // `delete_by_book` already finds the latest reviewable run and marks it superseded,
    // or supersedes legacy candidates if no run exists.
    CandidateRepository::delete_by_book(pool, book_id).await?;
    Ok(())
}

async fn invalidate_resolution_for_action(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    trigger: &str,
) -> Result<(), ApiError> {
    BookRepository::mark_resolution_pending(pool, book_id, trigger).await?;
    Ok(())
}

/// GET /api/books — paginated list with sorting, filtering, FTS search.
#[utoipa::path(
    get,
    path = "/api/books",
    tag = "books",
    params(BookListParams),
    responses(
        (status = 200, description = "Paginated book list", body = PaginatedBooks),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<BookListParams>,
) -> Result<Json<PaginatedBooks>, ApiError> {
    // Separate view params from filter state.
    let (filter_state, view) = params.into_filter_state().map_err(ApiError::Validation)?;

    let per_page = view.per_page.unwrap_or(25).min(100);
    let page = view.page.unwrap_or(1).max(1);

    let sort_order = match view.sort_order.as_deref() {
        Some("asc") => SortOrder::Asc,
        _ => SortOrder::Desc,
    };

    let pool = state.db_pool();

    // Parse and resolve DSL operators in `text_query`.
    let (filter_state, filter, warnings) = resolve_dsl(pool, filter_state).await?;

    // Re-check `has_query` after DSL resolution (`text_query` may have changed).
    let has_query = filter_state.text_query.is_some();
    let pagination = PaginationParams {
        page,
        per_page,
        sort_by: PaginationParams::resolve_default_sort(view.sort_by, has_query),
        sort_order,
    };

    let result = BookRepository::list(pool, &pagination, &filter).await?;

    // Parse includes
    let includes: HashSet<&str> = view
        .include
        .as_deref()
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();

    let mut books: PaginatedBooks = result.into();
    books.search_warnings = warnings;

    // Batch-load relations (replaces N+1 `get_with_relations` per book)
    if !includes.is_empty() {
        let book_ids: Vec<Uuid> = books.items.iter().map(|b| b.id).collect();
        let relations = BookRepository::batch_load_relations(pool, &book_ids, &includes).await?;

        for summary in &mut books.items {
            if let Some(bundle) = relations.get(&summary.id) {
                apply_relations(summary, bundle);
            }
        }
    }

    Ok(Json(books))
}

/// Apply a pre-loaded `RelationsBundle` onto a `BookSummary`.
fn apply_relations(summary: &mut BookSummary, bundle: &RelationsBundle) {
    if let Some(ref authors) = bundle.authors {
        summary.authors = Some(authors.iter().cloned().map(Into::into).collect());
    }
    if let Some(ref series) = bundle.series {
        summary.series = Some(series.iter().cloned().map(Into::into).collect());
    }
    if let Some(ref tags) = bundle.tags {
        summary.tags = Some(tags.iter().cloned().map(Into::into).collect());
    }
    if let Some(ref files) = bundle.files {
        summary.files = Some(files.iter().cloned().map(Into::into).collect());
    }
}

/// Parse DSL operators from `text_query`, resolve them against the DB,
/// and merge results back into the filter state.
///
/// Returns the updated filter state, a `BookFilter` ready for DB queries,
/// and any warnings from resolution.
fn maybe_promote_lone_isbn_query(
    filter_state: &mut LibraryFilterState,
    parsed: &archivis_core::search_query::SearchQuery,
) -> bool {
    if filter_state.has_identifier_filter() || !parsed.dropped_empty_fields.is_empty() {
        return false;
    }

    let Some(raw_query) = filter_state.text_query.as_deref() else {
        return false;
    };

    let [QueryClause::Text { negated: false, .. }] = parsed.clauses.as_slice() else {
        return false;
    };

    let validation = validate_isbn(raw_query);
    if !validation.valid {
        return false;
    }

    filter_state.identifier_type = Some("isbn".into());
    filter_state.identifier_value = Some(validation.normalized);
    filter_state.text_query = None;
    true
}

async fn resolve_dsl(
    pool: &archivis_db::DbPool,
    mut filter_state: LibraryFilterState,
) -> Result<(LibraryFilterState, BookFilter, Vec<QueryWarningResponse>), ApiError> {
    let raw_query = filter_state.text_query.clone().unwrap_or_default();
    let parsed = parse_search_query(&raw_query);

    // Collect warnings for any empty field operators (e.g. `author:` with no value).
    let dropped_warnings: Vec<QueryWarningResponse> = parsed
        .dropped_empty_fields
        .iter()
        .map(|d| QueryWarningResponse::EmptyFieldValue {
            field: d.field.as_str().to_owned(),
        })
        .collect();

    // Treat a lone valid ISBN in the freeform search box like the existing ISBN filter.
    if maybe_promote_lone_isbn_query(&mut filter_state, &parsed) {
        let book_filter = BookFilter::from(&filter_state);
        return Ok((filter_state, book_filter, dropped_warnings));
    }

    // If there are no executable DSL clauses, skip resolution.
    // Always clear `text_query` — the raw input (e.g. "-", "OR", "author:")
    // has no executable content and must not leak into the FTS5 MATCH.
    if parsed.clauses.is_empty() {
        filter_state.text_query = None;
        let mut book_filter = BookFilter::from(&filter_state);
        book_filter.matches_nothing = !raw_query.trim().is_empty();
        return Ok((filter_state, book_filter, dropped_warnings));
    }

    let mut resolved = SearchResolver::resolve(pool, &parsed).await?;

    // ── Filter non-searchable text / column terms before they reach FTS5 ──
    let mut no_search_warnings: Vec<QueryWarningResponse> = Vec::new();

    if let Some(ref text) = resolved.text_query {
        if !archivis_db::text_has_searchable_fts_terms(text) {
            no_search_warnings.push(QueryWarningResponse::NoSearchableTerms {
                text: text.clone(),
                field: None,
            });
            resolved.text_query = None;
        }
    }

    resolved.fts_column_filters.retain(|f| {
        if archivis_db::column_filter_has_searchable_chars(&f.term) {
            true
        } else {
            no_search_warnings.push(QueryWarningResponse::NoSearchableTerms {
                text: f.term.clone(),
                field: Some(f.column.clone()),
            });
            false
        }
    });

    // Merge resolved relation IDs into `filter_state` (explicit params win).
    merge_resolved_into_filter(&mut filter_state, &resolved);

    let mut warnings: Vec<QueryWarningResponse> =
        resolved.warnings.iter().cloned().map(Into::into).collect();
    warnings.extend(no_search_warnings);
    warnings.extend(dropped_warnings);

    let mut book_filter = BookFilter::from_resolved(&filter_state, &resolved);
    book_filter.matches_nothing = !resolved.has_constraints();
    Ok((filter_state, book_filter, warnings))
}

/// Merge DSL-resolved values into `LibraryFilterState`.
///
/// Explicit query parameters (already set in `lfs`) take precedence over
/// DSL-resolved values. Only fills in gaps.
fn merge_resolved_into_filter(lfs: &mut LibraryFilterState, resolved: &archivis_db::ResolvedQuery) {
    // Text query: always override with resolver's cleaned version
    // (the resolver strips out extracted field operators).
    lfs.text_query.clone_from(&resolved.text_query);

    // Relations: only fill if not already set by explicit params.
    if lfs.author_id.is_none() {
        lfs.author_id = resolved.author_id;
    }
    if lfs.series_id.is_none() {
        lfs.series_id = resolved.series_id;
    }
    if lfs.publisher_id.is_none() {
        lfs.publisher_id = resolved.publisher_id;
    }
    if lfs.tag_ids.is_empty() && !resolved.tag_ids.is_empty() {
        lfs.tag_ids.clone_from(&resolved.tag_ids);
    }

    // Scalars: only fill if not already set.
    if lfs.format.is_none() {
        lfs.format = resolved.format;
    }
    if lfs.metadata_status.is_none() {
        lfs.metadata_status = resolved.metadata_status;
    }
    if lfs.resolution_state.is_none() {
        lfs.resolution_state = resolved.resolution_state;
    }
    if lfs.resolution_outcome.is_none() {
        lfs.resolution_outcome = resolved.resolution_outcome;
    }
    if lfs.trusted.is_none() {
        lfs.trusted = resolved.trusted;
    }
    if lfs.locked.is_none() {
        lfs.locked = resolved.locked;
    }
    if lfs.language.is_none() {
        lfs.language.clone_from(&resolved.language);
    }
    if lfs.year_min.is_none() {
        lfs.year_min = resolved.year_min;
    }
    if lfs.year_max.is_none() {
        lfs.year_max = resolved.year_max;
    }
    if lfs.has_cover.is_none() {
        lfs.has_cover = resolved.has_cover;
    }
    if lfs.has_description.is_none() {
        lfs.has_description = resolved.has_description;
    }
    if lfs.has_identifiers.is_none() {
        lfs.has_identifiers = resolved.has_identifiers;
    }

    // Identifiers: only fill if not already set.
    if lfs.identifier_value.is_none() {
        lfs.identifier_type.clone_from(&resolved.identifier_type);
        lfs.identifier_value.clone_from(&resolved.identifier_value);
    }
}

/// POST /api/books/selection-scope — issue a signed scope token for promoted selection.
#[utoipa::path(
    post,
    path = "/api/books/selection-scope",
    tag = "books",
    request_body = IssueSelectionScopeRequest,
    responses(
        (status = 200, description = "Scope token issued", body = IssueSelectionScopeResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn issue_selection_scope(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<IssueSelectionScopeRequest>,
) -> Result<Json<IssueSelectionScopeResponse>, ApiError> {
    let mut filter = body.filters;
    filter.canonicalize();
    if let Some(ref identifier_type) = filter.identifier_type {
        if !is_supported_identifier_type(identifier_type) {
            return Err(ApiError::Validation(format!(
                "unknown identifier type: {identifier_type}"
            )));
        }
    }

    // Resolve DSL operators so the scope token embeds concrete IDs.
    let (filter, book_filter, _warnings) = resolve_dsl(state.db_pool(), filter).await?;
    let matching_count = BookRepository::count(state.db_pool(), &book_filter).await?;

    let scope_token = super::scope::sign_scope(state.scope_signing_key(), &filter);

    let summary = format!("{matching_count} books matching current filters");

    Ok(Json(IssueSelectionScopeResponse {
        scope_token,
        matching_count,
        summary,
    }))
}

/// GET /api/books/{id} — single book with all relations.
#[utoipa::path(
    get,
    path = "/api/books/{id}",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Book detail", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let bwr = BookRepository::get_with_relations(state.db_pool(), id).await?;
    Ok(Json(bwr.into()))
}

fn validate_language(input: &str) -> Result<Option<String>, ApiError> {
    if input.is_empty() {
        return Ok(None);
    }
    archivis_core::language::normalize_language(input)
        .map(|code| Some(code.to_string()))
        .ok_or_else(|| ApiError::Validation(format!("unrecognized language: {input:?}")))
}

/// PUT /api/books/{id} — update book metadata (partial update).
#[utoipa::path(
    put,
    path = "/api/books/{id}",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = UpdateBookRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
        (status = 409, description = "Metadata refresh in progress"),
    ),
    security(("bearer" = []))
)]
#[allow(clippy::too_many_lines)]
pub async fn update_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBookRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;
    let mut book_changed = false;
    let mut core_identity_changed = false;

    // --- Pre-validate all fields BEFORE any mutations ---
    let sanitize_opts = SanitizeOptions::default();

    let validated_title = body
        .title
        .as_ref()
        .map(|t| sanitize_text(t, &sanitize_opts).unwrap_or_default());
    if let Some(ref clean) = validated_title {
        if clean.is_empty() {
            return Err(ApiError::Validation("title must not be empty".into()));
        }
    }
    let validated_subtitle = body.subtitle.as_ref().map(|s| {
        if s.is_empty() {
            None
        } else {
            sanitize_text(s, &sanitize_opts)
        }
    });
    let validated_description = body
        .description
        .as_ref()
        .map(|d| sanitize_text(d, &sanitize_opts));
    let validated_language = body
        .language
        .as_deref()
        .map(validate_language)
        .transpose()?;

    // --- Trust change (atomic, after validation) ---
    if let Some(trusted) = body.metadata_user_trusted {
        if trusted != book.metadata_user_trusted {
            if trusted {
                let ok = state
                    .resolve_service()
                    .trust_metadata(id)
                    .await
                    .map_err(|e| ApiError::Internal(format!("trust failed: {e}")))?;
                if !ok {
                    return Err(ApiError::Conflict(
                        "Cannot change trust while a metadata refresh is in progress".into(),
                    ));
                }
                // Update in-memory book to reflect trust state so subsequent
                // BookRepository::update (for field changes) does not overwrite it.
                book.metadata_user_trusted = true;
                book.metadata_status = archivis_core::models::MetadataStatus::Identified;
                book.resolution_outcome = Some(archivis_core::models::ResolutionOutcome::Confirmed);
                book.resolution_state = archivis_core::models::ResolutionState::Done;
                book.review_baseline_metadata_status = None;
                book.review_baseline_resolution_outcome = None;
            } else {
                let result = state
                    .resolve_service()
                    .untrust_metadata(id)
                    .await
                    .map_err(|e| ApiError::Internal(format!("untrust failed: {e}")))?;
                if result.is_none() {
                    return Err(ApiError::Conflict(
                        "Cannot change trust while a metadata refresh is in progress".into(),
                    ));
                }
                // Reload book — untrust changed status, outcome, state
                book = BookRepository::get_by_id(pool, id).await?;
            }
        }
    }

    // --- Apply pre-validated fields ---
    if let Some(ref clean) = validated_title {
        if *clean != book.title {
            book.set_title(clean.clone());
            book.metadata_provenance.title = Some(user_field_provenance());
            book_changed = true;
            core_identity_changed = true;
        }
    }
    if let Some(ref new_subtitle) = validated_subtitle {
        if *new_subtitle != book.subtitle {
            book.subtitle = new_subtitle.clone();
            book.metadata_provenance.subtitle = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(ref new_description) = validated_description {
        if *new_description != book.description {
            book.description = new_description.clone();
            book.metadata_provenance.description = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(ref new_language) = validated_language {
        if *new_language != book.language {
            book.language = new_language.clone();
            book.metadata_provenance.language = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(pub_year) = body.publication_year {
        let new_publication_year = Some(pub_year);
        if new_publication_year != book.publication_year {
            book.publication_year = new_publication_year;
            book.metadata_provenance.publication_year = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(rating) = body.rating {
        let new_rating = Some(rating);
        if new_rating != book.rating {
            book.rating = new_rating;
            book_changed = true;
        }
    }
    if let Some(page_count) = body.page_count {
        let new_page_count = Some(page_count);
        if new_page_count != book.page_count {
            book.page_count = new_page_count;
            book.metadata_provenance.page_count = Some(user_field_provenance());
            book_changed = true;
        }
    }
    // publisher_id: Some(Some(id)) = set, Some(None) = clear, None = no change
    if let Some(pub_id) = body.publisher_id {
        if pub_id != book.publisher_id {
            book.publisher_id = pub_id;
            book.metadata_provenance.publisher = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if book_changed {
        BookRepository::update(pool, &book).await?;
    }
    if core_identity_changed {
        invalidate_resolution_for_core_edit(pool, id).await?;
    }
    // Single BWR load AFTER mutations + invalidation so `resolution_state` is fresh
    let mut bwr = BookRepository::get_with_relations(pool, id).await?;
    if book_changed {
        stamp_quality_score(pool, &mut bwr).await;
    }
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/lock — disable automated metadata mutation for a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/lock",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn lock_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if !book.metadata_locked {
        book.metadata_locked = true;
        BookRepository::update(pool, &book).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/unlock — re-enable automated metadata mutation for a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/unlock",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn unlock_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if book.metadata_locked {
        book.metadata_locked = false;
        BookRepository::update(pool, &book).await?;
        invalidate_resolution_for_action(pool, id, UNLOCK_TRIGGER).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/protect-fields — mark fields as protected from automation.
#[utoipa::path(
    post,
    path = "/api/books/{id}/protect-fields",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = FieldProtectionRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn protect_fields(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FieldProtectionRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if set_field_protection(&mut book, &body.fields, true)? {
        BookRepository::update(pool, &book).await?;
        invalidate_resolution_for_action(pool, id, PROTECT_FIELDS_TRIGGER).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/unprotect-fields — clear field protection without changing values.
#[utoipa::path(
    post,
    path = "/api/books/{id}/unprotect-fields",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = FieldProtectionRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn unprotect_fields(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FieldProtectionRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if set_field_protection(&mut book, &body.fields, false)? {
        BookRepository::update(pool, &book).await?;
        invalidate_resolution_for_action(pool, id, UNPROTECT_FIELDS_TRIGGER).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/override-status — manually set metadata status and lock the book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/override-status",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = OverrideStatusRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn override_status(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<OverrideStatusRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    book.metadata_status = body.metadata_status;
    book.metadata_locked = true;
    BookRepository::update(pool, &book).await?;

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// DELETE /api/books/{id} — delete book, its files, and cover.
#[utoipa::path(
    delete,
    path = "/api/books/{id}",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 204, description = "Book deleted"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();
    let bwr = BookRepository::get_with_relations(pool, id).await?;
    let storage = state.storage();

    // Delete book files from storage (ignore not-found)
    for file in &bwr.files {
        if let Err(e) = storage.delete(&file.storage_path).await {
            tracing::warn!(path = %file.storage_path, error = %e, "failed to delete book file from storage");
        }
    }

    // Delete cover from storage
    if let Some(ref cover_path) = bwr.book.cover_path {
        if let Err(e) = storage.delete(cover_path).await {
            tracing::warn!(path = %cover_path, error = %e, "failed to delete cover from storage");
        }
    }

    // Remove thumbnail cache directory
    let cache_dir = state.config().data_dir.join("covers").join(id.to_string());
    if cache_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await {
            tracing::warn!(path = ?cache_dir, error = %e, "failed to remove thumbnail cache");
        }
    }

    BookRepository::delete(pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/books/{id}/cover — serve cover image.
#[utoipa::path(
    get,
    path = "/api/books/{id}/cover",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        CoverParams,
    ),
    responses(
        (status = 200, description = "Cover image", content_type = "image/*"),
        (status = 304, description = "Not modified"),
        (status = 404, description = "Cover not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_cover(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Query(params): Query<CoverParams>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let book = BookRepository::get_by_id(pool, id).await?;

    let cover_path = book
        .cover_path
        .as_deref()
        .ok_or_else(|| ApiError::NotFound("book has no cover".into()))?;

    let size = params.size.as_deref().unwrap_or("original");
    let data_dir = &state.config().data_dir;

    match size {
        "sm" | "md" => {
            let thumb_path = data_dir
                .join("covers")
                .join(id.to_string())
                .join(format!("{size}.webp"));

            // Lazily generate if missing
            if !thumb_path.exists() {
                let storage = state.storage();
                let cover_bytes = storage.read(cover_path).await?;

                let target_height = if size == "sm" { 150 } else { 300 };

                // Write source to a temp file for generate_thumbnail
                let tmp_dir = tempfile::tempdir()
                    .map_err(|e| ApiError::Internal(format!("failed to create temp dir: {e}")))?;
                let tmp_source = tmp_dir.path().join("source");
                tokio::fs::write(&tmp_source, &cover_bytes)
                    .await
                    .map_err(|e| ApiError::Internal(format!("failed to write temp cover: {e}")))?;

                archivis_tasks::import::generate_thumbnail(
                    &tmp_source,
                    id,
                    data_dir,
                    size,
                    target_height,
                )
                .await
                .map_err(|e| ApiError::Internal(format!("thumbnail generation failed: {e}")))?;
            }

            serve_file_with_etag(&thumb_path, "image/webp", &headers).await
        }
        "lg" => {
            let thumb_path = data_dir.join("covers").join(id.to_string()).join("lg.webp");

            // Lazily generate if missing
            if !thumb_path.exists() {
                let storage = state.storage();
                let cover_bytes = storage.read(cover_path).await?;

                // Write source to a temp file for generate_thumbnail
                let tmp_dir = tempfile::tempdir()
                    .map_err(|e| ApiError::Internal(format!("failed to create temp dir: {e}")))?;
                let tmp_source = tmp_dir.path().join("source");
                tokio::fs::write(&tmp_source, &cover_bytes)
                    .await
                    .map_err(|e| ApiError::Internal(format!("failed to write temp cover: {e}")))?;

                archivis_tasks::import::generate_thumbnail(&tmp_source, id, data_dir, "lg", 600)
                    .await
                    .map_err(|e| ApiError::Internal(format!("thumbnail generation failed: {e}")))?;
            }

            serve_file_with_etag(&thumb_path, "image/webp", &headers).await
        }
        _ => {
            let storage = state.storage();
            let cover_bytes = storage.read(cover_path).await?;

            let content_type = match cover_path.rsplit('.').next() {
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("png") => "image/png",
                Some("gif") => "image/gif",
                Some("webp") => "image/webp",
                Some("svg") => "image/svg+xml",
                _ => "application/octet-stream",
            };

            // Use a hash-based ETag for storage-backed files
            let etag = format!("W/\"{}\"", simple_hash(&cover_bytes));

            if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
                if if_none_match == etag {
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            Ok((
                [
                    (CONTENT_TYPE, content_type.to_string()),
                    (CONTENT_LENGTH, cover_bytes.len().to_string()),
                    (ETAG, etag),
                    (
                        CACHE_CONTROL,
                        "public, max-age=86400, must-revalidate".into(),
                    ),
                ],
                cover_bytes,
            )
                .into_response())
        }
    }
}

/// POST /api/books/{id}/cover — upload or replace cover image.
#[utoipa::path(
    post,
    path = "/api/books/{id}/cover",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Cover uploaded, book updated", body = BookDetail),
        (status = 400, description = "Invalid image or no file provided"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn upload_cover(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let bwr = BookRepository::get_with_relations(pool, id).await?;
    let mut book = bwr.book;
    let storage = state.storage();
    let data_dir = &state.config().data_dir;

    // Extract the first file field from the multipart form
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::Validation(format!("multipart error: {e}")))?
        .ok_or_else(|| ApiError::Validation("no file provided".into()))?;

    // Validate content type is an image
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    if !content_type.starts_with("image/") {
        return Err(ApiError::Validation(format!(
            "file must be an image, got: {content_type}"
        )));
    }

    let image_bytes = field
        .bytes()
        .await
        .map_err(|e| ApiError::Validation(format!("failed to read upload: {e}")))?;

    if image_bytes.is_empty() {
        return Err(ApiError::Validation("uploaded file is empty".into()));
    }

    // Determine the book's storage directory from its first file
    let book_path_dir = bwr
        .files
        .first()
        .and_then(|f| {
            let p = &f.storage_path;
            p.rfind('/').map(|idx| &p[..idx])
        })
        .ok_or_else(|| {
            ApiError::Validation("book has no files; cannot determine storage directory".into())
        })?
        .to_string();

    // Delete old cover from storage if present
    if let Some(ref old_cover_path) = book.cover_path {
        if let Err(e) = storage.delete(old_cover_path).await {
            tracing::warn!(path = %old_cover_path, error = %e, "failed to delete old cover from storage");
        }
    }

    // Delete old thumbnail cache directory
    let cache_dir = data_dir.join("covers").join(id.to_string());
    if cache_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await {
            tracing::warn!(path = ?cache_dir, error = %e, "failed to remove old thumbnail cache");
        }
    }

    // Store new cover via StorageBackend
    let cover_data = CoverData {
        bytes: image_bytes.to_vec(),
        media_type: content_type,
    };

    let new_cover_path = archivis_tasks::import::store_cover(storage, &book_path_dir, &cover_data)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to store cover: {e}")))?;

    // Generate sm + md thumbnails
    let thumbnail_sizes = archivis_tasks::import::ThumbnailSizes::default();
    archivis_tasks::import::generate_thumbnails(&cover_data, id, data_dir, &thumbnail_sizes)
        .await
        .map_err(|e| ApiError::Internal(format!("thumbnail generation failed: {e}")))?;

    // Update book.cover_path in the database
    book.cover_path = Some(new_cover_path);
    BookRepository::update(pool, &book).await?;
    let mut bwr = BookRepository::get_with_relations(pool, id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// Serve a local file with ETag/304 support.
async fn serve_file_with_etag(
    path: &std::path::Path,
    content_type: &str,
    req_headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|_| ApiError::NotFound("cover file not found".into()))?;

    let modified = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let etag = format!(
        "W/\"{}-{}\"",
        modified
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        metadata.len()
    );

    if let Some(if_none_match) = req_headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_none_match == etag {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to read cover: {e}")))?;

    Ok((
        [
            (CONTENT_TYPE, content_type.to_string()),
            (CONTENT_LENGTH, bytes.len().to_string()),
            (ETAG, etag),
            (
                CACHE_CONTROL,
                "public, max-age=86400, must-revalidate".into(),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// Simple hash for `ETag` generation.
fn simple_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// `GET /api/books/{id}/files/{file_id}/download` — stream book file.
#[utoipa::path(
    get,
    path = "/api/books/{id}/files/{file_id}/download",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    responses(
        (status = 200, description = "File download", content_type = "application/octet-stream"),
        (status = 404, description = "File not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn download_file(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;

    // Verify the file belongs to this book
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    let storage = state.storage();
    let data = storage.read(&book_file.storage_path).await?;

    let book = BookRepository::get_by_id(pool, book_id).await?;
    let ext = book_file.format.extension();
    let filename = format!("{}.{ext}", book.title);
    // Sanitize filename for Content-Disposition
    let safe_filename = filename.replace('"', "'");

    Ok((
        [
            (CONTENT_TYPE, book_file.format.mime_type().to_string()),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{safe_filename}\""),
            ),
            (CONTENT_LENGTH, data.len().to_string()),
        ],
        data,
    )
        .into_response())
}

/// `GET /api/books/{id}/files/{file_id}/content` — serve book file for in-browser reading.
///
/// Unlike /download (Content-Disposition: attachment), this uses inline disposition
/// and aggressive caching since ebook files are immutable after import.
#[utoipa::path(
    get,
    path = "/api/books/{id}/files/{file_id}/content",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    responses(
        (status = 200, description = "File content", content_type = "application/octet-stream"),
        (status = 304, description = "Not modified"),
        (status = 404, description = "File not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn serve_file_content(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;

    // Verify the file belongs to this book
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    // ETag based on the file's SHA-256 hash
    let etag = format!("\"{}\"", book_file.hash);

    // Check If-None-Match for conditional request
    if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_none_match == etag {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    let storage = state.storage();
    let data = storage.read(&book_file.storage_path).await?;

    Ok((
        [
            (CONTENT_TYPE, book_file.format.mime_type().to_string()),
            (CONTENT_DISPOSITION, "inline".to_string()),
            (CONTENT_LENGTH, data.len().to_string()),
            (ETAG, etag),
            (CACHE_CONTROL, "public, max-age=604800, immutable".into()),
        ],
        data,
    )
        .into_response())
}

/// POST /api/books/{id}/authors — replace book-author links.
#[utoipa::path(
    post,
    path = "/api/books/{id}/authors",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = SetBookAuthorsRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book or author not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn set_book_authors(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetBookAuthorsRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    // Verify each author exists
    for link in &body.authors {
        AuthorRepository::get_by_id(pool, link.author_id).await?;
    }

    // Replace all author links
    BookRepository::clear_authors(pool, id).await?;
    for link in &body.authors {
        BookRepository::add_author(pool, id, link.author_id, &link.role, link.position).await?;
    }

    book.metadata_provenance.authors = Some(user_field_provenance());
    BookRepository::update(pool, &book).await?;
    invalidate_resolution_for_core_edit(pool, id).await?;
    let mut bwr = BookRepository::get_with_relations(pool, id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/series — replace book-series links.
#[utoipa::path(
    post,
    path = "/api/books/{id}/series",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = SetBookSeriesRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book or series not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn set_book_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetBookSeriesRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    // Verify each series exists
    for link in &body.series {
        SeriesRepository::get_by_id(pool, link.series_id).await?;
    }

    // Replace all series links
    BookRepository::clear_series(pool, id).await?;
    for link in &body.series {
        BookRepository::add_series(pool, id, link.series_id, link.position).await?;
    }

    book.metadata_provenance.series = Some(user_field_provenance());
    BookRepository::update(pool, &book).await?;
    invalidate_resolution_for_core_edit(pool, id).await?;
    let mut bwr = BookRepository::get_with_relations(pool, id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/tags — replace book-tag links.
#[utoipa::path(
    post,
    path = "/api/books/{id}/tags",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = SetBookTagsRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Tag link must have tag_id or name"),
        (status = 404, description = "Book or tag not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn set_book_tags(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetBookTagsRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    // Resolve each tag
    let mut tag_ids = Vec::with_capacity(body.tags.len());
    for link in &body.tags {
        let tag_id = if let Some(tid) = link.tag_id {
            // Verify tag exists
            TagRepository::get_by_id(pool, tid).await?;
            tid
        } else if let Some(ref name) = link.name {
            let tag = TagRepository::find_or_create(pool, name, link.category.as_deref()).await?;
            tag.id
        } else {
            return Err(ApiError::Validation(
                "each tag must have either tag_id or name".into(),
            ));
        };
        tag_ids.push(tag_id);
    }

    // Replace all tag links
    BookRepository::clear_tags(pool, id).await?;
    for tag_id in tag_ids {
        BookRepository::add_tag(pool, id, tag_id).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/identifiers — add a new identifier to a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/identifiers",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = AddIdentifierRequest,
    responses(
        (status = 200, description = "Identifier added, updated book returned", body = BookDetail),
        (status = 400, description = "Validation error (e.g. invalid ISBN checksum)"),
        (status = 404, description = "Book not found"),
        (status = 409, description = "Duplicate identifier"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn add_identifier(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AddIdentifierRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    let identifier_type = body.identifier_type;
    let mut value = body.value.trim().to_string();

    // For ISBN types: validate checksum and normalize
    if matches!(
        identifier_type,
        IdentifierType::Isbn13 | IdentifierType::Isbn10
    ) {
        let validation = validate_isbn(&value);
        if !validation.valid {
            return Err(ApiError::Validation(validation.message));
        }
        value = validation.normalized;
    }

    // Serialize the identifier type to its DB string form
    let type_str = serde_json::to_value(identifier_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();

    // Check for duplicates
    if IdentifierRepository::exists_for_book(pool, id, &type_str, &value).await? {
        return Err(ApiError::Validation(format!(
            "identifier {type_str}:{value} already exists for this book"
        )));
    }

    // Create the identifier with source: User and confidence: 1.0
    let identifier = Identifier::new(id, identifier_type, &value, MetadataSource::User, 1.0);
    IdentifierRepository::create(pool, &identifier).await?;
    invalidate_resolution_for_core_edit(pool, id).await?;
    let mut bwr = BookRepository::get_with_relations(pool, id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// `PUT /api/books/{id}/identifiers/{identifier_id}` — update an existing identifier.
#[utoipa::path(
    put,
    path = "/api/books/{id}/identifiers/{identifier_id}",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("identifier_id" = Uuid, Path, description = "Identifier ID"),
    ),
    request_body = UpdateIdentifierRequest,
    responses(
        (status = 200, description = "Identifier updated, updated book returned", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book or identifier not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_identifier(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, identifier_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateIdentifierRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Load identifier and verify it belongs to this book
    let existing = IdentifierRepository::get_by_id(pool, identifier_id).await?;
    if existing.book_id != book_id {
        return Err(ApiError::NotFound(
            "identifier not found for this book".into(),
        ));
    }

    let new_type = body.identifier_type.unwrap_or(existing.identifier_type);
    let new_value = body
        .value
        .map(|v| v.trim().to_string())
        .unwrap_or(existing.value);

    // Validate ISBN if the type is an ISBN type
    let final_value = if matches!(new_type, IdentifierType::Isbn13 | IdentifierType::Isbn10) {
        let validation = validate_isbn(&new_value);
        if !validation.valid {
            return Err(ApiError::Validation(validation.message));
        }
        validation.normalized
    } else {
        new_value
    };

    let type_str = serde_json::to_value(new_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();

    IdentifierRepository::update(pool, identifier_id, &final_value, &type_str).await?;
    invalidate_resolution_for_core_edit(pool, book_id).await?;
    let mut bwr = BookRepository::get_with_relations(pool, book_id).await?;
    stamp_quality_score(pool, &mut bwr).await;
    Ok(Json(bwr.into()))
}

/// `DELETE /api/books/{id}/identifiers/{identifier_id}` — remove an identifier.
#[utoipa::path(
    delete,
    path = "/api/books/{id}/identifiers/{identifier_id}",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("identifier_id" = Uuid, Path, description = "Identifier ID"),
    ),
    responses(
        (status = 204, description = "Identifier deleted"),
        (status = 404, description = "Book or identifier not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_identifier(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, identifier_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Load identifier and verify it belongs to this book
    let existing = IdentifierRepository::get_by_id(pool, identifier_id).await?;
    if existing.book_id != book_id {
        return Err(ApiError::NotFound(
            "identifier not found for this book".into(),
        ));
    }

    IdentifierRepository::delete(pool, identifier_id).await?;
    invalidate_resolution_for_core_edit(pool, book_id).await?;
    refresh_quality_score_best_effort(pool, book_id).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Validated + verified selection, ready for dispatch.
enum ResolvedSelection {
    /// Explicit book IDs (always small enough for sync execution).
    Ids(Vec<Uuid>),
    /// Scope with pre-computed matching count, no IDs materialized yet.
    Scope {
        filter: Box<archivis_core::models::LibraryFilterState>,
        excluded_ids: Vec<Uuid>,
        matching_count: u64,
    },
}

impl ResolvedSelection {
    /// Effective number of books that will be affected.
    fn count(&self) -> u64 {
        match self {
            Self::Ids(ids) => ids.len() as u64,
            Self::Scope { matching_count, .. } => *matching_count,
        }
    }

    /// True when the scope is small enough for synchronous execution.
    fn is_sync(&self) -> bool {
        self.count() <= 100
    }

    /// Materialize concrete IDs. Only called on the sync path (<=100).
    async fn into_ids(self, pool: &archivis_db::DbPool) -> Result<Vec<Uuid>, ApiError> {
        match self {
            Self::Ids(ids) => Ok(ids),
            Self::Scope {
                filter,
                excluded_ids,
                ..
            } => {
                let book_filter = BookFilter::from(filter.as_ref());
                let ids = BookRepository::resolve_scope(pool, &book_filter, &excluded_ids).await?;
                if ids.is_empty() {
                    return Err(ApiError::Validation(
                        "scope resolves to zero books after exclusions".into(),
                    ));
                }
                Ok(ids)
            }
        }
    }
}

/// Validate a `SelectionSpec` and compute matching count without materializing IDs.
///
/// For `Ids` mode: validates non-empty, returns IDs directly.
/// For `Scope` mode: verifies the token signature, computes `count()` minus exclusion
/// count (approximate — exact subtraction happens at execution time).
async fn resolve_selection(
    state: &AppState,
    selection: &SelectionSpec,
) -> Result<ResolvedSelection, ApiError> {
    match selection {
        SelectionSpec::Ids { ids } => {
            if ids.is_empty() {
                return Err(ApiError::Validation("ids must not be empty".into()));
            }
            Ok(ResolvedSelection::Ids(ids.clone()))
        }
        SelectionSpec::Scope {
            scope_token,
            excluded_ids,
        } => {
            let filter = super::scope::verify_scope(state.scope_signing_key(), scope_token)
                .map_err(|e| ApiError::Validation(e.to_string()))?;
            let book_filter = BookFilter::from(&filter);
            let matching_count =
                BookRepository::count_scope(state.db_pool(), &book_filter, excluded_ids).await?;
            if matching_count == 0 {
                return Err(ApiError::Validation(
                    "scope resolves to zero books after exclusions".into(),
                ));
            }
            Ok(ResolvedSelection::Scope {
                filter: Box::new(filter),
                excluded_ids: excluded_ids.clone(),
                matching_count,
            })
        }
    }
}

/// POST /api/books/batch-update -- batch update scalar fields on multiple books.
///
/// Accepts `SelectionSpec` (explicit IDs or scope token).
/// Returns 200 for synchronous execution (<=100 books) or 202 for async (>100).
#[utoipa::path(
    post,
    path = "/api/books/batch-update",
    tag = "books",
    request_body = BatchUpdateBooksRequest,
    responses(
        (status = 200, description = "Synchronous batch result", body = BatchSyncResponse),
        (status = 202, description = "Async task enqueued", body = BatchAsyncResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_update_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchUpdateBooksRequest>,
) -> Result<Response, ApiError> {
    body.validate()?;

    let resolved = resolve_selection(&state, &body.selection).await?;

    // Async path: >100 books → enqueue background task (no ID materialization)
    if !resolved.is_sync() {
        let matching_count = resolved.count();
        let (filter, excluded_ids) = match resolved {
            ResolvedSelection::Scope {
                filter,
                excluded_ids,
                ..
            } => (*filter, excluded_ids),
            ResolvedSelection::Ids(_) => {
                return Err(ApiError::Validation(
                    "batch update with more than 100 books requires a scope token".into(),
                ));
            }
        };

        let payload = BulkTaskPayload {
            filter,
            excluded_ids,
            operation: BulkOperation::Update {
                fields: BulkUpdateFields {
                    language: body.updates.language.clone(),
                    rating: body.updates.rating,
                    publisher_id: body.updates.publisher_id,
                },
            },
        };

        let payload_json = serde_json::to_value(&payload)
            .map_err(|e| ApiError::Internal(format!("failed to serialize payload: {e}")))?;
        let task_id = state
            .task_queue()
            .enqueue(TaskType::BulkUpdate, payload_json)
            .await?;

        return Ok((
            StatusCode::ACCEPTED,
            Json(BatchAsyncResponse {
                task_id,
                task_type: TaskType::BulkUpdate.to_string(),
                matching_count,
                message: format!("Bulk update enqueued for {matching_count} books"),
            }),
        )
            .into_response());
    }

    // Sync path: <=100 books → materialize IDs and execute immediately
    let pool = state.db_pool();
    let ids = resolved.into_ids(pool).await?;
    let mut updated_count: u32 = 0;
    let mut errors = Vec::new();

    for &book_id in &ids {
        match apply_batch_fields(pool, book_id, &body.updates).await {
            Ok(()) => updated_count += 1,
            Err(e) => errors.push(BatchUpdateError {
                book_id,
                error: e.to_string(),
            }),
        }
    }

    Ok(Json(BatchSyncResponse {
        updated_count,
        errors,
    })
    .into_response())
}

/// Apply batch field updates to a single book. Returns `Ok(())` on success.
///
/// Delegates to [`apply_bulk_update_to_book`] so that the API sync path and
/// the background bulk worker share identical semantics (language validation,
/// provenance stamping, quality-score refresh).
async fn apply_batch_fields(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    fields: &BatchBookFields,
) -> Result<(), ApiError> {
    let bulk_fields = BulkUpdateFields {
        language: fields.language.clone(),
        rating: fields.rating,
        publisher_id: fields.publisher_id,
    };
    apply_bulk_update_to_book(pool, book_id, &bulk_fields)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            BulkFieldError::Validation(msg) => ApiError::Validation(msg),
            BulkFieldError::Db(msg) => ApiError::Internal(msg),
        })
}

/// POST /api/books/batch-tags -- batch set or add tags on multiple books.
///
/// Accepts `SelectionSpec` (explicit IDs or scope token).
/// Returns 200 for synchronous execution (<=100 books) or 202 for async (>100).
#[utoipa::path(
    post,
    path = "/api/books/batch-tags",
    tag = "books",
    request_body = BatchSetTagsRequest,
    responses(
        (status = 200, description = "Synchronous batch result", body = BatchSyncResponse),
        (status = 202, description = "Async task enqueued", body = BatchAsyncResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_set_tags(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchSetTagsRequest>,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();

    // ── 1. Validate selection FIRST (before any tag side effects) ──
    let resolved = resolve_selection(&state, &body.selection).await?;

    // Validate tag input structure (no find_or_create yet — just check shape).
    for link in &body.tags {
        if link.tag_id.is_none() && link.name.is_none() {
            return Err(ApiError::Validation(
                "each tag must have either tag_id or name".into(),
            ));
        }
    }

    // ── 2. Now resolve tag IDs (find_or_create is safe since selection is valid) ──
    let mut tag_ids = Vec::with_capacity(body.tags.len());
    for link in &body.tags {
        let tag_id = if let Some(tid) = link.tag_id {
            TagRepository::get_by_id(pool, tid).await?;
            tid
        } else {
            // name is guaranteed Some by the validation above.
            let name = link.name.as_ref().unwrap();
            let tag = TagRepository::find_or_create(pool, name, link.category.as_deref()).await?;
            tag.id
        };
        tag_ids.push(tag_id);
    }

    // ── 3. Dispatch async or sync ──

    // Async path: >100 books → enqueue background task (no ID materialization)
    if !resolved.is_sync() {
        let matching_count = resolved.count();
        let (filter, excluded_ids) = match resolved {
            ResolvedSelection::Scope {
                filter,
                excluded_ids,
                ..
            } => (*filter, excluded_ids),
            ResolvedSelection::Ids(_) => {
                return Err(ApiError::Validation(
                    "batch tag update with more than 100 books requires a scope token".into(),
                ));
            }
        };

        let bulk_mode = match body.mode {
            BatchTagMode::Replace => BulkTagMode::Replace,
            BatchTagMode::Add => BulkTagMode::Add,
        };

        let payload = BulkTaskPayload {
            filter,
            excluded_ids,
            operation: BulkOperation::SetTags {
                mode: bulk_mode,
                tags: tag_ids
                    .iter()
                    .map(|&tag_id| BulkTagEntry { tag_id })
                    .collect(),
            },
        };

        let payload_json = serde_json::to_value(&payload)
            .map_err(|e| ApiError::Internal(format!("failed to serialize payload: {e}")))?;
        let task_id = state
            .task_queue()
            .enqueue(TaskType::BulkSetTags, payload_json)
            .await?;

        return Ok((
            StatusCode::ACCEPTED,
            Json(BatchAsyncResponse {
                task_id,
                task_type: TaskType::BulkSetTags.to_string(),
                matching_count,
                message: format!("Bulk tag update enqueued for {matching_count} books"),
            }),
        )
            .into_response());
    }

    // Sync path: <=100 books → materialize IDs and execute immediately
    let ids = resolved.into_ids(pool).await?;
    let mut updated_count: u32 = 0;
    let mut errors = Vec::new();

    for &book_id in &ids {
        match apply_batch_tags(pool, book_id, &tag_ids, &body.mode).await {
            Ok(()) => updated_count += 1,
            Err(e) => errors.push(BatchUpdateError {
                book_id,
                error: e.to_string(),
            }),
        }
    }

    Ok(Json(BatchSyncResponse {
        updated_count,
        errors,
    })
    .into_response())
}

/// Apply tag changes to a single book. Returns `Ok(())` on success.
async fn apply_batch_tags(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    tag_ids: &[Uuid],
    mode: &BatchTagMode,
) -> Result<(), ApiError> {
    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    match mode {
        BatchTagMode::Replace => {
            BookRepository::clear_tags(pool, book_id).await?;
            for &tag_id in tag_ids {
                BookRepository::add_tag(pool, book_id, tag_id).await?;
            }
        }
        BatchTagMode::Add => {
            for &tag_id in tag_ids {
                BookRepository::add_tag(pool, book_id, tag_id).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::Json;
    use tempfile::TempDir;

    use archivis_auth::{AuthService, LocalAuthAdapter};
    use archivis_core::models::{
        Author, Book, FieldProvenance, Identifier, IdentifierType, MetadataSource, MetadataStatus,
        ResolutionOutcome, ResolutionState, Series, User, UserRole,
    };
    use archivis_db::{create_pool, run_migrations};
    use archivis_metadata::{MetadataResolver, ProviderRegistry};
    use archivis_storage::local::LocalStorage;
    use archivis_tasks::merge::MergeService;
    use archivis_tasks::queue::TaskQueue;
    use archivis_tasks::resolve::ResolutionService;

    use crate::books::types::{
        BatchSetTagsRequest, BatchSyncResponse, BatchTagMode, BatchUpdateBooksRequest,
        BookAuthorLink, BookSeriesLink, BookTagLink, FieldProtectionRequest,
        IssueSelectionScopeRequest, SelectionSpec,
    };
    use crate::settings::service::ConfigService;
    use crate::state::{ApiConfig, AppState};

    use super::*;

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
        let config_service = Arc::new(ConfigService::for_tests(db_pool.clone()));

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

    async fn create_resolved_book(pool: &archivis_db::DbPool, title: &str) -> (Book, Identifier) {
        let mut book = Book::new(title);
        book.metadata_status = MetadataStatus::Identified;
        book.resolution_state = ResolutionState::Done;
        BookRepository::create(pool, &book).await.unwrap();

        let author = Author::new("Seed Author");
        AuthorRepository::create(pool, &author).await.unwrap();
        BookRepository::add_author(pool, book.id, author.id, "author", 0)
            .await
            .unwrap();

        let identifier = Identifier::new(
            book.id,
            IdentifierType::Asin,
            "B000SEED",
            MetadataSource::Provider("seed".into()),
            0.7,
        );
        IdentifierRepository::create(pool, &identifier)
            .await
            .unwrap();

        (book, identifier)
    }

    #[tokio::test]
    async fn update_book_title_sets_user_provenance_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Before").await;

        let _ = update_book(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(UpdateBookRequest {
                title: Some("After".into()),
                subtitle: None,
                description: None,
                language: None,
                publication_year: None,
                rating: None,
                page_count: None,
                publisher_id: None,
                metadata_user_trusted: None,
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(updated.title, "After");
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.resolution_requested_reason.as_deref(),
            Some(USER_EDIT_TRIGGER)
        );
        assert_eq!(
            updated.metadata_provenance.title.as_ref().unwrap().origin,
            MetadataSource::User
        );
        assert!(
            updated
                .metadata_provenance
                .title
                .as_ref()
                .unwrap()
                .protected
        );
    }

    #[tokio::test]
    async fn set_book_authors_sets_user_provenance_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Author Edit").await;
        let replacement = Author::new("Replacement Author");
        AuthorRepository::create(state.db_pool(), &replacement)
            .await
            .unwrap();

        let _ = set_book_authors(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(SetBookAuthorsRequest {
                authors: vec![BookAuthorLink {
                    author_id: replacement.id,
                    role: "author".into(),
                    position: 0,
                }],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let with_relations = BookRepository::get_with_relations(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.metadata_provenance.authors.as_ref().unwrap().origin,
            MetadataSource::User
        );
        assert!(
            updated
                .metadata_provenance
                .authors
                .as_ref()
                .unwrap()
                .protected
        );
        assert_eq!(with_relations.authors.len(), 1);
        assert_eq!(with_relations.authors[0].author.id, replacement.id);
    }

    #[tokio::test]
    async fn set_book_series_sets_user_provenance_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Series Edit").await;
        let series = Series::new("Replacement Series");
        SeriesRepository::create(state.db_pool(), &series)
            .await
            .unwrap();

        let _ = set_book_series(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(SetBookSeriesRequest {
                series: vec![BookSeriesLink {
                    series_id: series.id,
                    position: Some(2.0),
                }],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let with_relations = BookRepository::get_with_relations(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.metadata_provenance.series.as_ref().unwrap().origin,
            MetadataSource::User
        );
        assert!(
            updated
                .metadata_provenance
                .series
                .as_ref()
                .unwrap()
                .protected
        );
        assert_eq!(with_relations.series.len(), 1);
        assert_eq!(with_relations.series[0].series.id, series.id);
    }

    #[tokio::test]
    async fn update_identifier_sets_user_source_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, identifier) = create_resolved_book(state.db_pool(), "Identifier Edit").await;

        let _ = update_identifier(
            State(state.clone()),
            auth_user(),
            Path((book.id, identifier.id)),
            Json(UpdateIdentifierRequest {
                identifier_type: Some(IdentifierType::Asin),
                value: Some("B000UPDATED".into()),
            }),
        )
        .await
        .unwrap();

        let updated_book = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let updated_identifier = IdentifierRepository::get_by_id(state.db_pool(), identifier.id)
            .await
            .unwrap();

        assert_eq!(updated_book.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated_book.resolution_requested_reason.as_deref(),
            Some(USER_EDIT_TRIGGER)
        );
        assert_eq!(updated_identifier.value, "B000UPDATED");
        assert_eq!(updated_identifier.source, MetadataSource::User);
        assert!((updated_identifier.confidence - 1.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn get_book_returns_resolution_fields() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut book = Book::new("Resolution Detail");
        book.metadata_status = MetadataStatus::NeedsReview;
        book.resolution_state = ResolutionState::Done;
        book.resolution_outcome = Some(ResolutionOutcome::Disputed);
        book.metadata_locked = true;
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let Json(detail) = get_book(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();

        assert_eq!(detail.metadata_status, MetadataStatus::NeedsReview);
        assert_eq!(detail.resolution_state, ResolutionState::Done);
        assert_eq!(detail.resolution_outcome, Some(ResolutionOutcome::Disputed));
        assert!(detail.metadata_locked);
        assert!(detail.metadata_provenance.title.is_none());
    }

    #[tokio::test]
    async fn lock_and_unlock_metadata_toggle_book_lock() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let book = Book::new("Toggle Lock");
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let Json(locked) = lock_metadata(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();
        assert!(locked.metadata_locked);

        let Json(unlocked) = unlock_metadata(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();
        assert!(!unlocked.metadata_locked);
        assert_eq!(unlocked.resolution_state, ResolutionState::Pending);

        let refreshed = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(
            refreshed.resolution_requested_reason.as_deref(),
            Some(UNLOCK_TRIGGER)
        );
    }

    #[tokio::test]
    async fn protect_fields_preserves_existing_origin() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut book = Book::new("Protect Me");
        book.metadata_provenance.title = Some(FieldProvenance {
            origin: MetadataSource::Provider("seed".into()),
            protected: false,
            applied_candidate_id: None,
        });
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let _ = protect_fields(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(FieldProtectionRequest {
                fields: vec!["title".into()],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let title = updated.metadata_provenance.title.as_ref().unwrap();
        assert_eq!(title.origin, MetadataSource::Provider("seed".into()));
        assert!(title.protected);
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.resolution_requested_reason.as_deref(),
            Some(PROTECT_FIELDS_TRIGGER)
        );
    }

    #[tokio::test]
    async fn unprotect_fields_clears_protection_without_dropping_origin() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut book = Book::new("Unprotect Me");
        book.metadata_provenance.title = Some(FieldProvenance {
            origin: MetadataSource::Embedded,
            protected: true,
            applied_candidate_id: None,
        });
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let _ = unprotect_fields(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(FieldProtectionRequest {
                fields: vec!["title".into()],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let title = updated.metadata_provenance.title.as_ref().unwrap();
        assert_eq!(title.origin, MetadataSource::Embedded);
        assert!(!title.protected);
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.resolution_requested_reason.as_deref(),
            Some(UNPROTECT_FIELDS_TRIGGER)
        );
    }

    #[tokio::test]
    async fn update_book_returns_metadata_quality_score() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Score Update").await;

        let Json(detail) = update_book(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(UpdateBookRequest {
                title: Some("Score Updated".into()),
                subtitle: None,
                description: None,
                language: None,
                publication_year: None,
                rating: None,
                page_count: None,
                publisher_id: None,
                metadata_user_trusted: None,
            }),
        )
        .await
        .unwrap();

        assert!(
            detail.metadata_quality_score.is_some(),
            "update_book response should include metadata_quality_score"
        );
    }

    #[tokio::test]
    async fn set_book_authors_returns_metadata_quality_score() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Author Score").await;
        let new_author = Author::new("New Author");
        AuthorRepository::create(state.db_pool(), &new_author)
            .await
            .unwrap();

        let Json(detail) = set_book_authors(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(SetBookAuthorsRequest {
                authors: vec![BookAuthorLink {
                    author_id: new_author.id,
                    role: "author".into(),
                    position: 0,
                }],
            }),
        )
        .await
        .unwrap();

        assert!(
            detail.metadata_quality_score.is_some(),
            "set_book_authors response should include metadata_quality_score"
        );
    }

    #[tokio::test]
    async fn add_identifier_returns_metadata_quality_score() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "ISBN Score").await;

        let Json(detail) = add_identifier(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(AddIdentifierRequest {
                identifier_type: IdentifierType::Isbn13,
                value: "9783161484100".into(),
            }),
        )
        .await
        .unwrap();

        assert!(
            detail.metadata_quality_score.is_some(),
            "add_identifier response should include metadata_quality_score"
        );
        // ISBN should boost score above the base title+author level
        let score = detail.metadata_quality_score.unwrap();
        assert!(
            score > 0.3,
            "score with ISBN should exceed title+author baseline of 0.3, got {score}"
        );
    }

    // ── Trust-aware invalidation tests ──

    /// Helper: trust a book via the resolution service, same as the PUT handler does.
    async fn trust_book(state: &AppState, book_id: Uuid) {
        let ok = state
            .resolve_service()
            .trust_metadata(book_id)
            .await
            .unwrap();
        assert!(ok, "trust_metadata should succeed");
    }

    #[tokio::test]
    async fn trust_plus_title_change_no_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Trust Title").await;

        let _ = update_book(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(UpdateBookRequest {
                title: Some("New Title".into()),
                subtitle: None,
                description: None,
                language: None,
                publication_year: None,
                rating: None,
                page_count: None,
                publisher_id: None,
                metadata_user_trusted: Some(true),
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(
            updated.resolution_state,
            ResolutionState::Done,
            "trusted book should stay Done, not Pending"
        );
        assert!(updated.metadata_user_trusted, "book should be trusted");
        assert_eq!(
            updated.metadata_status,
            MetadataStatus::Identified,
            "trusted book status should be Identified"
        );
        assert_eq!(updated.title, "New Title");
    }

    #[tokio::test]
    async fn title_edit_on_trusted_book_no_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Already Trusted").await;

        // Trust first
        trust_book(&state, book.id).await;

        // Then edit title (no trust field in request)
        let _ = update_book(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(UpdateBookRequest {
                title: Some("Edited Title".into()),
                subtitle: None,
                description: None,
                language: None,
                publication_year: None,
                rating: None,
                page_count: None,
                publisher_id: None,
                metadata_user_trusted: None,
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(
            updated.resolution_state,
            ResolutionState::Done,
            "trusted book should stay Done after title edit"
        );
        assert!(updated.metadata_user_trusted);
        assert_eq!(updated.metadata_status, MetadataStatus::Identified,);
    }

    #[tokio::test]
    async fn authors_edit_on_trusted_book_no_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Trusted Authors").await;

        // Trust first
        trust_book(&state, book.id).await;

        let replacement = Author::new("New Author");
        AuthorRepository::create(state.db_pool(), &replacement)
            .await
            .unwrap();

        let _ = set_book_authors(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(SetBookAuthorsRequest {
                authors: vec![BookAuthorLink {
                    author_id: replacement.id,
                    role: "author".into(),
                    position: 0,
                }],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(
            updated.resolution_state,
            ResolutionState::Done,
            "trusted book should stay Done after authors edit"
        );
        assert!(updated.metadata_user_trusted);
        assert_eq!(updated.metadata_status, MetadataStatus::Identified,);
    }

    #[tokio::test]
    async fn untrust_plus_title_change_enters_pipeline() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Untrust Title").await;

        // Trust first
        trust_book(&state, book.id).await;

        // Then untrust + edit title
        let _ = update_book(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(UpdateBookRequest {
                title: Some("Changed Title".into()),
                subtitle: None,
                description: None,
                language: None,
                publication_year: None,
                rating: None,
                page_count: None,
                publisher_id: None,
                metadata_user_trusted: Some(false),
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(
            updated.resolution_state,
            ResolutionState::Pending,
            "untrusted book should enter Pending after title edit"
        );
        assert!(!updated.metadata_user_trusted);
    }

    // ── Search sort-default regression ─────────────────────────

    #[tokio::test]
    async fn list_books_search_defaults_to_relevance_sort() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        // Seed two books; one matches in title, the other only in description
        let mut b1 = Book::new("Zzz Unrelated");
        b1.description = Some("sanderson reference".into());
        BookRepository::create(state.db_pool(), &b1).await.unwrap();

        let b2 = Book::new("Sanderson Novel");
        BookRepository::create(state.db_pool(), &b2).await.unwrap();

        // q is set, sort_by is omitted → should use relevance
        let params = BookListParams {
            q: Some("sanderson".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        // Title match should rank first (relevance sort)
        assert_eq!(result.items[0].title, "Sanderson Novel");
    }

    #[tokio::test]
    async fn list_books_search_with_explicit_title_sort() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut b1 = Book::new("Zebra");
        b1.description = Some("sanderson".into());
        BookRepository::create(state.db_pool(), &b1).await.unwrap();

        let mut b2 = Book::new("Alpha");
        b2.description = Some("sanderson".into());
        BookRepository::create(state.db_pool(), &b2).await.unwrap();

        let params = BookListParams {
            q: Some("sanderson".into()),
            sort_by: Some("title".into()),
            sort_order: Some("asc".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].title, "Alpha");
        assert_eq!(result.items[1].title, "Zebra");
    }

    #[tokio::test]
    async fn list_books_no_search_defaults_to_added_at() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        BookRepository::create(state.db_pool(), &Book::new("First"))
            .await
            .unwrap();
        BookRepository::create(state.db_pool(), &Book::new("Second"))
            .await
            .unwrap();

        let params = BookListParams::default();

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        // Default DESC: most recent first
        assert_eq!(result.items[0].title, "Second");
        assert_eq!(result.items[1].title, "First");
    }

    // ── ISBN filter regression ────────────────────────────────────

    #[tokio::test]
    async fn list_books_isbn_filter_matches_isbn13() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let book = Book::new("ISBN13 Target");
        BookRepository::create(pool, &book).await.unwrap();
        let other = Book::new("No Identifier");
        BookRepository::create(pool, &other).await.unwrap();

        let ident = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780451524935",
            MetadataSource::User,
            1.0,
        );
        archivis_db::IdentifierRepository::create(pool, &ident)
            .await
            .unwrap();

        let params = BookListParams {
            identifier_type: Some("isbn".into()),
            identifier_value: Some("9780451524935".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].title, "ISBN13 Target");
    }

    #[tokio::test]
    async fn list_books_isbn_filter_matches_isbn10() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let book = Book::new("ISBN10 Target");
        BookRepository::create(pool, &book).await.unwrap();

        let ident = Identifier::new(
            book.id,
            IdentifierType::Isbn10,
            "0451524934",
            MetadataSource::User,
            1.0,
        );
        archivis_db::IdentifierRepository::create(pool, &ident)
            .await
            .unwrap();

        let params = BookListParams {
            identifier_type: Some("isbn".into()),
            identifier_value: Some("0451524934".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].title, "ISBN10 Target");
    }

    #[tokio::test]
    async fn list_books_isbn_filter_normalizes_hyphens() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let book = Book::new("Hyphenated ISBN");
        BookRepository::create(pool, &book).await.unwrap();

        let ident = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9783161484100",
            MetadataSource::User,
            1.0,
        );
        archivis_db::IdentifierRepository::create(pool, &ident)
            .await
            .unwrap();

        // Pass ISBN with hyphens — the handler normalizes via `canonicalize()`
        let params = BookListParams {
            identifier_type: Some("isbn".into()),
            identifier_value: Some("978-3-16-148410-0".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].title, "Hyphenated ISBN");
    }

    #[tokio::test]
    async fn list_books_bare_isbn_query_matches_isbn13() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let book = Book::new("Bare ISBN13 Target");
        BookRepository::create(pool, &book).await.unwrap();
        let other = Book::new("No Identifier");
        BookRepository::create(pool, &other).await.unwrap();

        let ident = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780451524935",
            MetadataSource::User,
            1.0,
        );
        archivis_db::IdentifierRepository::create(pool, &ident)
            .await
            .unwrap();

        let params = BookListParams {
            q: Some("9780451524935".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].title, "Bare ISBN13 Target");
    }

    #[tokio::test]
    async fn list_books_bare_isbn_query_normalizes_hyphens() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let book = Book::new("Bare Hyphenated ISBN Target");
        BookRepository::create(pool, &book).await.unwrap();

        let ident = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9783161484100",
            MetadataSource::User,
            1.0,
        );
        archivis_db::IdentifierRepository::create(pool, &ident)
            .await
            .unwrap();

        let params = BookListParams {
            q: Some("978-3-16-148410-0".into()),
            ..BookListParams::default()
        };

        let Json(result) = list_books(State(state.clone()), auth_user(), Query(params))
            .await
            .unwrap();

        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].title, "Bare Hyphenated ISBN Target");
    }

    // ── Phase 4: Scope / Selection / Batch tests ────────────────

    /// Helper: extract a JSON body from a `Response`.
    async fn json_body<T: serde::de::DeserializeOwned>(resp: Response) -> (StatusCode, T) {
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: T = serde_json::from_slice(&bytes).unwrap();
        (status, body)
    }

    /// Helper: create N books for batch testing.
    async fn create_n_books(pool: &archivis_db::DbPool, n: usize) -> Vec<Uuid> {
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            let book = Book::new(format!("Batch Book {i}"));
            BookRepository::create(pool, &book).await.unwrap();
            ids.push(book.id);
        }
        ids
    }

    #[tokio::test]
    async fn scope_issuance_returns_correct_matching_count() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        // Create 5 books with language "en".
        for i in 0..5 {
            let mut book = Book::new(format!("EN Book {i}"));
            book.language = Some("en".into());
            BookRepository::create(pool, &book).await.unwrap();
        }
        // Create 3 books with language "de".
        for i in 0..3 {
            let mut book = Book::new(format!("DE Book {i}"));
            book.language = Some("de".into());
            BookRepository::create(pool, &book).await.unwrap();
        }

        let filter = archivis_core::models::LibraryFilterState {
            language: Some("en".into()),
            ..Default::default()
        };

        let Json(resp) = issue_selection_scope(
            State(state.clone()),
            auth_user(),
            Json(IssueSelectionScopeRequest {
                filters: filter.clone(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(resp.matching_count, 5);
        assert!(!resp.scope_token.is_empty());
    }

    #[tokio::test]
    async fn scope_exclusions_resolve_correctly() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let ids = create_n_books(pool, 10).await;

        // Issue scope for all books (no filter).
        let filter = archivis_core::models::LibraryFilterState::default();
        let Json(scope_resp) = issue_selection_scope(
            State(state.clone()),
            auth_user(),
            Json(IssueSelectionScopeRequest {
                filters: filter.clone(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(scope_resp.matching_count, 10);

        // Resolve with 3 exclusions → 7 books.
        let excluded = vec![ids[0], ids[3], ids[7]];
        let book_filter = BookFilter::from(&filter);
        let resolved = BookRepository::resolve_scope(pool, &book_filter, &excluded)
            .await
            .unwrap();

        assert_eq!(resolved.len(), 7);
        for ex_id in &excluded {
            assert!(!resolved.contains(ex_id));
        }
    }

    #[tokio::test]
    async fn ids_selection_lte_100_stays_synchronous() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let ids = create_n_books(pool, 3).await;

        let resp = batch_update_books(
            State(state.clone()),
            auth_user(),
            Json(BatchUpdateBooksRequest {
                selection: SelectionSpec::Ids { ids: ids.clone() },
                updates: super::super::types::BatchBookFields {
                    language: Some("fr".into()),
                    rating: None,
                    publisher_id: None,
                },
            }),
        )
        .await
        .unwrap();

        let (status, body): (_, BatchSyncResponse) = json_body(resp).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.updated_count, 3);
        assert!(body.errors.is_empty());

        // Verify books were actually updated.
        for &id in &ids {
            let book = BookRepository::get_by_id(pool, id).await.unwrap();
            assert_eq!(book.language.as_deref(), Some("fr"));
        }
    }

    #[tokio::test]
    async fn scope_selection_over_100_enqueues_async() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        // Create 101 books.
        create_n_books(pool, 101).await;

        // Issue scope token covering all.
        let filter = archivis_core::models::LibraryFilterState::default();
        let Json(scope_resp) = issue_selection_scope(
            State(state.clone()),
            auth_user(),
            Json(IssueSelectionScopeRequest {
                filters: filter.clone(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(scope_resp.matching_count, 101);

        let resp = batch_update_books(
            State(state.clone()),
            auth_user(),
            Json(BatchUpdateBooksRequest {
                selection: SelectionSpec::Scope {
                    scope_token: scope_resp.scope_token,
                    excluded_ids: vec![],
                },
                updates: super::super::types::BatchBookFields {
                    language: Some("de".into()),
                    rating: None,
                    publisher_id: None,
                },
            }),
        )
        .await
        .unwrap();

        let (status, body): (_, super::super::types::BatchAsyncResponse) = json_body(resp).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body.task_type, "bulk_update");
        assert_eq!(body.matching_count, 101);
    }

    #[tokio::test]
    async fn batch_tags_invalid_selection_does_not_create_tags() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let unique_tag = "should-not-exist-orphan-tag";

        // Send batch-tags with empty IDs (should fail validation) but a new tag name.
        let result = batch_set_tags(
            State(state.clone()),
            auth_user(),
            Json(BatchSetTagsRequest {
                selection: SelectionSpec::Ids { ids: vec![] },
                tags: vec![BookTagLink {
                    tag_id: None,
                    name: Some(unique_tag.into()),
                    category: None,
                }],
                mode: BatchTagMode::Add,
            }),
        )
        .await;

        assert!(result.is_err());

        // Verify no new tag was created by searching for it.
        let search_result = TagRepository::search(
            pool,
            Some(unique_tag),
            None,
            &archivis_db::PaginationParams::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            search_result.items.len(),
            0,
            "tag should not have been created on validation failure"
        );
    }

    // ── Scope counting: duplicate / out-of-scope exclusions ──────────

    #[tokio::test]
    async fn scope_count_ignores_duplicate_and_out_of_scope_exclusions() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let ids = create_n_books(pool, 5).await;
        let out_of_scope = Uuid::new_v4();

        let filter = archivis_core::models::LibraryFilterState::default();
        let Json(scope_resp) = issue_selection_scope(
            State(state.clone()),
            auth_user(),
            Json(IssueSelectionScopeRequest {
                filters: filter.clone(),
            }),
        )
        .await
        .unwrap();

        // Exclude: one valid ID (duplicated), plus one ID that is not in the scope.
        // Effective exclusion is 1 unique in-scope ID → 4 books affected.
        let excluded = vec![ids[0], ids[0], out_of_scope];

        let resp = batch_update_books(
            State(state.clone()),
            auth_user(),
            Json(BatchUpdateBooksRequest {
                selection: SelectionSpec::Scope {
                    scope_token: scope_resp.scope_token,
                    excluded_ids: excluded,
                },
                updates: super::super::types::BatchBookFields {
                    language: Some("fr".into()),
                    rating: None,
                    publisher_id: None,
                },
            }),
        )
        .await
        .unwrap();

        let (status, body): (_, BatchSyncResponse) = json_body(resp).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.updated_count, 4);
    }

    #[tokio::test]
    async fn scope_with_only_out_of_scope_exclusions_accepted() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        create_n_books(pool, 3).await;

        let filter = archivis_core::models::LibraryFilterState::default();
        let Json(scope_resp) = issue_selection_scope(
            State(state.clone()),
            auth_user(),
            Json(IssueSelectionScopeRequest {
                filters: filter.clone(),
            }),
        )
        .await
        .unwrap();

        // All exclusions are out-of-scope → no books removed, all 3 updated.
        let excluded = vec![Uuid::new_v4(), Uuid::new_v4()];

        let resp = batch_update_books(
            State(state.clone()),
            auth_user(),
            Json(BatchUpdateBooksRequest {
                selection: SelectionSpec::Scope {
                    scope_token: scope_resp.scope_token,
                    excluded_ids: excluded,
                },
                updates: super::super::types::BatchBookFields {
                    language: Some("fr".into()),
                    rating: None,
                    publisher_id: None,
                },
            }),
        )
        .await
        .unwrap();

        let (status, body): (_, BatchSyncResponse) = json_body(resp).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.updated_count, 3);
    }

    // ── Bulk update parity: shared function tests ────────────────────

    #[tokio::test]
    async fn bulk_update_rejects_invalid_language() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let ids = create_n_books(pool, 1).await;

        let fields = archivis_core::models::BulkUpdateFields {
            language: Some("Klingon".into()),
            rating: None,
            publisher_id: None,
        };

        let result = apply_bulk_update_to_book(pool, ids[0], &fields).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, BulkFieldError::Validation(_)),
            "expected Validation error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn bulk_update_stamps_provenance_and_refreshes_quality_score() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let ids = create_n_books(pool, 1).await;

        let fields = archivis_core::models::BulkUpdateFields {
            language: Some("fr".into()),
            rating: None,
            publisher_id: None,
        };

        let changed = apply_bulk_update_to_book(pool, ids[0], &fields)
            .await
            .unwrap();
        assert!(changed);

        let book = BookRepository::get_by_id(pool, ids[0]).await.unwrap();
        assert_eq!(book.language.as_deref(), Some("fr"));

        // Verify language provenance was stamped.
        let prov = book
            .metadata_provenance
            .language
            .as_ref()
            .expect("language provenance should be set");
        assert_eq!(prov.origin, MetadataSource::User);
        assert!(prov.protected);

        // Verify quality score was refreshed (computed and persisted).
        assert!(
            book.metadata_quality_score.is_some(),
            "quality score should have been refreshed"
        );
    }

    #[tokio::test]
    async fn bulk_update_stamps_publisher_provenance() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let ids = create_n_books(pool, 1).await;

        // Create a real publisher to satisfy the FK constraint.
        let publisher = archivis_core::models::Publisher::new("Test Publisher");
        archivis_db::PublisherRepository::create(pool, &publisher)
            .await
            .unwrap();
        let pub_id = publisher.id;

        let fields = archivis_core::models::BulkUpdateFields {
            language: None,
            rating: None,
            publisher_id: Some(Some(pub_id)),
        };

        let changed = apply_bulk_update_to_book(pool, ids[0], &fields)
            .await
            .unwrap();
        assert!(changed);

        let book = BookRepository::get_by_id(pool, ids[0]).await.unwrap();
        assert_eq!(book.publisher_id, Some(pub_id));

        let prov = book
            .metadata_provenance
            .publisher
            .as_ref()
            .expect("publisher provenance should be set");
        assert_eq!(prov.origin, MetadataSource::User);
        assert!(prov.protected);
    }

    // ── Empty-field DSL regression tests ────────────────────────────

    #[tokio::test]
    async fn list_books_empty_field_returns_200_with_warning() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let result = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("author:".into()),
                ..Default::default()
            }),
        )
        .await;

        let resp = result.expect("should be 200, not 500");
        assert!(
            resp.search_warnings
                .iter()
                .any(|w| matches!(w, QueryWarningResponse::EmptyFieldValue { field } if field == "author")),
            "expected EmptyFieldValue warning for author, got: {:?}",
            resp.search_warnings,
        );
    }

    #[tokio::test]
    async fn list_books_multiple_empty_fields() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let result = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("author: series:".into()),
                ..Default::default()
            }),
        )
        .await;

        let resp = result.expect("should be 200, not 500");
        assert_eq!(
            resp.search_warnings.len(),
            2,
            "expected 2 warnings, got: {:?}",
            resp.search_warnings,
        );
    }

    #[tokio::test]
    async fn list_books_empty_field_regression_all_types() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        for q in ["author:", "series:", "publisher:", "tag:", "title:"] {
            let result = list_books(
                State(state.clone()),
                auth_user(),
                Query(BookListParams {
                    q: Some(q.into()),
                    ..Default::default()
                }),
            )
            .await;

            assert!(result.is_ok(), "q={q:?} returned error: {:?}", result.err());
        }
    }

    // ── Unsupported field-OR DSL regression tests ────────────────

    #[tokio::test]
    async fn list_books_field_or_returns_200_with_unsupported_warning() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let result = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("author:asimov OR author:clarke".into()),
                ..Default::default()
            }),
        )
        .await;

        let resp = result.expect("should be 200, not 500");
        assert_eq!(
            resp.search_warnings
                .iter()
                .filter(|w| matches!(w, QueryWarningResponse::UnsupportedOrField { .. }))
                .count(),
            2,
            "expected 2 UnsupportedOrField warnings, got: {:?}",
            resp.search_warnings,
        );
    }

    #[tokio::test]
    async fn list_books_mixed_or_keeps_text_warns_field() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let result = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("dune OR author:asimov".into()),
                ..Default::default()
            }),
        )
        .await;

        let resp = result.expect("should be 200, not 500");
        assert!(
            resp.search_warnings.iter().any(|w| matches!(
                w,
                QueryWarningResponse::UnsupportedOrField { field, .. } if field == "author"
            )),
            "expected UnsupportedOrField warning, got: {:?}",
            resp.search_warnings,
        );
    }

    #[tokio::test]
    async fn list_books_negated_field_or_warns() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let result = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("dune OR -author:asimov".into()),
                ..Default::default()
            }),
        )
        .await;

        let resp = result.expect("should be 200, not 500");
        assert!(
            resp.search_warnings.iter().any(|w| matches!(
                w,
                QueryWarningResponse::UnsupportedOrField { negated, .. } if *negated
            )),
            "expected UnsupportedOrField with negated=true, got: {:?}",
            resp.search_warnings,
        );
    }

    // ── Incomplete-token DSL regression tests ────────────────────

    #[tokio::test]
    async fn list_books_incomplete_tokens_return_200() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        for q in ["-", "OR", "\"\"", "   ", "- "] {
            let result = list_books(
                State(state.clone()),
                auth_user(),
                Query(BookListParams {
                    q: Some(q.into()),
                    ..Default::default()
                }),
            )
            .await;
            assert!(result.is_ok(), "q={q:?} should return 200, not 500");
        }
    }

    // ── Non-searchable text queries (punctuation-only) ──────────────

    #[tokio::test]
    async fn list_books_double_dash_returns_200_with_warning() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        BookRepository::create(state.db_pool(), &Book::new("Existing Book"))
            .await
            .unwrap();

        let result = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("--".into()),
                ..Default::default()
            }),
        )
        .await;

        let resp = result.expect("q=-- should return 200, not 500");
        assert_eq!(resp.total, 0);
        assert!(resp.items.is_empty());
        assert!(
            resp.search_warnings.iter().any(|w| matches!(
                w,
                QueryWarningResponse::NoSearchableTerms { field, .. } if field.is_none()
            )),
            "expected NoSearchableTerms warning, got: {:?}",
            resp.search_warnings,
        );
    }

    #[tokio::test]
    async fn list_books_punctuation_only_queries_return_200() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        for q in ["--", "---", "\"...\"", "NOT --"] {
            let result = list_books(
                State(state.clone()),
                auth_user(),
                Query(BookListParams {
                    q: Some(q.into()),
                    ..Default::default()
                }),
            )
            .await;
            assert!(result.is_ok(), "q={q:?} should return 200, not 500");
        }
    }

    #[tokio::test]
    async fn list_books_partial_punctuation_still_searches() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        for q in ["dune OR --", "-- OR dune"] {
            let result = list_books(
                State(state.clone()),
                auth_user(),
                Query(BookListParams {
                    q: Some(q.into()),
                    ..Default::default()
                }),
            )
            .await;
            assert!(result.is_ok(), "q={q:?} should return 200, not 500");
            // No NoSearchableTerms warning — there are still searchable terms.
            let resp = result.unwrap();
            assert!(
                !resp.search_warnings.iter().any(|w| matches!(
                    w,
                    QueryWarningResponse::NoSearchableTerms { field, .. } if field.is_none()
                )),
                "q={q:?} should not emit NoSearchableTerms for text when valid terms remain",
            );
        }
    }

    #[tokio::test]
    async fn list_books_column_filter_punctuation_returns_200_with_warning() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        BookRepository::create(state.db_pool(), &Book::new("Existing Book"))
            .await
            .unwrap();

        for q in ["title:--", "title:\"...\""] {
            let result = list_books(
                State(state.clone()),
                auth_user(),
                Query(BookListParams {
                    q: Some(q.into()),
                    ..Default::default()
                }),
            )
            .await;

            let resp = result.unwrap_or_else(|_| panic!("q={q:?} should return 200, not 500"));
            assert_eq!(resp.total, 0, "q={q:?} should fail closed");
            assert!(
                resp.items.is_empty(),
                "q={q:?} should not fall back to the default library list",
            );
            assert!(
                resp.search_warnings.iter().any(|w| matches!(
                    w,
                    QueryWarningResponse::NoSearchableTerms { field, .. } if field.as_deref() == Some("title")
                )),
                "q={q:?} expected NoSearchableTerms warning with field=title, got: {:?}",
                resp.search_warnings,
            );
        }
    }

    #[tokio::test]
    async fn list_books_double_dash_word_returns_200_with_warning_and_no_results() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        BookRepository::create(state.db_pool(), &Book::new("Existing Book"))
            .await
            .unwrap();

        let Json(resp) = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("--x".into()),
                ..Default::default()
            }),
        )
        .await
        .expect("q=--x should return 200, not 500");

        assert_eq!(resp.total, 0);
        assert!(resp.items.is_empty());
        assert!(
            resp.search_warnings.iter().any(|w| matches!(
                w,
                QueryWarningResponse::NoSearchableTerms { field, text }
                    if field.is_none() && text == "--x"
            )),
            "expected NoSearchableTerms warning for --x, got: {:?}",
            resp.search_warnings,
        );
    }

    #[tokio::test]
    async fn list_books_cplusplus_search_is_targeted() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        BookRepository::create(pool, &Book::new("C++ Programming"))
            .await
            .unwrap();
        BookRepository::create(pool, &Book::new("C Primer"))
            .await
            .unwrap();
        BookRepository::create(pool, &Book::new("Cat Tales"))
            .await
            .unwrap();

        let Json(resp) = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("c++".into()),
                ..Default::default()
            }),
        )
        .await
        .expect("q=c++ should return 200");

        assert_eq!(resp.total, 1);
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].title, "C++ Programming");
    }

    #[tokio::test]
    async fn list_books_title_cplusplus_search_is_targeted() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        let title_match = Book::new("C++ Cookbook");
        BookRepository::create(pool, &title_match).await.unwrap();

        let mut desc_only = Book::new("Reference Book");
        desc_only.description = Some("C++ concepts".into());
        BookRepository::create(pool, &desc_only).await.unwrap();

        let Json(resp) = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("title:c++".into()),
                ..Default::default()
            }),
        )
        .await
        .expect("q=title:c++ should return 200");

        assert_eq!(resp.total, 1);
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].title, "C++ Cookbook");
    }

    #[tokio::test]
    async fn list_books_hyphenated_search_is_targeted() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let pool = state.db_pool();

        BookRepository::create(pool, &Book::new("Some-Hyphenated-Phrase"))
            .await
            .unwrap();
        BookRepository::create(pool, &Book::new("Some Hyphenated Phrase"))
            .await
            .unwrap();

        let Json(resp) = list_books(
            State(state.clone()),
            auth_user(),
            Query(BookListParams {
                q: Some("some-hyphenated-phrase".into()),
                ..Default::default()
            }),
        )
        .await
        .expect("hyphenated query should return 200");

        assert!(
            resp.items
                .iter()
                .any(|item| item.title == "Some-Hyphenated-Phrase"),
            "hyphenated search should find the intended title"
        );
    }
}
