-- ────────────────────────────────────────────────────────────────
-- Phase 2: Search Indexes + FTS5 V2
-- ────────────────────────────────────────────────────────────────
-- Adds missing indexes on junction/filter columns and rebuilds
-- the FTS5 table to include series, publisher, and tag columns.

-- ────────────────────────────────────────────────────────────────
-- 1. Indexes for filter/join performance
-- ────────────────────────────────────────────────────────────────

CREATE INDEX idx_book_authors_author_id ON book_authors(author_id, book_id);
CREATE INDEX idx_book_series_series_id  ON book_series(series_id, book_id);
CREATE INDEX idx_book_tags_tag_id       ON book_tags(tag_id, book_id);
CREATE INDEX idx_book_files_format      ON book_files(format, book_id);
CREATE INDEX idx_books_publisher_id     ON books(publisher_id);
CREATE INDEX idx_books_language         ON books(language);
CREATE INDEX idx_books_publication_year ON books(publication_year);

-- ────────────────────────────────────────────────────────────────
-- 2. Drop old FTS5 triggers + table
-- ────────────────────────────────────────────────────────────────

DROP TRIGGER IF EXISTS books_fts_insert;
DROP TRIGGER IF EXISTS books_fts_update;
DROP TRIGGER IF EXISTS books_fts_delete;
DROP TRIGGER IF EXISTS book_authors_fts_insert;
DROP TRIGGER IF EXISTS book_authors_fts_delete;
DROP TRIGGER IF EXISTS authors_fts_update;

DROP TABLE IF EXISTS books_fts;

-- ────────────────────────────────────────────────────────────────
-- 3. Create new FTS5 table with expanded columns
-- ────────────────────────────────────────────────────────────────

CREATE VIRTUAL TABLE books_fts USING fts5(
    book_id UNINDEXED,
    title,
    description,
    author_names,
    series_names,
    publisher_name,
    tag_names,
    tokenize='unicode61 remove_diacritics 2',
    prefix='2,3,4'
);

-- ────────────────────────────────────────────────────────────────
-- 4. Backfill FTS from existing data
-- ────────────────────────────────────────────────────────────────

INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
SELECT
    b.id,
    b.title || COALESCE(' ' || b.subtitle, ''),
    COALESCE(b.description, ''),
    COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
    COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
    COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
    COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
FROM books b;

-- ────────────────────────────────────────────────────────────────
-- 5. New FTS triggers
-- ────────────────────────────────────────────────────────────────
-- Each trigger follows: delete FTS row, re-insert with fresh
-- denormalized data from correlated GROUP_CONCAT subqueries.

-- ── books table triggers ────────────────────────────────────────

CREATE TRIGGER books_fts_insert AFTER INSERT ON books BEGIN
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    VALUES (
        NEW.id,
        NEW.title || COALESCE(' ' || NEW.subtitle, ''),
        COALESCE(NEW.description, ''),
        '',
        '',
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = NEW.publisher_id), ''),
        ''
    );
END;

CREATE TRIGGER books_fts_update AFTER UPDATE OF title, subtitle, description, publisher_id ON books BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = NEW.id;
END;

CREATE TRIGGER books_fts_delete BEFORE DELETE ON books BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.id;
END;

-- ── book_authors triggers ───────────────────────────────────────

CREATE TRIGGER book_authors_fts_insert AFTER INSERT ON book_authors BEGIN
    DELETE FROM books_fts WHERE book_id = NEW.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = NEW.book_id;
END;

CREATE TRIGGER book_authors_fts_delete AFTER DELETE ON book_authors BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = OLD.book_id;
END;

-- ── authors triggers ────────────────────────────────────────────

CREATE TRIGGER authors_fts_update AFTER UPDATE OF name ON authors BEGIN
    DELETE FROM books_fts WHERE book_id IN (
        SELECT book_id FROM book_authors WHERE author_id = NEW.id
    );
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b
    WHERE b.id IN (SELECT book_id FROM book_authors WHERE author_id = NEW.id);
END;

-- ── book_series triggers ────────────────────────────────────────

CREATE TRIGGER book_series_fts_insert AFTER INSERT ON book_series BEGIN
    DELETE FROM books_fts WHERE book_id = NEW.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = NEW.book_id;
END;

CREATE TRIGGER book_series_fts_delete AFTER DELETE ON book_series BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = OLD.book_id;
END;

-- ── series triggers ─────────────────────────────────────────────

CREATE TRIGGER series_fts_update AFTER UPDATE OF name ON series BEGIN
    DELETE FROM books_fts WHERE book_id IN (
        SELECT book_id FROM book_series WHERE series_id = NEW.id
    );
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b
    WHERE b.id IN (SELECT book_id FROM book_series WHERE series_id = NEW.id);
END;

-- ── book_tags triggers ──────────────────────────────────────────

CREATE TRIGGER book_tags_fts_insert AFTER INSERT ON book_tags BEGIN
    DELETE FROM books_fts WHERE book_id = NEW.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = NEW.book_id;
END;

CREATE TRIGGER book_tags_fts_delete AFTER DELETE ON book_tags BEGIN
    DELETE FROM books_fts WHERE book_id = OLD.book_id;
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b WHERE b.id = OLD.book_id;
END;

-- ── tags triggers ───────────────────────────────────────────────

CREATE TRIGGER tags_fts_update AFTER UPDATE OF name ON tags BEGIN
    DELETE FROM books_fts WHERE book_id IN (
        SELECT book_id FROM book_tags WHERE tag_id = NEW.id
    );
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b
    WHERE b.id IN (SELECT book_id FROM book_tags WHERE tag_id = NEW.id);
END;

-- ── publishers triggers ─────────────────────────────────────────

CREATE TRIGGER publishers_fts_update AFTER UPDATE OF name ON publishers BEGIN
    DELETE FROM books_fts WHERE book_id IN (
        SELECT id FROM books WHERE publisher_id = NEW.id
    );
    INSERT INTO books_fts(book_id, title, description, author_names, series_names, publisher_name, tag_names)
    SELECT
        b.id,
        b.title || COALESCE(' ' || b.subtitle, ''),
        COALESCE(b.description, ''),
        COALESCE((SELECT GROUP_CONCAT(a.name, ' ') FROM book_authors ba JOIN authors a ON a.id = ba.author_id WHERE ba.book_id = b.id), ''),
        COALESCE((SELECT GROUP_CONCAT(s.name, ' ') FROM book_series bs JOIN series s ON s.id = bs.series_id WHERE bs.book_id = b.id), ''),
        COALESCE((SELECT p.name FROM publishers p WHERE p.id = b.publisher_id), ''),
        COALESCE((SELECT GROUP_CONCAT(t.name, ' ') FROM book_tags bt JOIN tags t ON t.id = bt.tag_id WHERE bt.book_id = b.id), '')
    FROM books b
    WHERE b.publisher_id = NEW.id;
END;
