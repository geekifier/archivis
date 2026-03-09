-- Archivis initial schema (consolidated).
-- SQLite with WAL mode (set via PRAGMA, not migration).

-- ────────────────────────────────────────────────────────────────
-- Publishers & Books
-- ────────────────────────────────────────────────────────────────

CREATE TABLE publishers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL
);

CREATE TABLE books (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT NOT NULL,
    description TEXT,
    language TEXT,
    publication_year INTEGER,
    publisher_id TEXT REFERENCES publishers(id) ON DELETE SET NULL,
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    rating REAL CHECK (rating IS NULL OR (rating >= 0.0 AND rating <= 5.0)),
    page_count INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    metadata_status TEXT NOT NULL DEFAULT 'unidentified'
        CHECK (metadata_status IN ('identified', 'needs_review', 'unidentified')),
    ingest_quality_score REAL NOT NULL DEFAULT 0.0
        CHECK (ingest_quality_score >= 0.0 AND ingest_quality_score <= 1.0),
    cover_path TEXT,
    norm_title TEXT NOT NULL DEFAULT '',
    subtitle TEXT,
    resolution_state TEXT NOT NULL DEFAULT 'pending'
        CHECK (resolution_state IN ('pending', 'running', 'done', 'failed')),
    resolution_outcome TEXT
        CHECK (resolution_outcome IN ('confirmed', 'enriched', 'disputed', 'ambiguous', 'unmatched')),
    resolution_requested_at TEXT NOT NULL DEFAULT '',
    resolution_requested_reason TEXT,
    last_resolved_at TEXT,
    last_resolution_run_id TEXT,
    metadata_locked INTEGER NOT NULL DEFAULT 0,
    metadata_provenance TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_books_sort_title ON books(sort_title);
CREATE INDEX idx_books_metadata_status ON books(metadata_status);
CREATE INDEX idx_books_added_at ON books(added_at);
CREATE INDEX idx_books_norm_prefix ON books(SUBSTR(norm_title, 1, 3));
CREATE INDEX idx_books_resolution_queue
    ON books(resolution_state, metadata_locked, resolution_requested_at);

CREATE TRIGGER books_updated_at AFTER UPDATE ON books
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE books SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;

-- ────────────────────────────────────────────────────────────────
-- Authors
-- ────────────────────────────────────────────────────────────────

CREATE TABLE authors (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    sort_name TEXT NOT NULL
);

CREATE INDEX idx_authors_sort_name ON authors(sort_name);

-- Book-Author junction (M:N with role and ordering)
CREATE TABLE book_authors (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'author',  -- e.g., 'author', 'editor', 'translator'
    position INTEGER NOT NULL DEFAULT 0,  -- ordering within the book
    PRIMARY KEY (book_id, author_id, role)
);

-- ────────────────────────────────────────────────────────────────
-- Series
-- ────────────────────────────────────────────────────────────────

CREATE TABLE series (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT
);

-- Book-Series junction (M:N with position for series order)
CREATE TABLE book_series (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    series_id TEXT NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    position REAL,  -- REAL allows 1.5 for interstitial ordering
    PRIMARY KEY (book_id, series_id)
);

-- ────────────────────────────────────────────────────────────────
-- Book files
-- ────────────────────────────────────────────────────────────────

CREATE TABLE book_files (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    format TEXT NOT NULL,  -- enum stored as text: epub, pdf, mobi, etc.
    format_version TEXT,
    storage_path TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    hash TEXT NOT NULL,  -- SHA-256, hex-encoded
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX idx_book_files_hash ON book_files(hash);
CREATE INDEX idx_book_files_book_id ON book_files(book_id);

-- ────────────────────────────────────────────────────────────────
-- Identifiers (ISBN, ASIN, etc.)
-- ────────────────────────────────────────────────────────────────

CREATE TABLE identifiers (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    identifier_type TEXT NOT NULL,  -- isbn13, isbn10, asin, google_books, open_library, hardcover
    value TEXT NOT NULL,
    source_type TEXT NOT NULL,  -- embedded, filename, provider, user
    source_name TEXT,           -- provider name when source_type='provider'
    confidence REAL NOT NULL DEFAULT 0.0
        CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE INDEX idx_identifiers_type_value ON identifiers(identifier_type, value);
CREATE INDEX idx_identifiers_book_id ON identifiers(book_id);

-- ────────────────────────────────────────────────────────────────
-- Tags
-- ────────────────────────────────────────────────────────────────

CREATE TABLE tags (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    category TEXT
);

CREATE UNIQUE INDEX idx_tags_name_category ON tags(name, category);

CREATE TABLE book_tags (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (book_id, tag_id)
);

-- ────────────────────────────────────────────────────────────────
-- Full-Text Search (FTS5)
-- ────────────────────────────────────────────────────────────────
-- Content-bearing FTS5 table with denormalized data.
-- Triggers keep the FTS index in sync with source tables.
-- Using `book_id` (UNINDEXED) to join back to the books table.

CREATE VIRTUAL TABLE books_fts USING fts5(
    book_id UNINDEXED,
    title,
    description,
    author_names
);

CREATE TRIGGER books_fts_insert AFTER INSERT ON books BEGIN
    INSERT INTO books_fts(book_id, title, description, author_names)
    VALUES (
        NEW.id,
        NEW.title || COALESCE(' ' || NEW.subtitle, ''),
        COALESCE(NEW.description, ''),
        ''
    );
END;

CREATE TRIGGER books_fts_update AFTER UPDATE OF title, subtitle, description ON books BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.id;
    INSERT INTO books_fts(book_id, title, description, author_names)
    VALUES (
        NEW.id,
        NEW.title || COALESCE(' ' || NEW.subtitle, ''),
        COALESCE(NEW.description, ''),
        COALESCE(
            (SELECT GROUP_CONCAT(a.name, ' ')
             FROM book_authors ba JOIN authors a ON a.id = ba.author_id
             WHERE ba.book_id = NEW.id),
            ''
        )
    );
END;

CREATE TRIGGER books_fts_delete BEFORE DELETE ON books BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.id;
END;

CREATE TRIGGER book_authors_fts_insert AFTER INSERT ON book_authors BEGIN
    DELETE FROM books_fts WHERE book_id = NEW.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE(
            (SELECT GROUP_CONCAT(a.name, ' ')
             FROM book_authors ba2 JOIN authors a ON a.id = ba2.author_id
             WHERE ba2.book_id = b.id),
            ''
        )
    FROM books b WHERE b.id = NEW.book_id;
END;

CREATE TRIGGER book_authors_fts_delete AFTER DELETE ON book_authors BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE(
            (SELECT GROUP_CONCAT(a.name, ' ')
             FROM book_authors ba2 JOIN authors a ON a.id = ba2.author_id
             WHERE ba2.book_id = b.id),
            ''
        )
    FROM books b WHERE b.id = OLD.book_id;
END;

CREATE TRIGGER authors_fts_update AFTER UPDATE OF name ON authors BEGIN
    DELETE FROM books_fts WHERE book_id IN (
        SELECT book_id FROM book_authors WHERE author_id = NEW.id
    );
    INSERT INTO books_fts(book_id, title, description, author_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE(
            (SELECT GROUP_CONCAT(a.name, ' ')
             FROM book_authors ba2 JOIN authors a ON a.id = ba2.author_id
             WHERE ba2.book_id = b.id),
            ''
        )
    FROM books b
    WHERE b.id IN (SELECT book_id FROM book_authors WHERE author_id = NEW.id);
END;

-- ────────────────────────────────────────────────────────────────
-- Tasks (background jobs)
-- ────────────────────────────────────────────────────────────────

CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL,
    task_type TEXT NOT NULL,
    payload TEXT NOT NULL,  -- JSON
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    progress INTEGER NOT NULL DEFAULT 0
        CHECK (progress >= 0 AND progress <= 100),
    message TEXT,
    result TEXT,  -- JSON result on completion
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at TEXT,
    completed_at TEXT,
    error_message TEXT,
    parent_task_id TEXT REFERENCES tasks(id)
);

CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_created_at ON tasks(created_at);
CREATE INDEX idx_tasks_parent ON tasks(parent_task_id);

-- ────────────────────────────────────────────────────────────────
-- Auth (users & sessions)
-- ────────────────────────────────────────────────────────────────

CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT UNIQUE NOT NULL,
    email TEXT,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user' CHECK(role IN ('admin', 'user')),
    created_at TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_users_username ON users(username);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_sessions_token_hash ON sessions(token_hash);
CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);

-- ────────────────────────────────────────────────────────────────
-- Identification & Resolution
-- ────────────────────────────────────────────────────────────────

CREATE TABLE resolution_runs (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    trigger TEXT NOT NULL,
    state TEXT NOT NULL
        CHECK (state IN ('running', 'done', 'failed', 'superseded')),
    outcome TEXT
        CHECK (outcome IN ('confirmed', 'enriched', 'disputed', 'ambiguous', 'unmatched')),
    query_json TEXT NOT NULL,
    decision_code TEXT NOT NULL,
    candidate_count INTEGER NOT NULL DEFAULT 0,
    best_candidate_id TEXT,
    best_score REAL,
    best_tier TEXT,
    error TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX idx_resolution_runs_book_started
    ON resolution_runs(book_id, started_at DESC);

CREATE TABLE identification_candidates (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES resolution_runs(id) ON DELETE SET NULL,
    provider_name TEXT NOT NULL,
    score REAL NOT NULL DEFAULT 0.0,
    metadata TEXT NOT NULL,
    match_reasons TEXT,
    disputes TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'applied', 'rejected', 'superseded')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    tier TEXT,
    apply_changeset TEXT
);

CREATE INDEX idx_candidates_book_id ON identification_candidates(book_id);
CREATE INDEX idx_candidates_run_id ON identification_candidates(run_id);
CREATE INDEX idx_candidates_status ON identification_candidates(status);

-- ────────────────────────────────────────────────────────────────
-- Duplicate detection
-- ────────────────────────────────────────────────────────────────

CREATE TABLE duplicate_links (
    id TEXT PRIMARY KEY NOT NULL,
    book_id_a TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    book_id_b TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    detection_method TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'merged', 'dismissed')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(book_id_a, book_id_b),
    CHECK(book_id_a != book_id_b)
);

CREATE INDEX idx_duplicate_links_status ON duplicate_links(status);
CREATE INDEX idx_duplicate_links_book_a ON duplicate_links(book_id_a);
CREATE INDEX idx_duplicate_links_book_b ON duplicate_links(book_id_b);

-- ────────────────────────────────────────────────────────────────
-- Settings
-- ────────────────────────────────────────────────────────────────

CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ────────────────────────────────────────────────────────────────
-- Reading progress & bookmarks
-- ────────────────────────────────────────────────────────────────

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

CREATE TRIGGER reading_progress_updated_at AFTER UPDATE ON reading_progress
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE reading_progress SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;

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

-- ────────────────────────────────────────────────────────────────
-- Watched directories
-- ────────────────────────────────────────────────────────────────

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

-- ────────────────────────────────────────────────────────────────
-- Seed data
-- ────────────────────────────────────────────────────────────────

INSERT INTO settings (key, value) VALUES ('watcher.debounce_ms', '"2000"');
INSERT INTO settings (key, value) VALUES ('watcher.default_poll_interval_secs', '"30"');
INSERT INTO settings (key, value) VALUES ('watcher.delete_source_after_import', '"false"');
