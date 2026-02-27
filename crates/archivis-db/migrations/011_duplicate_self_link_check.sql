-- Prevent self-referential duplicate links and clean up any existing ones.

-- Remove any existing self-links (book_id_a == book_id_b).
DELETE FROM duplicate_links WHERE book_id_a = book_id_b;

-- Recreate the table with a CHECK constraint preventing self-links.
-- SQLite does not support ALTER TABLE ADD CHECK, so we must recreate.
CREATE TABLE duplicate_links_new (
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

INSERT INTO duplicate_links_new
    SELECT id, book_id_a, book_id_b, detection_method, confidence, status, created_at
    FROM duplicate_links;

DROP TABLE duplicate_links;
ALTER TABLE duplicate_links_new RENAME TO duplicate_links;

CREATE INDEX idx_duplicate_links_status ON duplicate_links(status);
CREATE INDEX idx_duplicate_links_book_a ON duplicate_links(book_id_a);
CREATE INDEX idx_duplicate_links_book_b ON duplicate_links(book_id_b);
