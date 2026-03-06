-- Add resolution lifecycle state and durable run history foundations.

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

DROP TRIGGER books_updated_at;

ALTER TABLE books ADD COLUMN resolution_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (resolution_state IN ('pending', 'running', 'done', 'failed'));

ALTER TABLE books ADD COLUMN resolution_outcome TEXT
    CHECK (resolution_outcome IN ('confirmed', 'enriched', 'disputed', 'ambiguous', 'unmatched'));

-- SQLite cannot add a populated-table column with a non-constant default.
-- Existing rows are backfilled from `added_at`, and application writes provide
-- a real request timestamp for new rows during this transitional phase.
ALTER TABLE books ADD COLUMN resolution_requested_at TEXT NOT NULL DEFAULT '';

ALTER TABLE books ADD COLUMN resolution_requested_reason TEXT;

ALTER TABLE books ADD COLUMN last_resolved_at TEXT;

ALTER TABLE books ADD COLUMN last_resolution_run_id TEXT;

ALTER TABLE books ADD COLUMN metadata_locked INTEGER NOT NULL DEFAULT 0;

ALTER TABLE books ADD COLUMN metadata_provenance TEXT NOT NULL DEFAULT '{}';

UPDATE books
SET
    resolution_requested_at = COALESCE(NULLIF(added_at, ''), '1970-01-01T00:00:00Z'),
    resolution_requested_reason = 'migration_backfill'
WHERE resolution_requested_at = ''
   OR resolution_requested_reason IS NULL;

CREATE INDEX idx_books_resolution_queue
    ON books(resolution_state, metadata_locked, resolution_requested_at);

CREATE TRIGGER books_updated_at AFTER UPDATE ON books
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE books SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;

ALTER TABLE identification_candidates RENAME TO identification_candidates_old;

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
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT INTO identification_candidates (
    id,
    book_id,
    run_id,
    provider_name,
    score,
    metadata,
    match_reasons,
    disputes,
    status,
    created_at
)
SELECT
    id,
    book_id,
    NULL,
    provider_name,
    score,
    metadata,
    match_reasons,
    NULL,
    status,
    created_at
FROM identification_candidates_old;

DROP TABLE identification_candidates_old;

CREATE INDEX idx_candidates_book_id ON identification_candidates(book_id);
CREATE INDEX idx_candidates_run_id ON identification_candidates(run_id);
CREATE INDEX idx_candidates_status ON identification_candidates(status);
