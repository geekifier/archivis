-- Store identification candidates for user review.
CREATE TABLE identification_candidates (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    provider_name TEXT NOT NULL,            -- "open_library", "hardcover"
    score REAL NOT NULL DEFAULT 0.0,
    metadata TEXT NOT NULL,                 -- JSON: serialized ProviderMetadata
    match_reasons TEXT,                     -- JSON: array of reason strings
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'applied', 'rejected')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_candidates_book_id ON identification_candidates(book_id);
CREATE INDEX idx_candidates_status ON identification_candidates(status);
