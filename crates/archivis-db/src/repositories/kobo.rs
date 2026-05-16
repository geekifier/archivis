use archivis_core::errors::DbError;
use archivis_core::models::{KoboDevice, KoboDeviceSyncItem, KoboSyncSelection};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Devices ────────────────────────────────────────────────────────────

pub struct KoboDeviceRepository;

impl KoboDeviceRepository {
    pub async fn create(pool: &SqlitePool, device: &KoboDevice) -> Result<(), DbError> {
        let id = device.id.to_string();
        let user_id = device.user_id.to_string();
        let created_at = device.created_at.to_rfc3339();
        let last_seen_at = device.last_seen_at.map(|d| d.to_rfc3339());
        let revoked_at = device.revoked_at.map(|d| d.to_rfc3339());

        sqlx::query!(
            "INSERT INTO kobo_devices (id, user_id, token_hash, display_name, created_at, last_seen_at, revoked_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            id,
            user_id,
            device.token_hash,
            device.display_name,
            created_at,
            last_seen_at,
            revoked_at,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<KoboDevice, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            KoboDeviceRow,
            "SELECT id, user_id, token_hash, display_name, created_at, last_seen_at, revoked_at
             FROM kobo_devices WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "kobo_device",
            id: id_str,
        })?;
        row.into_device()
    }

    pub async fn get_active_by_token_hash(
        pool: &SqlitePool,
        token_hash: &str,
    ) -> Result<KoboDevice, DbError> {
        let row = sqlx::query_as!(
            KoboDeviceRow,
            "SELECT id, user_id, token_hash, display_name, created_at, last_seen_at, revoked_at
             FROM kobo_devices WHERE token_hash = ? AND revoked_at IS NULL",
            token_hash,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or_else(|| DbError::NotFound {
            entity: "kobo_device",
            id: "by_token_hash".to_string(),
        })?;
        row.into_device()
    }

    pub async fn list_for_user(
        pool: &SqlitePool,
        user_id: Uuid,
    ) -> Result<Vec<KoboDevice>, DbError> {
        let user_id_str = user_id.to_string();
        let rows = sqlx::query_as!(
            KoboDeviceRow,
            "SELECT id, user_id, token_hash, display_name, created_at, last_seen_at, revoked_at
             FROM kobo_devices WHERE user_id = ? ORDER BY created_at ASC",
            user_id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        rows.into_iter().map(KoboDeviceRow::into_device).collect()
    }

    /// Mark a device as revoked. Returns `true` if the device was active and
    /// got revoked; `false` if it was already revoked or doesn't exist.
    pub async fn revoke(pool: &SqlitePool, id: Uuid, user_id: Uuid) -> Result<bool, DbError> {
        let id_str = id.to_string();
        let user_id_str = user_id.to_string();
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query!(
            "UPDATE kobo_devices SET revoked_at = ?
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
            now,
            id_str,
            user_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn touch_last_seen(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query!(
            "UPDATE kobo_devices SET last_seen_at = ? WHERE id = ?",
            now,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }
}

// ── Selections ─────────────────────────────────────────────────────────

pub struct KoboSyncSelectionRepository;

impl KoboSyncSelectionRepository {
    /// Insert or update the user's selection for a book. The trigger updates
    /// `updated_at` only when the row content changes; we explicitly reset it
    /// here for changes to `selected_book_file_id`, since `SQLite`'s trigger
    /// fires only when `NEW.updated_at == OLD.updated_at`.
    pub async fn upsert(
        pool: &SqlitePool,
        user_id: Uuid,
        book_id: Uuid,
        selected_book_file_id: Option<Uuid>,
    ) -> Result<KoboSyncSelection, DbError> {
        let user_id_str = user_id.to_string();
        let book_id_str = book_id.to_string();
        let file_id_str = selected_book_file_id.map(|u| u.to_string());
        let now = Utc::now().to_rfc3339();

        sqlx::query!(
            "INSERT INTO kobo_sync_selections (user_id, book_id, selected_book_file_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(user_id, book_id) DO UPDATE SET
                 selected_book_file_id = excluded.selected_book_file_id,
                 updated_at = excluded.updated_at",
            user_id_str,
            book_id_str,
            file_id_str,
            now,
            now,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Self::get(pool, user_id, book_id).await
    }

    pub async fn get(
        pool: &SqlitePool,
        user_id: Uuid,
        book_id: Uuid,
    ) -> Result<KoboSyncSelection, DbError> {
        let user_id_str = user_id.to_string();
        let book_id_str = book_id.to_string();
        let row = sqlx::query_as!(
            KoboSyncSelectionRow,
            "SELECT user_id, book_id, selected_book_file_id, created_at, updated_at
             FROM kobo_sync_selections WHERE user_id = ? AND book_id = ?",
            user_id_str,
            book_id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or_else(|| DbError::NotFound {
            entity: "kobo_sync_selection",
            id: format!("{user_id}/{book_id}"),
        })?;
        row.into_selection()
    }

    pub async fn find(
        pool: &SqlitePool,
        user_id: Uuid,
        book_id: Uuid,
    ) -> Result<Option<KoboSyncSelection>, DbError> {
        match Self::get(pool, user_id, book_id).await {
            Ok(s) => Ok(Some(s)),
            Err(DbError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn delete(pool: &SqlitePool, user_id: Uuid, book_id: Uuid) -> Result<bool, DbError> {
        let user_id_str = user_id.to_string();
        let book_id_str = book_id.to_string();
        let result = sqlx::query!(
            "DELETE FROM kobo_sync_selections WHERE user_id = ? AND book_id = ?",
            user_id_str,
            book_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    /// Return all selections for the user that have a non-null
    /// `selected_book_file_id` (i.e. desired set, before further joins).
    pub async fn list_active_for_user(
        pool: &SqlitePool,
        user_id: Uuid,
    ) -> Result<Vec<KoboSyncSelection>, DbError> {
        let user_id_str = user_id.to_string();
        let rows = sqlx::query_as!(
            KoboSyncSelectionRow,
            "SELECT user_id, book_id, selected_book_file_id, created_at, updated_at
             FROM kobo_sync_selections
             WHERE user_id = ? AND selected_book_file_id IS NOT NULL
             ORDER BY book_id ASC",
            user_id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        rows.into_iter()
            .map(KoboSyncSelectionRow::into_selection)
            .collect()
    }

    /// Return a bounded page of active selections that may need to emit a
    /// New/Changed entitlement for a device.
    ///
    /// The SQL predicate intentionally over-selects metadata changes via
    /// `books.updated_at`; callers still recompute the canonical revision hash
    /// before deciding whether to emit.
    pub async fn list_sync_candidate_page(
        pool: &SqlitePool,
        user_id: Uuid,
        device_id: Uuid,
        limit: i64,
    ) -> Result<Vec<KoboSyncSelection>, DbError> {
        let user_id_str = user_id.to_string();
        let device_id_str = device_id.to_string();
        let rows = sqlx::query_as::<_, KoboSyncSelectionRow>(
            "SELECT s.user_id, s.book_id, s.selected_book_file_id, s.created_at, s.updated_at
             FROM kobo_sync_selections s
             JOIN books b ON b.id = s.book_id
             JOIN book_files f
               ON f.id = s.selected_book_file_id
              AND f.book_id = s.book_id
              AND lower(f.format) = 'epub'
             LEFT JOIN kobo_device_sync_items l
               ON l.device_id = ?
              AND l.book_id = s.book_id
             WHERE s.user_id = ?
               AND s.selected_book_file_id IS NOT NULL
               AND (
                    l.device_id IS NULL
                 OR l.removed_synced_at IS NOT NULL
                 OR l.book_file_id IS NULL
                 OR l.book_file_id != f.id
                 OR l.file_hash IS NULL
                 OR l.file_hash != f.hash
                 OR l.selection_updated_at IS NULL
                 OR l.selection_updated_at != s.updated_at
                 OR l.delivered_at IS NULL
                 OR datetime(b.updated_at) > datetime(l.delivered_at)
               )
             ORDER BY s.book_id ASC
             LIMIT ?",
        )
        .bind(device_id_str)
        .bind(user_id_str)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(KoboSyncSelectionRow::into_selection)
            .collect()
    }
}

// ── Device Sync Ledger ─────────────────────────────────────────────────

pub struct KoboDeviceSyncItemRepository;

impl KoboDeviceSyncItemRepository {
    /// List all ledger rows for a device.
    pub async fn list_for_device(
        pool: &SqlitePool,
        device_id: Uuid,
    ) -> Result<Vec<KoboDeviceSyncItem>, DbError> {
        let id_str = device_id.to_string();
        let rows = sqlx::query_as!(
            KoboDeviceSyncItemRow,
            "SELECT device_id, book_id, book_file_id, file_hash, desired_revision_hash,
                    selection_updated_at, delivered_at, removed_at, removed_synced_at
             FROM kobo_device_sync_items WHERE device_id = ? ORDER BY book_id ASC",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        rows.into_iter()
            .map(KoboDeviceSyncItemRow::into_item)
            .collect()
    }

    /// Return a bounded page of device ledger rows that no longer have a
    /// current valid desired selection and therefore may need a removal
    /// entitlement.
    pub async fn list_removal_candidate_page(
        pool: &SqlitePool,
        device_id: Uuid,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<KoboDeviceSyncItem>, DbError> {
        let device_id_str = device_id.to_string();
        let user_id_str = user_id.to_string();
        let rows = sqlx::query_as::<_, KoboDeviceSyncItemRow>(
            "SELECT l.device_id, l.book_id, l.book_file_id, l.file_hash, l.desired_revision_hash,
                    l.selection_updated_at, l.delivered_at, l.removed_at, l.removed_synced_at
             FROM kobo_device_sync_items l
             WHERE l.device_id = ?
               AND l.removed_synced_at IS NULL
               AND NOT EXISTS (
                    SELECT 1
                    FROM kobo_sync_selections s
                    JOIN book_files f
                      ON f.id = s.selected_book_file_id
                     AND f.book_id = s.book_id
                     AND lower(f.format) = 'epub'
                    WHERE s.user_id = ?
                      AND s.book_id = l.book_id
                      AND s.selected_book_file_id IS NOT NULL
               )
             ORDER BY l.book_id ASC
             LIMIT ?",
        )
        .bind(device_id_str)
        .bind(user_id_str)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(KoboDeviceSyncItemRow::into_item)
            .collect()
    }

    /// Upsert a delivered ledger row (after emitting a New or Changed
    /// entitlement). Always clears `removed_at`/`removed_synced_at` so a
    /// previously tombstoned row is brought back to active.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_delivered(
        pool: &SqlitePool,
        device_id: Uuid,
        book_id: Uuid,
        book_file_id: Option<Uuid>,
        file_hash: Option<&str>,
        desired_revision_hash: &str,
        selection_updated_at: DateTime<Utc>,
        delivered_at: DateTime<Utc>,
    ) -> Result<(), DbError> {
        let device_id_str = device_id.to_string();
        let book_id_str = book_id.to_string();
        let file_id_str = book_file_id.map(|u| u.to_string());
        let selection_ts = selection_updated_at.to_rfc3339();
        let delivered_ts = delivered_at.to_rfc3339();

        sqlx::query!(
            "INSERT INTO kobo_device_sync_items
                (device_id, book_id, book_file_id, file_hash, desired_revision_hash,
                 selection_updated_at, delivered_at, removed_at, removed_synced_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL)
             ON CONFLICT(device_id, book_id) DO UPDATE SET
                 book_file_id = excluded.book_file_id,
                 file_hash = excluded.file_hash,
                 desired_revision_hash = excluded.desired_revision_hash,
                 selection_updated_at = excluded.selection_updated_at,
                 delivered_at = excluded.delivered_at,
                 removed_at = NULL,
                 removed_synced_at = NULL",
            device_id_str,
            book_id_str,
            file_id_str,
            file_hash,
            desired_revision_hash,
            selection_ts,
            delivered_ts,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Mark a ledger row as removed and acknowledged in one go (the device
    /// has just received the removed entitlement in the current sync page).
    pub async fn mark_tombstoned(
        pool: &SqlitePool,
        device_id: Uuid,
        book_id: Uuid,
        when: DateTime<Utc>,
    ) -> Result<(), DbError> {
        let device_id_str = device_id.to_string();
        let book_id_str = book_id.to_string();
        let when_str = when.to_rfc3339();
        sqlx::query!(
            "UPDATE kobo_device_sync_items
             SET removed_at = COALESCE(removed_at, ?),
                 removed_synced_at = ?
             WHERE device_id = ? AND book_id = ?",
            when_str,
            when_str,
            device_id_str,
            book_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Find a single ledger row, if any.
    pub async fn find(
        pool: &SqlitePool,
        device_id: Uuid,
        book_id: Uuid,
    ) -> Result<Option<KoboDeviceSyncItem>, DbError> {
        let device_id_str = device_id.to_string();
        let book_id_str = book_id.to_string();
        let row = sqlx::query_as!(
            KoboDeviceSyncItemRow,
            "SELECT device_id, book_id, book_file_id, file_hash, desired_revision_hash,
                    selection_updated_at, delivered_at, removed_at, removed_synced_at
             FROM kobo_device_sync_items WHERE device_id = ? AND book_id = ?",
            device_id_str,
            book_id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        row.map(KoboDeviceSyncItemRow::into_item).transpose()
    }

    /// Delete acknowledged tombstones older than the cutoff.
    pub async fn delete_acknowledged_tombstones_before(
        pool: &SqlitePool,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, DbError> {
        let cutoff_str = cutoff.to_rfc3339();
        let result = sqlx::query!(
            "DELETE FROM kobo_device_sync_items
             WHERE removed_synced_at IS NOT NULL AND removed_synced_at < ?",
            cutoff_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

// ── Row types ──────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct KoboDeviceRow {
    id: String,
    user_id: String,
    token_hash: String,
    display_name: String,
    created_at: String,
    last_seen_at: Option<String>,
    revoked_at: Option<String>,
}

impl KoboDeviceRow {
    fn into_device(self) -> Result<KoboDevice, DbError> {
        let id = parse_uuid(&self.id, "kobo_device.id")?;
        let user_id = parse_uuid(&self.user_id, "kobo_device.user_id")?;
        let created_at = parse_datetime(&self.created_at, "kobo_device.created_at")?;
        let last_seen_at = self
            .last_seen_at
            .map(|s| parse_datetime(&s, "kobo_device.last_seen_at"))
            .transpose()?;
        let revoked_at = self
            .revoked_at
            .map(|s| parse_datetime(&s, "kobo_device.revoked_at"))
            .transpose()?;
        Ok(KoboDevice {
            id,
            user_id,
            token_hash: self.token_hash,
            display_name: self.display_name,
            created_at,
            last_seen_at,
            revoked_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct KoboSyncSelectionRow {
    user_id: String,
    book_id: String,
    selected_book_file_id: Option<String>,
    created_at: String,
    updated_at: String,
}

impl KoboSyncSelectionRow {
    fn into_selection(self) -> Result<KoboSyncSelection, DbError> {
        let user_id = parse_uuid(&self.user_id, "kobo_sync_selection.user_id")?;
        let book_id = parse_uuid(&self.book_id, "kobo_sync_selection.book_id")?;
        let selected_book_file_id = self
            .selected_book_file_id
            .map(|s| parse_uuid(&s, "kobo_sync_selection.selected_book_file_id"))
            .transpose()?;
        let created_at = parse_datetime(&self.created_at, "kobo_sync_selection.created_at")?;
        let updated_at = parse_datetime(&self.updated_at, "kobo_sync_selection.updated_at")?;
        Ok(KoboSyncSelection {
            user_id,
            book_id,
            selected_book_file_id,
            created_at,
            updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct KoboDeviceSyncItemRow {
    device_id: String,
    book_id: String,
    book_file_id: Option<String>,
    file_hash: Option<String>,
    desired_revision_hash: Option<String>,
    selection_updated_at: Option<String>,
    delivered_at: Option<String>,
    removed_at: Option<String>,
    removed_synced_at: Option<String>,
}

impl KoboDeviceSyncItemRow {
    fn into_item(self) -> Result<KoboDeviceSyncItem, DbError> {
        let device_id = parse_uuid(&self.device_id, "kobo_device_sync_item.device_id")?;
        let book_id = parse_uuid(&self.book_id, "kobo_device_sync_item.book_id")?;
        let book_file_id = self
            .book_file_id
            .map(|s| parse_uuid(&s, "kobo_device_sync_item.book_file_id"))
            .transpose()?;
        let selection_updated_at = self
            .selection_updated_at
            .map(|s| parse_datetime(&s, "kobo_device_sync_item.selection_updated_at"))
            .transpose()?;
        let delivered_at = self
            .delivered_at
            .map(|s| parse_datetime(&s, "kobo_device_sync_item.delivered_at"))
            .transpose()?;
        let removed_at = self
            .removed_at
            .map(|s| parse_datetime(&s, "kobo_device_sync_item.removed_at"))
            .transpose()?;
        let removed_synced_at = self
            .removed_synced_at
            .map(|s| parse_datetime(&s, "kobo_device_sync_item.removed_synced_at"))
            .transpose()?;
        Ok(KoboDeviceSyncItem {
            device_id,
            book_id,
            book_file_id,
            file_hash: self.file_hash,
            desired_revision_hash: self.desired_revision_hash,
            selection_updated_at,
            delivered_at,
            removed_at,
            removed_synced_at,
        })
    }
}

fn parse_uuid(s: &str, field: &str) -> Result<Uuid, DbError> {
    Uuid::parse_str(s).map_err(|e| DbError::Query(format!("invalid {field}: {e}")))
}

fn parse_datetime(s: &str, field: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|ndt| ndt.and_utc())
        })
        .map_err(|e| DbError::Query(format!("invalid {field}: {e}")))
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::book::BookRepository;
    use crate::repositories::book_file::BookFileRepository;
    use crate::repositories::user::UserRepository;
    use crate::{create_pool, run_migrations};
    use archivis_core::models::{Book, BookFile, BookFormat, User, UserRole};
    use sqlx::SqlitePool;
    use tempfile::TempDir;

    async fn test_pool() -> (SqlitePool, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("kobo.db");
        let pool = create_pool(&db).await.unwrap();
        run_migrations(&pool).await.unwrap();
        (pool, dir)
    }

    async fn make_user(pool: &SqlitePool, name: &str) -> User {
        let user = User::new(name.into(), "hash".into(), UserRole::User);
        UserRepository::create(pool, &user).await.unwrap();
        user
    }

    async fn make_book_with_epub(pool: &SqlitePool) -> (Book, BookFile) {
        let book = Book::new("Sample");
        BookRepository::create(pool, &book).await.unwrap();
        let file = BookFile::new(
            book.id,
            BookFormat::Epub,
            format!("{}.epub", book.id),
            12345,
            format!("hash-{}", book.id),
            Some("3.0".into()),
        );
        BookFileRepository::create(pool, &file).await.unwrap();
        (book, file)
    }

    fn make_device(user_id: Uuid, token_hash: &str) -> KoboDevice {
        KoboDevice {
            id: Uuid::new_v4(),
            user_id,
            token_hash: token_hash.into(),
            display_name: "Test Kobo".into(),
            created_at: Utc::now(),
            last_seen_at: None,
            revoked_at: None,
        }
    }

    #[tokio::test]
    async fn device_token_lookup_finds_active_device() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let device = make_device(user.id, "hash-1");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();

        let found = KoboDeviceRepository::get_active_by_token_hash(&pool, "hash-1")
            .await
            .unwrap();
        assert_eq!(found.id, device.id);
    }

    #[tokio::test]
    async fn revoked_device_cannot_authenticate() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let device = make_device(user.id, "hash-r");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();
        assert!(KoboDeviceRepository::revoke(&pool, device.id, user.id)
            .await
            .unwrap());
        assert!(matches!(
            KoboDeviceRepository::get_active_by_token_hash(&pool, "hash-r").await,
            Err(DbError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn revoke_is_idempotent_returns_false_second_time() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let device = make_device(user.id, "hash-x");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();
        assert!(KoboDeviceRepository::revoke(&pool, device.id, user.id)
            .await
            .unwrap());
        assert!(!KoboDeviceRepository::revoke(&pool, device.id, user.id)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn selection_upsert_replaces_file_id() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let (book, file) = make_book_with_epub(&pool).await;

        let s1 = KoboSyncSelectionRepository::upsert(&pool, user.id, book.id, Some(file.id))
            .await
            .unwrap();
        assert_eq!(s1.selected_book_file_id, Some(file.id));

        let other_file = BookFile::new(
            book.id,
            BookFormat::Epub,
            "other.epub",
            999,
            "hash-other",
            None,
        );
        BookFileRepository::create(&pool, &other_file)
            .await
            .unwrap();

        let s2 = KoboSyncSelectionRepository::upsert(&pool, user.id, book.id, Some(other_file.id))
            .await
            .unwrap();
        assert_eq!(s2.selected_book_file_id, Some(other_file.id));
    }

    #[tokio::test]
    async fn deleting_selected_file_nulls_the_selection() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let (book, file) = make_book_with_epub(&pool).await;

        KoboSyncSelectionRepository::upsert(&pool, user.id, book.id, Some(file.id))
            .await
            .unwrap();

        BookFileRepository::delete(&pool, file.id).await.unwrap();

        let sel = KoboSyncSelectionRepository::get(&pool, user.id, book.id)
            .await
            .unwrap();
        assert_eq!(sel.selected_book_file_id, None);
    }

    #[tokio::test]
    async fn book_deletion_cascades_selection_but_keeps_ledger_row() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let (book, file) = make_book_with_epub(&pool).await;
        let device = make_device(user.id, "hash-d");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();

        KoboSyncSelectionRepository::upsert(&pool, user.id, book.id, Some(file.id))
            .await
            .unwrap();
        KoboDeviceSyncItemRepository::upsert_delivered(
            &pool,
            device.id,
            book.id,
            Some(file.id),
            Some(&file.hash),
            "rev-1",
            Utc::now(),
            Utc::now(),
        )
        .await
        .unwrap();

        BookRepository::delete(&pool, book.id).await.unwrap();

        // Selection cascaded.
        assert!(matches!(
            KoboSyncSelectionRepository::get(&pool, user.id, book.id).await,
            Err(DbError::NotFound { .. })
        ));
        // Ledger row preserved (it doesn't FK back to books).
        let row = KoboDeviceSyncItemRepository::find(&pool, device.id, book.id)
            .await
            .unwrap()
            .expect("ledger row preserved");
        assert_eq!(row.book_id, book.id);
    }

    #[tokio::test]
    async fn upsert_delivered_clears_tombstone_on_reselect() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let (book, file) = make_book_with_epub(&pool).await;
        let device = make_device(user.id, "hash-d");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();

        // Initial delivery, then tombstone the row, then re-deliver.
        KoboDeviceSyncItemRepository::upsert_delivered(
            &pool,
            device.id,
            book.id,
            Some(file.id),
            Some(&file.hash),
            "rev-1",
            Utc::now(),
            Utc::now(),
        )
        .await
        .unwrap();
        KoboDeviceSyncItemRepository::mark_tombstoned(&pool, device.id, book.id, Utc::now())
            .await
            .unwrap();

        KoboDeviceSyncItemRepository::upsert_delivered(
            &pool,
            device.id,
            book.id,
            Some(file.id),
            Some(&file.hash),
            "rev-2",
            Utc::now(),
            Utc::now(),
        )
        .await
        .unwrap();

        let row = KoboDeviceSyncItemRepository::find(&pool, device.id, book.id)
            .await
            .unwrap()
            .unwrap();
        assert!(row.removed_at.is_none());
        assert!(row.removed_synced_at.is_none());
        assert_eq!(row.desired_revision_hash.as_deref(), Some("rev-2"));
    }

    #[tokio::test]
    async fn cleanup_purges_only_acknowledged_old_tombstones() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let (book, file) = make_book_with_epub(&pool).await;
        let device = make_device(user.id, "hash-d");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();

        KoboDeviceSyncItemRepository::upsert_delivered(
            &pool,
            device.id,
            book.id,
            Some(file.id),
            Some(&file.hash),
            "rev-1",
            Utc::now(),
            Utc::now(),
        )
        .await
        .unwrap();
        // Mark tombstoned a long time ago.
        let old = Utc::now() - chrono::Duration::days(180);
        KoboDeviceSyncItemRepository::mark_tombstoned(&pool, device.id, book.id, old)
            .await
            .unwrap();

        // Cleanup with cutoff at 90 days ago.
        let cutoff = Utc::now() - chrono::Duration::days(90);
        let deleted =
            KoboDeviceSyncItemRepository::delete_acknowledged_tombstones_before(&pool, cutoff)
                .await
                .unwrap();
        assert_eq!(deleted, 1);
        assert!(
            KoboDeviceSyncItemRepository::find(&pool, device.id, book.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn cleanup_does_not_purge_unacknowledged_tombstones() {
        let (pool, _dir) = test_pool().await;
        let user = make_user(&pool, "alice").await;
        let (book, file) = make_book_with_epub(&pool).await;
        let device = make_device(user.id, "hash-d");
        KoboDeviceRepository::create(&pool, &device).await.unwrap();

        KoboDeviceSyncItemRepository::upsert_delivered(
            &pool,
            device.id,
            book.id,
            Some(file.id),
            Some(&file.hash),
            "rev-1",
            Utc::now(),
            Utc::now(),
        )
        .await
        .unwrap();

        // Direct UPDATE so we have removed_at set without removed_synced_at.
        let device_id_str = device.id.to_string();
        let book_id_str = book.id.to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query!(
            "UPDATE kobo_device_sync_items SET removed_at = ?, removed_synced_at = NULL
             WHERE device_id = ? AND book_id = ?",
            now,
            device_id_str,
            book_id_str,
        )
        .execute(&pool)
        .await
        .unwrap();

        let cutoff = Utc::now() - chrono::Duration::days(90);
        let deleted =
            KoboDeviceSyncItemRepository::delete_acknowledged_tombstones_before(&pool, cutoff)
                .await
                .unwrap();
        assert_eq!(deleted, 0);

        let row = KoboDeviceSyncItemRepository::find(&pool, device.id, book.id)
            .await
            .unwrap();
        assert!(row.is_some(), "unacknowledged tombstone must survive");
    }
}
