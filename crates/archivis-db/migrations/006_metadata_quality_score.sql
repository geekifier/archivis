ALTER TABLE books
  ADD COLUMN metadata_quality_score REAL
  CHECK (metadata_quality_score IS NULL OR (metadata_quality_score >= 0.0 AND metadata_quality_score <= 1.0));
