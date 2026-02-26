CREATE TABLE reading_progress (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    book_file_id TEXT NOT NULL REFERENCES book_files(id) ON DELETE CASCADE,
    location TEXT,
    progress REAL NOT NULL DEFAULT 0.0
        CHECK (progress >= 0.0 AND progress <= 1.0),
    device_id TEXT,
    preferences TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(user_id, book_file_id, device_id)
);
CREATE INDEX idx_reading_progress_user_book ON reading_progress(user_id, book_id);
CREATE INDEX idx_reading_progress_updated ON reading_progress(updated_at);

CREATE TABLE bookmarks (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    book_file_id TEXT NOT NULL REFERENCES book_files(id) ON DELETE CASCADE,
    location TEXT NOT NULL,
    label TEXT,
    excerpt TEXT,
    position REAL NOT NULL DEFAULT 0.0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX idx_bookmarks_user_book ON bookmarks(user_id, book_id);

CREATE TRIGGER reading_progress_updated_at AFTER UPDATE ON reading_progress
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE reading_progress SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
