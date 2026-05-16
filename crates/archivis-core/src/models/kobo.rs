use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A Kobo device paired to an Archivis user. Only the SHA-256 hash of the
/// pairing token is stored; the raw token is shown to the user once at
/// pairing time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KoboDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl KoboDevice {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// A user's intent to sync a particular book to their Kobo devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KoboSyncSelection {
    pub user_id: Uuid,
    pub book_id: Uuid,
    /// `None` once the originally selected file has been deleted (the row
    /// is preserved as a stale selection, excluded from the desired set).
    pub selected_book_file_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Per-device ledger row tracking what has been delivered to a Kobo device,
/// and what (if anything) is currently a pending tombstone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KoboDeviceSyncItem {
    pub device_id: Uuid,
    pub book_id: Uuid,
    pub book_file_id: Option<Uuid>,
    pub file_hash: Option<String>,
    pub desired_revision_hash: Option<String>,
    pub selection_updated_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub removed_at: Option<DateTime<Utc>>,
    pub removed_synced_at: Option<DateTime<Utc>>,
}

impl KoboDeviceSyncItem {
    pub fn is_tombstone(&self) -> bool {
        self.removed_at.is_some()
    }

    pub fn tombstone_acknowledged(&self) -> bool {
        self.removed_synced_at.is_some()
    }
}
