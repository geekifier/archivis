use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::filter::LibraryFilterState;

/// Payload stored in the task table for bulk operations.
///
/// Shared between `archivis-api` (enqueue side) and `archivis-tasks` (worker side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkTaskPayload {
    pub filter: LibraryFilterState,
    pub excluded_ids: Vec<Uuid>,
    pub operation: BulkOperation,
}

/// The operation a bulk task should perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkOperation {
    Update {
        fields: BulkUpdateFields,
    },
    SetTags {
        mode: BulkTagMode,
        tags: Vec<BulkTagEntry>,
    },
}

/// Scalar fields for bulk update. All optional — only `Some` fields are applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkUpdateFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f32>,
    /// `Some(None)` = clear publisher, `Some(Some(id))` = set publisher, `None` = no change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_id: Option<Option<Uuid>>,
}

/// Mode for batch tag operations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BulkTagMode {
    /// Clear all existing tags and set these.
    Replace,
    /// Add tags without removing existing ones.
    Add,
}

/// A resolved tag reference for bulk operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkTagEntry {
    pub tag_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_operation_update_serde_roundtrip() {
        let op = BulkOperation::Update {
            fields: BulkUpdateFields {
                language: Some("en".into()),
                rating: Some(4.5),
                publisher_id: None,
            },
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"update""#));
        let deserialized: BulkOperation = serde_json::from_str(&json).unwrap();
        match deserialized {
            BulkOperation::Update { fields } => {
                assert_eq!(fields.language.as_deref(), Some("en"));
                assert_eq!(fields.rating, Some(4.5));
            }
            BulkOperation::SetTags { .. } => panic!("expected Update variant"),
        }
    }

    #[test]
    fn bulk_operation_set_tags_serde_roundtrip() {
        let tag_id = Uuid::new_v4();
        let op = BulkOperation::SetTags {
            mode: BulkTagMode::Add,
            tags: vec![BulkTagEntry { tag_id }],
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""type":"set_tags""#));
        let deserialized: BulkOperation = serde_json::from_str(&json).unwrap();
        match deserialized {
            BulkOperation::SetTags { mode, tags } => {
                assert!(matches!(mode, BulkTagMode::Add));
                assert_eq!(tags.len(), 1);
                assert_eq!(tags[0].tag_id, tag_id);
            }
            BulkOperation::Update { .. } => panic!("expected SetTags variant"),
        }
    }

    #[test]
    fn bulk_task_payload_serde_roundtrip() {
        let payload = BulkTaskPayload {
            filter: LibraryFilterState::default(),
            excluded_ids: vec![Uuid::new_v4()],
            operation: BulkOperation::Update {
                fields: BulkUpdateFields {
                    language: None,
                    rating: None,
                    publisher_id: Some(None),
                },
            },
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: BulkTaskPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.excluded_ids.len(), 1);
        assert_eq!(deserialized.excluded_ids[0], payload.excluded_ids[0]);
    }

    #[test]
    fn clear_publisher_serializes_as_null() {
        let fields = BulkUpdateFields {
            language: None,
            rating: None,
            publisher_id: Some(None),
        };
        let json = serde_json::to_string(&fields).unwrap();
        assert!(json.contains(r#""publisher_id":null"#));
    }
}
