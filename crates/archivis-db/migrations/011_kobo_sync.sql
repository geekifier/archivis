-- Kobo Sync schema.
--
-- See `.docs/plans/kobo-plan-3C.md` (Schema section) for the rationale.

CREATE TABLE kobo_devices (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT,
    revoked_at TEXT
);

CREATE INDEX idx_kobo_devices_user_id ON kobo_devices(user_id);
CREATE INDEX idx_kobo_devices_token_hash ON kobo_devices(token_hash);

CREATE TABLE kobo_sync_selections (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    selected_book_file_id TEXT REFERENCES book_files(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (user_id, book_id)
);

CREATE INDEX idx_kobo_sync_selections_user_id
    ON kobo_sync_selections(user_id);

CREATE INDEX idx_kobo_sync_selections_selected_file
    ON kobo_sync_selections(selected_book_file_id);

CREATE TRIGGER kobo_sync_selections_updated_at
AFTER UPDATE ON kobo_sync_selections
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE kobo_sync_selections
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE user_id = NEW.user_id AND book_id = NEW.book_id;
END;

-- Per-device delivery ledger. `book_id` and `book_file_id` are intentionally
-- stored as plain TEXT without FK constraints so the ledger survives
-- book/file deletion and remains the tombstone source for devices that
-- already received the item.
CREATE TABLE kobo_device_sync_items (
    device_id TEXT NOT NULL REFERENCES kobo_devices(id) ON DELETE CASCADE,
    book_id TEXT NOT NULL,
    book_file_id TEXT,
    file_hash TEXT,
    desired_revision_hash TEXT,
    selection_updated_at TEXT,
    delivered_at TEXT,
    removed_at TEXT,
    removed_synced_at TEXT,
    PRIMARY KEY (device_id, book_id)
);

CREATE INDEX idx_kobo_device_sync_items_device
    ON kobo_device_sync_items(device_id);

CREATE INDEX idx_kobo_device_sync_items_removed
    ON kobo_device_sync_items(device_id, removed_at, removed_synced_at);
