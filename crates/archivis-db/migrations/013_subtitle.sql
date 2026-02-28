-- Add subtitle column to books table.
ALTER TABLE books ADD COLUMN subtitle TEXT;

-- Update FTS triggers to include subtitle in the title field for search.

-- Drop and recreate the insert trigger.
DROP TRIGGER IF EXISTS books_fts_insert;
CREATE TRIGGER books_fts_insert AFTER INSERT ON books BEGIN
    INSERT INTO books_fts(book_id, title, description, author_names)
    VALUES (
        NEW.id,
        NEW.title || COALESCE(' ' || NEW.subtitle, ''),
        COALESCE(NEW.description, ''),
        ''
    );
END;

-- Drop and recreate the update trigger (add subtitle to the column list).
DROP TRIGGER IF EXISTS books_fts_update;
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

-- Drop and recreate author-change triggers to include subtitle.
DROP TRIGGER IF EXISTS book_authors_fts_insert;
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

DROP TRIGGER IF EXISTS book_authors_fts_delete;
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

DROP TRIGGER IF EXISTS authors_fts_update;
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
