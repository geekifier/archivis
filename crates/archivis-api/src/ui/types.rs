use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct SidebarCountsResponse {
    pub duplicates: i64,
    pub needs_review: i64,
    pub unidentified: i64,
}
