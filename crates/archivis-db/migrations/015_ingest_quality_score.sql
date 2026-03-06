-- Rename import-time metadata quality to its permanent internal name.

ALTER TABLE books RENAME COLUMN metadata_confidence TO ingest_quality_score;
