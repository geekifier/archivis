use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request body for creating a new metadata rule.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMetadataRuleRequest {
    /// Rule type, e.g. `"publisher"`.
    pub rule_type: String,
    /// Value to match against (e.g. a publisher name).
    pub match_value: String,
    /// How to compare: `"exact"` or `"contains"`. Defaults to `"exact"`.
    pub match_mode: Option<String>,
    /// Action when matched: `"trust_metadata"`. Defaults to `"trust_metadata"`.
    pub outcome: Option<String>,
}

/// Request body for updating an existing metadata rule.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMetadataRuleRequest {
    /// New match value. `None` = don't change.
    pub match_value: Option<String>,
    /// New match mode: `"exact"` or `"contains"`. `None` = don't change.
    pub match_mode: Option<String>,
    /// Enable or disable the rule. `None` = don't change.
    pub enabled: Option<bool>,
}

/// Response representing a single metadata rule.
#[derive(Debug, Serialize, ToSchema)]
pub struct MetadataRuleResponse {
    /// Unique identifier.
    pub id: String,
    /// Rule type, e.g. `"publisher"`.
    pub rule_type: String,
    /// Value to match against.
    pub match_value: String,
    /// Match mode: `"exact"` or `"contains"`.
    pub match_mode: String,
    /// Outcome when matched: `"trust_metadata"`.
    pub outcome: String,
    /// Whether this rule is active.
    pub enabled: bool,
    /// Whether this rule is a built-in default.
    pub builtin: bool,
    /// ISO 8601 timestamp of when this rule was created.
    pub created_at: String,
}
