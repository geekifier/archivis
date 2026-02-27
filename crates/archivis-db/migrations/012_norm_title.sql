-- Add a pre-computed normalized title column for duplicate detection.
-- Computed by normalize_title() in Rust, eliminates SQL/Rust normalization mismatch.
ALTER TABLE books ADD COLUMN norm_title TEXT NOT NULL DEFAULT '';
CREATE INDEX idx_books_norm_prefix ON books(SUBSTR(norm_title, 1, 3));
