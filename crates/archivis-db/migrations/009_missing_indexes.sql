-- Standalone indexes for boolean filter columns on `books`.
-- The composite index on (resolution_state, metadata_locked, ...) exists
-- but a standalone `metadata_locked` index is needed for direct filter queries.

CREATE INDEX IF NOT EXISTS idx_books_trusted ON books(metadata_user_trusted);
CREATE INDEX IF NOT EXISTS idx_books_locked  ON books(metadata_locked);
