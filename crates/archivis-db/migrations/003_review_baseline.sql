ALTER TABLE books
  ADD COLUMN review_baseline_metadata_status TEXT
  CHECK (review_baseline_metadata_status IS NULL
      OR review_baseline_metadata_status IN ('identified', 'needs_review', 'unidentified'));
