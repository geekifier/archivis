CREATE TABLE watched_directories (
    id TEXT PRIMARY KEY NOT NULL,
    path TEXT NOT NULL UNIQUE,
    -- User's explicit choice. 'native' or 'poll'. No 'auto' — the user always decides,
    -- informed by the fs detection hint shown in the UI.
    watch_mode TEXT NOT NULL DEFAULT 'poll'
        CHECK (watch_mode IN ('native', 'poll')),
    -- Polling interval for this path in seconds. NULL = use global default.
    poll_interval_secs INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    -- Last error from the watcher for this path (NULL = healthy).
    -- Surfaced in the API/UI so users can diagnose issues.
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TRIGGER watched_directories_updated_at AFTER UPDATE ON watched_directories
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE watched_directories SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;

-- Seed global watcher defaults in the settings table (JSON-encoded values).
INSERT INTO settings (key, value) VALUES
    ('watcher.debounce_ms', '"2000"'),
    ('watcher.default_poll_interval_secs', '"30"'),
    ('watcher.delete_source_after_import', '"false"')
ON CONFLICT(key) DO NOTHING;
