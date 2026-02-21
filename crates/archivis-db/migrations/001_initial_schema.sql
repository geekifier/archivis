-- Initial schema for Archivis book library.
-- SQLite with WAL mode (set via PRAGMA, not migration).

-- Publishers
CREATE TABLE publishers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL
);

-- Books
CREATE TABLE books (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT NOT NULL,
    description TEXT,
    language TEXT,
    publication_date TEXT,  -- ISO 8601 date (YYYY-MM-DD)
    publisher_id TEXT REFERENCES publishers(id) ON DELETE SET NULL,
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    rating REAL CHECK (rating IS NULL OR (rating >= 0.0 AND rating <= 5.0)),
    page_count INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    metadata_status TEXT NOT NULL DEFAULT 'unidentified'
        CHECK (metadata_status IN ('identified', 'needs_review', 'unidentified')),
    metadata_confidence REAL NOT NULL DEFAULT 0.0
        CHECK (metadata_confidence >= 0.0 AND metadata_confidence <= 1.0),
    cover_path TEXT
);

CREATE INDEX idx_books_sort_title ON books(sort_title);
CREATE INDEX idx_books_metadata_status ON books(metadata_status);
CREATE INDEX idx_books_added_at ON books(added_at);

-- Authors
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

-- Series
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

-- Book files (1:N from books)
CREATE TABLE book_files (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    format TEXT NOT NULL,  -- enum stored as text: epub, pdf, mobi, etc.
    storage_path TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    hash TEXT NOT NULL,  -- SHA-256, hex-encoded
    added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX idx_book_files_hash ON book_files(hash);
CREATE INDEX idx_book_files_book_id ON book_files(book_id);

-- External identifiers (ISBN, ASIN, etc.)
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

-- Tags
CREATE TABLE tags (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    category TEXT
);

CREATE UNIQUE INDEX idx_tags_name_category ON tags(name, category);

-- Book-Tag junction (M:N)
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
-- Using book_id (UNINDEXED) to join back to the books table.

CREATE VIRTUAL TABLE books_fts USING fts5(
    book_id UNINDEXED,
    title,
    description,
    author_names
);

-- Triggers to keep FTS in sync with books table
CREATE TRIGGER books_fts_insert AFTER INSERT ON books BEGIN
    INSERT INTO books_fts(book_id, title, description, author_names)
    VALUES (NEW.id, NEW.title, COALESCE(NEW.description, ''), '');
END;

CREATE TRIGGER books_fts_update AFTER UPDATE OF title, description ON books BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.id;
    INSERT INTO books_fts(book_id, title, description, author_names)
    VALUES (
        NEW.id,
        NEW.title,
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

-- Triggers to update FTS when book-author links change
CREATE TRIGGER book_authors_fts_insert AFTER INSERT ON book_authors BEGIN
    DELETE FROM books_fts WHERE book_id = NEW.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names)
    SELECT
        b.id,
        b.title,
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
        b.title,
        COALESCE(b.description, ''),
        COALESCE(
            (SELECT GROUP_CONCAT(a.name, ' ')
             FROM book_authors ba2 JOIN authors a ON a.id = ba2.author_id
             WHERE ba2.book_id = b.id),
            ''
        )
    FROM books b WHERE b.id = OLD.book_id;
END;

-- Trigger to update FTS when author names change
CREATE TRIGGER authors_fts_update AFTER UPDATE OF name ON authors BEGIN
    -- Re-index all books by this author
    DELETE FROM books_fts WHERE book_id IN (
        SELECT book_id FROM book_authors WHERE author_id = NEW.id
    );
    INSERT INTO books_fts(book_id, title, description, author_names)
    SELECT
        b.id,
        b.title,
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

-- Trigger to auto-update the updated_at timestamp on books
CREATE TRIGGER books_updated_at AFTER UPDATE ON books
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
    UPDATE books SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;
