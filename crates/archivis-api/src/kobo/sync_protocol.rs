//! Kobo protocol sync diff algorithm and JSON response types.
//!
//! Wire shape mirrors the Kobo store's responses (as documented by
//! calibre-web's Kobo sync handlers, used as a field-level reference).
//! Internal Rust types use idiomatic names; serialization uses `PascalCase`
//! to match the protocol.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use archivis_core::models::{Book, BookFile, KoboDeviceSyncItem, KoboSyncSelection};

// ── Protocol response shapes ────────────────────────────────────────

/// Top-level `/v1/library/sync` element. Each element is a single entitlement
/// envelope; the list is heterogeneous, so each variant is its own object key.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SyncEntry {
    New(NewEntitlementEnvelope),
    Changed(ChangedEntitlementEnvelope),
    Removed(RemovedEntitlementEnvelope),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct NewEntitlementEnvelope {
    pub new_entitlement: EntitlementBundle,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChangedEntitlementEnvelope {
    pub changed_entitlement: EntitlementBundle,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemovedEntitlementEnvelope {
    pub changed_entitlement: EntitlementBundle,
}

/// Bundle of entitlement + metadata + reading state. Reading state is empty
/// in this iteration (no position sync).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct EntitlementBundle {
    pub book_entitlement: BookEntitlement,
    pub book_metadata: BookMetadata,
    pub reading_state: ReadingState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BookEntitlement {
    pub accessibility: &'static str,
    pub active_period: ActivePeriod,
    pub created: DateTime<Utc>,
    pub cross_revision_id: Uuid,
    pub id: Uuid,
    pub is_hidden_from_archive: bool,
    pub is_locked: bool,
    pub is_removed: bool,
    pub last_modified: DateTime<Utc>,
    pub origin_category: &'static str,
    pub revision_id: String,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ActivePeriod {
    pub from: DateTime<Utc>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BookMetadata {
    #[serde(rename = "RevisionId")]
    pub revision_id: String,
    #[serde(rename = "CrossRevisionId")]
    pub cross_revision_id: Uuid,
    pub work_id: Uuid,
    pub entitlement_id: Uuid,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub language: String,
    pub contributors: Vec<String>,
    pub contributor_roles: Vec<ContributorRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Publisher>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime<Utc>>,
    pub current_display_price: DisplayPrice,
    pub current_love_display_price: DisplayPrice,
    pub download_urls: Vec<DownloadUrl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_image_id: Option<Uuid>,
    pub external_ids: Vec<String>,
    pub genre: &'static str,
    pub is_internet_archived: bool,
    pub is_pre_order: bool,
    pub is_social_enabled: bool,
    pub is_eligible_for_kobo_love: bool,
    pub phonetic_pronunciations: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContributorRole {
    pub name: String,
    pub role: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Publisher {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imprint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DisplayPrice {
    #[serde(rename = "CurrencyCode")]
    pub currency_code: &'static str,
    #[serde(rename = "TotalAmount")]
    pub total_amount: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DownloadUrl {
    pub format: &'static str,
    pub size: i64,
    pub url: String,
    pub platform: &'static str,
    pub drm_type: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReadingState {
    pub entitlement_id: Uuid,
    pub created: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    pub priority_timestamp: DateTime<Utc>,
    pub status_info: StatusInfo,
    pub statistics: Statistics,
    pub current_bookmark: serde_json::Value,
    pub location: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct StatusInfo {
    pub last_modified: DateTime<Utc>,
    pub status: &'static str,
    pub times_started_reading: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Statistics {
    pub last_modified: DateTime<Utc>,
    pub spent_reading_minutes: i32,
    pub remaining_time_minutes: i32,
}

// ── Diff inputs / outputs ───────────────────────────────────────────

/// One desired item to be considered for the sync diff. The caller pre-joins
/// `selection` ↔ `book` ↔ `book_file`.
pub struct DesiredItem {
    pub selection: KoboSyncSelection,
    pub book: Book,
    pub book_file: BookFile,
}

/// What the diff decides to do for a particular `book_id`.
#[allow(clippy::large_enum_variant)]
pub enum DiffOutcome {
    Emit(SyncEntry, LedgerWrite),
    Skip,
}

pub enum LedgerWrite {
    /// Upsert delivered ledger row with the given fields.
    Upsert {
        book_id: Uuid,
        book_file_id: Option<Uuid>,
        file_hash: Option<String>,
        desired_revision_hash: String,
        selection_updated_at: DateTime<Utc>,
        delivered_at: DateTime<Utc>,
    },
    /// Mark a tombstone delivered.
    Tombstone { book_id: Uuid, when: DateTime<Utc> },
}

/// Compute the desired-revision hash for an item. Uses the rendered metadata
/// inputs plus `book_file_id`, `file_hash`, and `selection_updated_at`. This
/// is more robust than comparing only `books.updated_at`.
pub fn compute_revision_hash(item: &DesiredItem) -> String {
    let mut hasher = Sha256::new();
    hasher.update(item.book.id.as_bytes());
    hasher.update(item.book.title.as_bytes());
    if let Some(ref s) = item.book.subtitle {
        hasher.update(b"|sub:");
        hasher.update(s.as_bytes());
    }
    if let Some(ref d) = item.book.description {
        hasher.update(b"|desc:");
        hasher.update(d.as_bytes());
    }
    if let Some(ref l) = item.book.language {
        hasher.update(b"|lang:");
        hasher.update(l.as_bytes());
    }
    if let Some(year) = item.book.publication_year {
        hasher.update(b"|year:");
        hasher.update(year.to_le_bytes());
    }
    if let Some(ref cover_path) = item.book.cover_path {
        hasher.update(b"|cover:");
        hasher.update(cover_path.as_bytes());
    }
    hasher.update(b"|file:");
    hasher.update(item.book_file.id.as_bytes());
    hasher.update(b"|hash:");
    hasher.update(item.book_file.hash.as_bytes());
    hasher.update(b"|selupd:");
    hasher.update(item.selection.updated_at.timestamp_millis().to_le_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(result.len() * 2);
    for b in result {
        write!(s, "{b:02x}").expect("hex encoding cannot fail");
    }
    s
}

/// Decide what to do for one currently-desired item.
pub fn diff_desired(
    item: &DesiredItem,
    existing: Option<&KoboDeviceSyncItem>,
    download_url: String,
    now: DateTime<Utc>,
) -> DiffOutcome {
    let revision_hash = compute_revision_hash(item);

    let needs_emit = existing.is_none_or(|row| {
        row.removed_synced_at.is_some()
            || row.desired_revision_hash.as_deref() != Some(revision_hash.as_str())
            || row.book_file_id != Some(item.book_file.id)
            || row.file_hash.as_deref() != Some(item.book_file.hash.as_str())
    });

    if !needs_emit {
        return DiffOutcome::Skip;
    }

    let is_change = matches!(existing, Some(row) if row.delivered_at.is_some() && row.removed_synced_at.is_none());

    let bundle = build_bundle(item, &revision_hash, download_url);
    let entry = if is_change {
        SyncEntry::Changed(ChangedEntitlementEnvelope {
            changed_entitlement: bundle,
        })
    } else {
        SyncEntry::New(NewEntitlementEnvelope {
            new_entitlement: bundle,
        })
    };

    DiffOutcome::Emit(
        entry,
        LedgerWrite::Upsert {
            book_id: item.book.id,
            book_file_id: Some(item.book_file.id),
            file_hash: Some(item.book_file.hash.clone()),
            desired_revision_hash: revision_hash,
            selection_updated_at: item.selection.updated_at,
            delivered_at: now,
        },
    )
}

/// Build a removed entitlement for a ledger row that is no longer desired.
pub fn diff_removal(row: &KoboDeviceSyncItem, now: DateTime<Utc>) -> DiffOutcome {
    let bundle = build_removal_bundle(row);
    DiffOutcome::Emit(
        SyncEntry::Removed(RemovedEntitlementEnvelope {
            changed_entitlement: bundle,
        }),
        LedgerWrite::Tombstone {
            book_id: row.book_id,
            when: now,
        },
    )
}

pub fn build_bundle(
    item: &DesiredItem,
    revision_hash: &str,
    download_url: String,
) -> EntitlementBundle {
    let book_id = item.book.id;
    let now = Utc::now();
    let publication_date = item.book.publication_year.and_then(|y| {
        chrono::NaiveDate::from_ymd_opt(y, 1, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|ndt| ndt.and_utc())
    });

    EntitlementBundle {
        book_entitlement: BookEntitlement {
            accessibility: "Full",
            active_period: ActivePeriod {
                from: item.book.added_at,
            },
            created: item.book.added_at,
            cross_revision_id: book_id,
            id: book_id,
            is_hidden_from_archive: false,
            is_locked: false,
            is_removed: false,
            last_modified: item.book.updated_at,
            origin_category: "Imported",
            revision_id: revision_hash.to_string(),
            status: "Active",
        },
        book_metadata: BookMetadata {
            revision_id: revision_hash.to_string(),
            cross_revision_id: book_id,
            work_id: book_id,
            entitlement_id: book_id,
            title: item.book.title.clone(),
            subtitle: item.book.subtitle.clone(),
            description: item.book.description.clone(),
            language: item
                .book
                .language
                .clone()
                .unwrap_or_else(|| "en".to_string()),
            contributors: Vec::new(),
            contributor_roles: Vec::new(),
            publisher: None,
            publication_date,
            current_display_price: DisplayPrice {
                currency_code: "USD",
                total_amount: 0.0,
            },
            current_love_display_price: DisplayPrice {
                currency_code: "LVE",
                total_amount: 0.0,
            },
            download_urls: vec![DownloadUrl {
                format: "KEPUB",
                size: item.book_file.file_size,
                url: download_url,
                platform: "Android",
                drm_type: "None",
            }],
            cover_image_id: item.book.cover_path.as_ref().map(|_| book_id),
            external_ids: Vec::new(),
            genre: "00000000-0000-0000-0000-000000000001",
            is_internet_archived: false,
            is_pre_order: false,
            is_social_enabled: false,
            is_eligible_for_kobo_love: false,
            phonetic_pronunciations: serde_json::json!({}),
        },
        reading_state: ReadingState {
            entitlement_id: book_id,
            created: item.book.added_at,
            last_modified: now,
            priority_timestamp: now,
            status_info: StatusInfo {
                last_modified: now,
                status: "ReadyToRead",
                times_started_reading: 0,
            },
            statistics: Statistics {
                last_modified: now,
                spent_reading_minutes: 0,
                remaining_time_minutes: 0,
            },
            current_bookmark: serde_json::json!({}),
            location: serde_json::json!({}),
        },
    }
}

fn build_removal_bundle(row: &KoboDeviceSyncItem) -> EntitlementBundle {
    let book_id = row.book_id;
    let now = Utc::now();
    EntitlementBundle {
        book_entitlement: BookEntitlement {
            accessibility: "Full",
            active_period: ActivePeriod { from: now },
            created: row.delivered_at.unwrap_or(now),
            cross_revision_id: book_id,
            id: book_id,
            is_hidden_from_archive: true,
            is_locked: false,
            is_removed: true,
            last_modified: now,
            origin_category: "Imported",
            revision_id: row.desired_revision_hash.clone().unwrap_or_default(),
            status: "Active",
        },
        book_metadata: BookMetadata {
            revision_id: row.desired_revision_hash.clone().unwrap_or_default(),
            cross_revision_id: book_id,
            work_id: book_id,
            entitlement_id: book_id,
            title: String::new(),
            subtitle: None,
            description: None,
            language: "en".to_string(),
            contributors: Vec::new(),
            contributor_roles: Vec::new(),
            publisher: None,
            publication_date: None,
            current_display_price: DisplayPrice {
                currency_code: "USD",
                total_amount: 0.0,
            },
            current_love_display_price: DisplayPrice {
                currency_code: "LVE",
                total_amount: 0.0,
            },
            download_urls: Vec::new(),
            cover_image_id: None,
            external_ids: Vec::new(),
            genre: "00000000-0000-0000-0000-000000000001",
            is_internet_archived: false,
            is_pre_order: false,
            is_social_enabled: false,
            is_eligible_for_kobo_love: false,
            phonetic_pronunciations: serde_json::json!({}),
        },
        reading_state: ReadingState {
            entitlement_id: book_id,
            created: now,
            last_modified: now,
            priority_timestamp: now,
            status_info: StatusInfo {
                last_modified: now,
                status: "ReadyToRead",
                times_started_reading: 0,
            },
            statistics: Statistics {
                last_modified: now,
                spent_reading_minutes: 0,
                remaining_time_minutes: 0,
            },
            current_bookmark: serde_json::json!({}),
            location: serde_json::json!({}),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archivis_core::models::{BookFormat, MetadataStatus, ResolutionState};

    fn fixture_book(id: Uuid, title: &str, updated: DateTime<Utc>) -> Book {
        let mut book = Book::new(title);
        book.id = id;
        book.added_at = updated;
        book.updated_at = updated;
        book.language = Some("en".into());
        book.publication_year = Some(2024);
        book.metadata_status = MetadataStatus::Identified;
        book.resolution_state = ResolutionState::Done;
        book
    }

    fn fixture_file(book_id: Uuid, hash: &str, size: i64) -> BookFile {
        BookFile {
            id: Uuid::new_v4(),
            book_id,
            format: BookFormat::Epub,
            format_version: Some("3.0".into()),
            storage_path: format!("{book_id}.epub"),
            file_size: size,
            hash: hash.into(),
            added_at: Utc::now(),
        }
    }

    fn fixture_selection(book_id: Uuid, file_id: Uuid, when: DateTime<Utc>) -> KoboSyncSelection {
        KoboSyncSelection {
            user_id: Uuid::new_v4(),
            book_id,
            selected_book_file_id: Some(file_id),
            created_at: when,
            updated_at: when,
        }
    }

    #[test]
    fn revision_hash_changes_with_inputs() {
        let book_id = Uuid::new_v4();
        let when = Utc::now();
        let book = fixture_book(book_id, "T", when);
        let file = fixture_file(book_id, "abc", 100);
        let sel = fixture_selection(book_id, file.id, when);

        let h1 = compute_revision_hash(&DesiredItem {
            selection: sel.clone(),
            book: book.clone(),
            book_file: file.clone(),
        });

        let mut book2 = book.clone();
        book2.title = "Different".into();
        let h2 = compute_revision_hash(&DesiredItem {
            selection: sel.clone(),
            book: book2,
            book_file: file,
        });
        assert_ne!(h1, h2);

        let file2 = fixture_file(book_id, "def", 200);
        let h3 = compute_revision_hash(&DesiredItem {
            selection: sel,
            book,
            book_file: file2,
        });
        assert_ne!(h1, h3);
    }

    #[test]
    fn first_emission_is_new_entitlement() {
        let book_id = Uuid::new_v4();
        let when = Utc::now();
        let book = fixture_book(book_id, "T", when);
        let file = fixture_file(book_id, "abc", 100);
        let sel = fixture_selection(book_id, file.id, when);
        let item = DesiredItem {
            selection: sel,
            book,
            book_file: file,
        };

        let outcome = diff_desired(&item, None, "https://x/dl".into(), Utc::now());
        match outcome {
            DiffOutcome::Emit(SyncEntry::New(_), _) => {}
            _ => panic!("expected new entitlement"),
        }
    }

    #[test]
    fn unchanged_emits_skip() {
        let book_id = Uuid::new_v4();
        let when = Utc::now();
        let book = fixture_book(book_id, "T", when);
        let file = fixture_file(book_id, "abc", 100);
        let sel = fixture_selection(book_id, file.id, when);
        let file_id = file.id;
        let file_hash = file.hash.clone();
        let item = DesiredItem {
            selection: sel,
            book,
            book_file: file,
        };
        let revision = compute_revision_hash(&item);

        let row = KoboDeviceSyncItem {
            device_id: Uuid::new_v4(),
            book_id,
            book_file_id: Some(file_id),
            file_hash: Some(file_hash),
            desired_revision_hash: Some(revision),
            selection_updated_at: Some(when),
            delivered_at: Some(when),
            removed_at: None,
            removed_synced_at: None,
        };

        let outcome = diff_desired(&item, Some(&row), "https://x/dl".into(), Utc::now());
        assert!(matches!(outcome, DiffOutcome::Skip));
    }

    #[test]
    fn changed_revision_emits_changed() {
        let book_id = Uuid::new_v4();
        let when = Utc::now();
        let book = fixture_book(book_id, "T", when);
        let file = fixture_file(book_id, "abc", 100);
        let sel = fixture_selection(book_id, file.id, when);
        let file_id = file.id;
        let item = DesiredItem {
            selection: sel,
            book,
            book_file: file,
        };

        let row = KoboDeviceSyncItem {
            device_id: Uuid::new_v4(),
            book_id,
            book_file_id: Some(file_id),
            file_hash: Some("OLD".into()),
            desired_revision_hash: Some("OLD-REV".into()),
            selection_updated_at: Some(when),
            delivered_at: Some(when),
            removed_at: None,
            removed_synced_at: None,
        };

        let outcome = diff_desired(&item, Some(&row), "https://x/dl".into(), Utc::now());
        match outcome {
            DiffOutcome::Emit(SyncEntry::Changed(_), _) => {}
            _ => panic!("expected changed entitlement"),
        }
    }

    #[test]
    fn reselect_after_tombstone_acknowledged_emits_new() {
        let book_id = Uuid::new_v4();
        let when = Utc::now();
        let book = fixture_book(book_id, "T", when);
        let file = fixture_file(book_id, "abc", 100);
        let sel = fixture_selection(book_id, file.id, when);
        let file_id = file.id;
        let file_hash = file.hash.clone();
        let item = DesiredItem {
            selection: sel,
            book,
            book_file: file,
        };

        let row = KoboDeviceSyncItem {
            device_id: Uuid::new_v4(),
            book_id,
            book_file_id: Some(file_id),
            file_hash: Some(file_hash),
            desired_revision_hash: Some(compute_revision_hash(&item)),
            selection_updated_at: Some(when),
            delivered_at: Some(when),
            removed_at: Some(when),
            removed_synced_at: Some(when),
        };

        let outcome = diff_desired(&item, Some(&row), "https://x/dl".into(), Utc::now());
        match outcome {
            DiffOutcome::Emit(SyncEntry::New(_), _) => {}
            _ => panic!("expected new entitlement after tombstone ack"),
        }
    }
}
