use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadingProgressResponse {
    pub id: String,
    pub book_id: String,
    pub book_file_id: String,
    pub location: Option<String>,
    pub progress: f64,
    pub device_id: Option<String>,
    pub preferences: Option<serde_json::Value>,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProgressRequest {
    pub location: Option<String>,
    pub progress: f64,
    pub device_id: Option<String>,
    pub preferences: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContinueReadingItem {
    pub book_id: String,
    pub book_title: String,
    pub book_file_id: String,
    pub file_format: String,
    pub progress: f64,
    pub location: Option<String>,
    pub has_cover: bool,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBookmarkRequest {
    pub location: String,
    pub label: Option<String>,
    pub excerpt: Option<String>,
    pub position: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BookmarkResponse {
    pub id: String,
    pub location: String,
    pub label: Option<String>,
    pub excerpt: Option<String>,
    pub position: f64,
    pub created_at: String,
}
