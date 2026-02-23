-- Track detected duplicate relationships between books.
CREATE TABLE duplicate_links (
    id TEXT PRIMARY KEY NOT NULL,
    book_id_a TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    book_id_b TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    detection_method TEXT NOT NULL,     -- "hash", "isbn", "fuzzy", "user"
    confidence REAL NOT NULL DEFAULT 0.0,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'merged', 'dismissed')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(book_id_a, book_id_b)       -- No duplicate links
);

CREATE INDEX idx_duplicate_links_status ON duplicate_links(status);
CREATE INDEX idx_duplicate_links_book_a ON duplicate_links(book_id_a);
CREATE INDEX idx_duplicate_links_book_b ON duplicate_links(book_id_b);
