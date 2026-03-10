use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::{MatchMode, MetadataRule, MetadataRuleType, RuleOutcome};
use archivis_db::MetadataRuleRepository;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{CreateMetadataRuleRequest, MetadataRuleResponse, UpdateMetadataRuleRequest};

/// Build a `MetadataRuleResponse` from a domain model.
fn to_response(rule: &MetadataRule) -> MetadataRuleResponse {
    MetadataRuleResponse {
        id: rule.id.to_string(),
        rule_type: rule.rule_type.to_string(),
        match_value: rule.match_value.clone(),
        match_mode: rule.match_mode.to_string(),
        outcome: rule.outcome.to_string(),
        enabled: rule.enabled,
        builtin: rule.builtin,
        created_at: rule.created_at.to_rfc3339(),
    }
}

// -- Handlers -----------------------------------------------------------------

/// GET /api/metadata-rules -- list all metadata rules.
///
/// Returns both enabled and disabled rules, ordered by builtin first then by
/// creation time.
#[utoipa::path(
    get,
    path = "/api/metadata-rules",
    tag = "metadata-rules",
    responses(
        (status = 200, description = "List of metadata rules", body = Vec<MetadataRuleResponse>),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_metadata_rules(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<Vec<MetadataRuleResponse>>, ApiError> {
    let rules = MetadataRuleRepository::list_all(state.db_pool()).await?;

    let responses: Vec<MetadataRuleResponse> = rules.iter().map(to_response).collect();

    Ok(Json(responses))
}

/// POST /api/metadata-rules -- create a new metadata rule.
///
/// Validates enum fields and creates the rule. Returns 201 on success.
#[utoipa::path(
    post,
    path = "/api/metadata-rules",
    tag = "metadata-rules",
    request_body = CreateMetadataRuleRequest,
    responses(
        (status = 201, description = "Metadata rule created", body = MetadataRuleResponse),
        (status = 400, description = "Validation error (invalid enum values)"),
        (status = 401, description = "Not authenticated"),
        (status = 409, description = "Rule already exists for this type and value"),
    ),
    security(("bearer" = []))
)]
pub async fn create_metadata_rule(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<CreateMetadataRuleRequest>,
) -> Result<(StatusCode, Json<MetadataRuleResponse>), ApiError> {
    let rule_type: MetadataRuleType = body
        .rule_type
        .parse()
        .map_err(|e: String| ApiError::Validation(e))?;

    let match_mode: MatchMode = body
        .match_mode
        .as_deref()
        .unwrap_or("exact")
        .parse()
        .map_err(|e: String| ApiError::Validation(e))?;

    let outcome: RuleOutcome = body
        .outcome
        .as_deref()
        .unwrap_or("trust_metadata")
        .parse()
        .map_err(|e: String| ApiError::Validation(e))?;

    let rule = MetadataRuleRepository::create(
        state.db_pool(),
        rule_type,
        &body.match_value,
        match_mode,
        outcome,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(to_response(&rule))))
}

/// PUT /api/metadata-rules/{id} -- update an existing metadata rule.
///
/// Can change `match_value`, `match_mode`, and `enabled`. Cannot change
/// `rule_type` or `outcome` (delete and re-create instead).
#[utoipa::path(
    put,
    path = "/api/metadata-rules/{id}",
    tag = "metadata-rules",
    params(("id" = Uuid, Path, description = "Metadata rule ID")),
    request_body = UpdateMetadataRuleRequest,
    responses(
        (status = 200, description = "Updated metadata rule", body = MetadataRuleResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Metadata rule not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_metadata_rule(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AxumPath(id): AxumPath<Uuid>,
    Json(body): Json<UpdateMetadataRuleRequest>,
) -> Result<Json<MetadataRuleResponse>, ApiError> {
    let new_match_mode: Option<MatchMode> = body
        .match_mode
        .as_deref()
        .map(|s| s.parse().map_err(|e: String| ApiError::Validation(e)))
        .transpose()?;

    let updated = MetadataRuleRepository::update(
        state.db_pool(),
        id,
        body.match_value.as_deref(),
        new_match_mode,
        body.enabled,
    )
    .await?;

    Ok(Json(to_response(&updated)))
}

/// DELETE /api/metadata-rules/{id} -- delete a metadata rule.
///
/// Built-in rules cannot be deleted (returns 409). Does NOT affect any
/// previously resolved books.
#[utoipa::path(
    delete,
    path = "/api/metadata-rules/{id}",
    tag = "metadata-rules",
    params(("id" = Uuid, Path, description = "Metadata rule ID")),
    responses(
        (status = 204, description = "Metadata rule deleted"),
        (status = 404, description = "Metadata rule not found"),
        (status = 409, description = "Cannot delete a builtin rule"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_metadata_rule(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AxumPath(id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    MetadataRuleRepository::delete(state.db_pool(), id).await?;

    Ok(StatusCode::NO_CONTENT)
}
