use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ── User-facing device-management API ────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct PairDeviceRequest {
    /// Display name for the new device, e.g. "Kobo Libra".
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PairDeviceResponse {
    pub id: Uuid,
    pub display_name: String,
    /// Raw pairing token. Returned exactly once at pairing time.
    pub token: String,
    /// Full Kobo API endpoint to configure on the device, e.g.
    /// `https://example.test/kobo/<token>`.
    pub api_endpoint: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeviceResponse {
    pub id: Uuid,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct KoboStatusResponse {
    pub enabled: bool,
    pub active_device_count: usize,
    pub device_count: usize,
}

// ── Selection API ────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertSelectionRequest {
    /// Whether sync is enabled. Always `true` from the first UI; included for
    /// future-proofing toward an explicit disable-via-PUT path.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional explicit EPUB file id to sync. When omitted, the backend
    /// chooses a deterministic EPUB.
    #[serde(default)]
    pub book_file_id: Option<Uuid>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct KoboSyncStateResponse {
    pub enabled: bool,
    pub selected_book_file_id: Option<Uuid>,
    pub eligible_file_ids: Vec<Uuid>,
    pub stale: bool,
    pub reason: Option<String>,
}
