-- Rebuild FTS5 with tokenizer support for technical identifiers like C++/C#.
-- `+` and `#` become token characters so quoted MATCH terms such as `"c++"*`
-- and `"c#"*` target the indexed token rather than degrading to `c`.

DROP TRIGGER IF EXISTS books_fts_insert;
DROP TRIGGER IF EXISTS books_fts_update;
DROP TRIGGER IF EXISTS books_fts_delete;
DROP TRIGGER IF EXISTS book_authors_fts_insert;
DROP TRIGGER IF EXISTS book_authors_fts_delete;
DROP TRIGGER IF EXISTS authors_fts_update;
DROP TRIGGER IF EXISTS book_series_fts_insert;
DROP TRIGGER IF EXISTS book_series_fts_delete;
DROP TRIGGER IF EXISTS series_fts_update;
DROP TRIGGER IF EXISTS book_tags_fts_insert;
DROP TRIGGER IF EXISTS book_tags_fts_delete;
DROP TRIGGER IF EXISTS tags_fts_update;
DROP TRIGGER IF EXISTS publishers_fts_update;

DROP TABLE IF EXISTS books_fts;

CREATE VIRTUAL TABLE books_fts USING fts5(
    book_id UNINDEXED,
    title,
    description,
    author_names,
    series_names,
    publisher_name,
    tag_names,
    tokenize='unicode61 remove_diacritics 2 tokenchars ''+#''',
    prefix='2,3,4'
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
FROM books b;

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
