-- Add format_version column to book_files for storing the format specification
-- version (e.g., EPUB 3.0, PDF 1.7). Nullable because existing rows and some
-- formats don't have a meaningful version.
ALTER TABLE book_files ADD COLUMN format_version TEXT;
