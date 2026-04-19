use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::service::{SettingEntry, SettingError};

#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsResponse {
    pub settings: Vec<SettingEntry>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    pub settings: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateSettingsResponse {
    pub updated: Vec<String>,
    pub requires_restart: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateSettingsErrorResponse {
    pub errors: Vec<SettingError>,
}
