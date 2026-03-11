ALTER TABLE books
  ADD COLUMN review_baseline_resolution_outcome TEXT
  CHECK (review_baseline_resolution_outcome IS NULL
      OR review_baseline_resolution_outcome IN ('confirmed', 'enriched', 'disputed', 'ambiguous', 'unmatched'));
