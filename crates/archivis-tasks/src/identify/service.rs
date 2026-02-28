use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::{
    Book, CandidateStatus, IdentificationCandidate, Identifier, IdentifierType, MetadataSource,
    MetadataStatus,
};
use archivis_db::{
    AuthorRepository, BookRepository, CandidateRepository, DbPool, IdentifierRepository,
    SeriesRepository,
};
use archivis_formats::sanitize::{sanitize_text, SanitizeOptions};
use archivis_metadata::{
    ExistingBookMetadata, MetadataQuery, MetadataResolver, ProviderIdentifier, ProviderMetadata,
    ResolverResult, ScoredCandidate,
};
use archivis_storage::StorageBackend;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::import::cover;
use crate::import::types::ThumbnailSizes;

/// Orchestrates the identification of books using external metadata providers.
pub struct IdentificationService<S: StorageBackend> {
    db_pool: DbPool,
    resolver: Arc<MetadataResolver>,
    storage: S,
    data_dir: PathBuf,
    thumbnail_sizes: ThumbnailSizes,
}

impl<S: StorageBackend> IdentificationService<S> {
    pub fn new(
        db_pool: DbPool,
        resolver: Arc<MetadataResolver>,
        storage: S,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            db_pool,
            resolver,
            storage,
            data_dir,
            thumbnail_sizes: ThumbnailSizes::default(),
        }
    }

    /// Identify a single book by querying metadata providers.
    ///
    /// Builds a `MetadataQuery` from the book's existing metadata, queries
    /// the resolver, stores all candidates in the database, and optionally
    /// auto-applies the best match.
    pub async fn identify_book(&self, book_id: Uuid) -> Result<ResolverResult, TaskError> {
        // 1. Load book from DB with identifiers
        let book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;

        let identifiers = IdentifierRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load identifiers: {e}")))?;

        let authors = self.load_author_names(book_id).await?;

        // 2. Build MetadataQuery from book's existing metadata
        let query = build_metadata_query(&book, &identifiers, &authors);

        debug!(
            book_id = %book_id,
            isbn = ?query.isbn,
            title = ?query.title,
            author = ?query.author,
            "built metadata query for identification"
        );

        // 3. Build ExistingBookMetadata for cross-validation
        let existing = ExistingBookMetadata {
            title: Some(book.title.clone()),
            authors: authors.clone(),
            identifiers: identifiers
                .iter()
                .map(|id| ProviderIdentifier {
                    identifier_type: id.identifier_type,
                    value: id.value.clone(),
                })
                .collect(),
            metadata_source: MetadataSource::Embedded,
        };

        // 4. Call resolver
        let result = self.resolver.resolve(&query, Some(&existing)).await;

        // 5. Store all candidates in identification_candidates table
        // Clear old candidates first
        CandidateRepository::delete_by_book(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to clear old candidates: {e}")))?;

        for scored in &result.candidates {
            let metadata_json = serde_json::to_value(&scored.metadata)
                .map_err(|e| TaskError::Failed(format!("failed to serialize metadata: {e}")))?;

            let candidate = IdentificationCandidate::new(
                book_id,
                &scored.provider_name,
                scored.score,
                metadata_json,
                scored.match_reasons.clone(),
            );

            CandidateRepository::create(&self.db_pool, &candidate)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to store candidate: {e}")))?;
        }

        // 6. Determine whether to auto-apply the best match.
        //
        // Even when the resolver flags auto_apply=true, we apply additional
        // conservative guards here to avoid overwriting metadata when results
        // are ambiguous:
        //   - Never auto-apply when 2+ candidates are above the auto-apply threshold (0.85).
        //   - Never auto-apply when the top two candidates have scores within 0.1.
        //   - Only auto-apply when the best candidate is at least 0.15 above the second-best.
        let should_auto_apply = result.auto_apply && {
            let dominated = is_clearly_dominant(&result.candidates, 0.85);
            if !dominated {
                info!(
                    book_id = %book_id,
                    "auto-apply suppressed by service: ambiguous candidates"
                );
            }
            dominated
        };

        if should_auto_apply {
            if let Some(ref best) = result.best_match {
                info!(
                    book_id = %book_id,
                    score = best.score,
                    provider = %best.provider_name,
                    "auto-applying best match"
                );

                // Find the candidate we just stored that matches the best match
                let candidates = CandidateRepository::list_by_book(&self.db_pool, book_id)
                    .await
                    .map_err(|e| TaskError::Failed(format!("failed to list candidates: {e}")))?;

                if let Some(best_candidate) = candidates.first() {
                    if let Err(e) = self
                        .apply_candidate(book_id, best_candidate.id, &HashSet::new())
                        .await
                    {
                        warn!(
                            book_id = %book_id,
                            error = %e,
                            "auto-apply failed, candidates stored for manual review"
                        );
                    }
                }
            }
        } else if !result.candidates.is_empty() {
            // Update status to NeedsReview if there are candidates but no auto-apply
            let mut book = book;
            book.metadata_status = MetadataStatus::NeedsReview;
            BookRepository::update(&self.db_pool, &book)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to update book status: {e}")))?;
        }

        info!(
            book_id = %book_id,
            candidates = result.candidates.len(),
            auto_apply = result.auto_apply,
            "identification complete"
        );

        Ok(result)
    }

    /// Apply a candidate's metadata to a book.
    ///
    /// Overwrites only fields that are from lower-trust sources; never
    /// overwrites user-edited metadata.
    ///
    /// When `exclude_fields` is non-empty, the named fields are skipped.
    /// Valid field names: `title`, `description`, `publication_date`,
    /// `authors`, `identifiers`, `series`, `cover`.
    pub async fn apply_candidate(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
        exclude_fields: &HashSet<String>,
    ) -> Result<Book, TaskError> {
        // Guard: check if another candidate is already applied for this book
        let existing_candidates = CandidateRepository::list_by_book(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to list candidates: {e}")))?;

        if existing_candidates
            .iter()
            .any(|c| c.status == CandidateStatus::Applied && c.id != candidate_id)
        {
            return Err(TaskError::Failed(
                "another candidate is already applied for this book".into(),
            ));
        }

        // 1. Load candidate from DB
        let candidate = CandidateRepository::get_by_id(&self.db_pool, candidate_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load candidate: {e}")))?
            .ok_or_else(|| TaskError::Failed(format!("candidate not found: {candidate_id}")))?;

        if candidate.book_id != book_id {
            return Err(TaskError::Failed(
                "candidate does not belong to the specified book".into(),
            ));
        }

        if candidate.status != CandidateStatus::Pending {
            return Err(TaskError::Failed(format!(
                "candidate already {}, cannot apply",
                candidate.status
            )));
        }

        // Deserialize the provider metadata
        let provider_meta: ProviderMetadata = serde_json::from_value(candidate.metadata.clone())
            .map_err(|e| {
                TaskError::Failed(format!("failed to deserialize candidate metadata: {e}"))
            })?;

        // 2. Load current book and update fields
        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;

        // Check current metadata source to decide overwrite behavior
        let current_is_user_edited =
            book.metadata_status == MetadataStatus::Identified && book.metadata_confidence >= 1.0;

        if !current_is_user_edited {
            merge_book_fields(&mut book, &provider_meta, exclude_fields);
        }

        // 5. Update metadata_status and metadata_confidence
        book.metadata_status = MetadataStatus::Identified;
        book.metadata_confidence = candidate.score;

        // 4. If candidate has cover_url and book has no cover: fetch and store
        if book.cover_path.is_none() && !exclude_fields.contains("cover") {
            if let Some(ref cover_url) = provider_meta.cover_url {
                match self.fetch_and_store_cover(book_id, cover_url, &book).await {
                    Ok(path) => {
                        book.cover_path = Some(path);
                    }
                    Err(e) => {
                        warn!(
                            book_id = %book_id,
                            error = %e,
                            "cover fetch/store failed, continuing without cover"
                        );
                    }
                }
            }
        }

        // Save updated book
        BookRepository::update(&self.db_pool, &book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;

        // 3. Add new identifiers from provider
        if !exclude_fields.contains("identifiers") {
            self.add_provider_identifiers(book_id, &provider_meta, candidate.score)
                .await?;
        }

        // Update authors from provider if book has only "Unknown Author"
        if !exclude_fields.contains("authors") {
            self.update_authors_from_provider(book_id, &provider_meta)
                .await?;
        }

        // Update series from provider
        if !exclude_fields.contains("series") {
            self.update_series_from_provider(book_id, &provider_meta)
                .await?;
        }

        // 6. Mark candidate as Applied
        CandidateRepository::update_status(&self.db_pool, candidate_id, CandidateStatus::Applied)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update candidate status: {e}")))?;

        // 7. Auto-reject all other pending candidates for this book
        let all_candidates = CandidateRepository::list_by_book(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to list candidates: {e}")))?;

        for other in &all_candidates {
            if other.id != candidate_id && other.status == CandidateStatus::Pending {
                CandidateRepository::update_status(
                    &self.db_pool,
                    other.id,
                    CandidateStatus::Rejected,
                )
                .await
                .map_err(|e| TaskError::Failed(format!("failed to reject other candidate: {e}")))?;
            }
        }

        info!(
            book_id = %book_id,
            candidate_id = %candidate_id,
            provider = %provider_meta.provider_name,
            "candidate applied successfully"
        );

        // 7. Return updated book
        BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to reload book: {e}")))
    }

    /// Undo a previously applied candidate.
    ///
    /// Removes identifiers that were added by the candidate's provider,
    /// resets the candidate back to `Pending`, restores all other `Rejected`
    /// candidates to `Pending`, and sets `metadata_status` to `NeedsReview`.
    ///
    /// Does **not** revert title/author/description changes (too complex
    /// and potentially destructive if the user has since edited them).
    pub async fn undo_candidate(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
    ) -> Result<Book, TaskError> {
        // 1. Load candidate and verify it's Applied
        let candidate = CandidateRepository::get_by_id(&self.db_pool, candidate_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load candidate: {e}")))?
            .ok_or_else(|| TaskError::Failed(format!("candidate not found: {candidate_id}")))?;

        if candidate.book_id != book_id {
            return Err(TaskError::Failed(
                "candidate does not belong to the specified book".into(),
            ));
        }

        if candidate.status != CandidateStatus::Applied {
            return Err(TaskError::Failed(format!(
                "candidate is {}, can only undo applied candidates",
                candidate.status
            )));
        }

        // 2. Remove identifiers added by this provider
        let removed = IdentifierRepository::delete_by_provider(
            &self.db_pool,
            book_id,
            &candidate.provider_name,
        )
        .await
        .map_err(|e| TaskError::Failed(format!("failed to remove provider identifiers: {e}")))?;

        debug!(
            book_id = %book_id,
            provider = %candidate.provider_name,
            removed = removed,
            "removed provider identifiers"
        );

        // 3. Set this candidate back to Pending
        CandidateRepository::update_status(&self.db_pool, candidate_id, CandidateStatus::Pending)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to reset candidate status: {e}")))?;

        // 4. Restore all other Rejected candidates for this book to Pending
        let all_candidates = CandidateRepository::list_by_book(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to list candidates: {e}")))?;

        for other in &all_candidates {
            if other.id != candidate_id && other.status == CandidateStatus::Rejected {
                CandidateRepository::update_status(
                    &self.db_pool,
                    other.id,
                    CandidateStatus::Pending,
                )
                .await
                .map_err(|e| {
                    TaskError::Failed(format!("failed to restore candidate status: {e}"))
                })?;
            }
        }

        // 5. Update book metadata_status back to NeedsReview
        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;

        book.metadata_status = MetadataStatus::NeedsReview;

        BookRepository::update(&self.db_pool, &book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update book status: {e}")))?;

        info!(
            book_id = %book_id,
            candidate_id = %candidate_id,
            provider = %candidate.provider_name,
            "candidate application undone"
        );

        Ok(book)
    }

    /// Add new identifiers from the provider metadata.
    async fn add_provider_identifiers(
        &self,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
        confidence: f32,
    ) -> Result<(), TaskError> {
        let existing = IdentifierRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load identifiers: {e}")))?;

        for prov_id in &provider_meta.identifiers {
            // Skip if we already have this exact identifier
            let already_exists = existing
                .iter()
                .any(|e| e.identifier_type == prov_id.identifier_type && e.value == prov_id.value);

            if !already_exists {
                let identifier = Identifier::new(
                    book_id,
                    prov_id.identifier_type,
                    &prov_id.value,
                    MetadataSource::Provider(provider_meta.provider_name.clone()),
                    confidence,
                );
                IdentifierRepository::create(&self.db_pool, &identifier)
                    .await
                    .map_err(|e| TaskError::Failed(format!("failed to create identifier: {e}")))?;
            }
        }

        Ok(())
    }

    /// Update authors from provider if book currently has only "Unknown Author".
    async fn update_authors_from_provider(
        &self,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
    ) -> Result<(), TaskError> {
        if provider_meta.authors.is_empty() {
            return Ok(());
        }

        let current_authors = self.load_author_names(book_id).await?;

        // Only replace if the current authors are just "Unknown Author"
        let should_replace =
            current_authors.len() == 1 && current_authors[0].to_lowercase() == "unknown author";

        if should_replace {
            BookRepository::clear_authors(&self.db_pool, book_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to clear authors: {e}")))?;

            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            for (i, author) in provider_meta.authors.iter().enumerate() {
                let role = author.role.as_deref().unwrap_or("author");
                let db_author = if let Some(existing) =
                    AuthorRepository::find_by_name(&self.db_pool, &author.name)
                        .await
                        .map_err(|e| TaskError::Failed(format!("author lookup failed: {e}")))?
                {
                    existing
                } else {
                    let new_author = archivis_core::models::Author::new(&author.name);
                    AuthorRepository::create(&self.db_pool, &new_author)
                        .await
                        .map_err(|e| TaskError::Failed(format!("author create failed: {e}")))?;
                    new_author
                };

                BookRepository::add_author(&self.db_pool, book_id, db_author.id, role, i as i32)
                    .await
                    .map_err(|e| TaskError::Failed(format!("add author failed: {e}")))?;
            }
        }

        Ok(())
    }

    /// Update series from provider metadata.
    ///
    /// Uses `find_or_create` for deduplication, checks whether the book is
    /// already linked to the resolved series, and backfills the position when
    /// the existing link has no position but the provider supplies one.
    async fn update_series_from_provider(
        &self,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
    ) -> Result<(), TaskError> {
        if let Some(ref prov_series) = provider_meta.series {
            let series = SeriesRepository::find_or_create(&self.db_pool, &prov_series.name)
                .await
                .map_err(|e| TaskError::Failed(format!("series find_or_create failed: {e}")))?;

            let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

            let position = prov_series.position.map(f64::from);

            if let Some(existing) = relations.series.iter().find(|s| s.series.id == series.id) {
                // Already linked — backfill position if the existing link has none
                if existing.position.is_none() && position.is_some() {
                    BookRepository::update_series_position(
                        &self.db_pool,
                        book_id,
                        series.id,
                        position,
                    )
                    .await
                    .map_err(|e| {
                        TaskError::Failed(format!("update series position failed: {e}"))
                    })?;
                }
            } else {
                // Not yet linked — add the series link
                BookRepository::add_series(&self.db_pool, book_id, series.id, position)
                    .await
                    .map_err(|e| TaskError::Failed(format!("add series failed: {e}")))?;
            }
        }

        Ok(())
    }

    /// Fetch a cover image from URL and store it.
    async fn fetch_and_store_cover(
        &self,
        book_id: Uuid,
        cover_url: &str,
        book: &Book,
    ) -> Result<String, String> {
        // Fetch cover bytes
        let response = reqwest::get(cover_url)
            .await
            .map_err(|e| format!("failed to fetch cover: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("cover fetch returned status {}", response.status()));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();

        let cover_bytes = response
            .bytes()
            .await
            .map_err(|e| format!("failed to read cover bytes: {e}"))?;

        if cover_bytes.is_empty() {
            return Err("cover response was empty".into());
        }

        let cover_data = archivis_formats::CoverData {
            bytes: cover_bytes.to_vec(),
            media_type: content_type,
        };

        // Build a storage path for the cover
        let author = "Unknown Author"; // Use a simple default for the path
        let book_dir = archivis_storage::path::generate_book_path(author, &book.title, "cover.jpg");
        let book_dir = book_dir.rsplit_once('/').map_or(&*book_dir, |(dir, _)| dir);

        // Store the cover
        let cover_path = cover::store_cover(&self.storage, book_dir, &cover_data).await?;

        // Generate thumbnails
        if let Err(e) =
            cover::generate_thumbnails(&cover_data, book_id, &self.data_dir, &self.thumbnail_sizes)
                .await
        {
            warn!("thumbnail generation failed: {e}");
        }

        Ok(cover_path)
    }

    /// Load author names for a book.
    async fn load_author_names(&self, book_id: Uuid) -> Result<Vec<String>, TaskError> {
        let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

        Ok(relations
            .authors
            .iter()
            .map(|a| a.author.name.clone())
            .collect())
    }
}

/// Merge provider metadata fields into a book.
///
/// Only overwrites fields that are currently empty or from lower-trust
/// sources. User-edited fields are never overwritten.
///
/// Fields listed in `exclude_fields` are skipped entirely.
/// Provider text fields are sanitized before applying.
fn merge_book_fields(
    book: &mut Book,
    provider_meta: &ProviderMetadata,
    exclude_fields: &HashSet<String>,
) {
    let sanitize_opts = SanitizeOptions::default();

    // Title: overwrite with provider data (sanitized)
    if !exclude_fields.contains("title") {
        if let Some(ref title) = provider_meta.title {
            if let Some(clean_title) = sanitize_text(title, &sanitize_opts) {
                book.set_title(clean_title);
            }
        }
    }

    // Subtitle: fill if empty (sanitized)
    if !exclude_fields.contains("subtitle") && book.subtitle.is_none() {
        if let Some(ref subtitle) = provider_meta.subtitle {
            book.subtitle = sanitize_text(subtitle, &sanitize_opts);
        }
    }

    // Description: fill if empty (sanitized)
    if !exclude_fields.contains("description") && book.description.is_none() {
        if let Some(ref desc) = provider_meta.description {
            book.description = sanitize_text(desc, &sanitize_opts);
        }
    }

    // Language: fill if empty (never excluded — not in UI)
    if book.language.is_none() {
        book.language.clone_from(&provider_meta.language);
    }

    // Page count: fill if empty (never excluded — not in UI)
    if book.page_count.is_none() {
        book.page_count = provider_meta.page_count;
    }

    // Publication date: fill if empty
    if !exclude_fields.contains("publication_date") && book.publication_date.is_none() {
        if let Some(ref date_str) = provider_meta.publication_date {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                book.publication_date = Some(date);
            } else if date_str.len() >= 4 {
                // Try parsing just the year
                if let Ok(year) = date_str[..4].parse::<i32>() {
                    book.publication_date = chrono::NaiveDate::from_ymd_opt(year, 1, 1);
                }
            }
        }
    }
}

/// Check whether the best candidate is clearly dominant over all others.
///
/// Returns `true` only when:
/// - There is at most one candidate above the `threshold`.
/// - The top two candidates are NOT within 0.1 of each other.
/// - The best candidate is at least 0.15 above the second-best.
///
/// Candidates must already be sorted by score descending.
fn is_clearly_dominant(candidates: &[ScoredCandidate], threshold: f32) -> bool {
    // If there's zero or one candidate, it's trivially dominant.
    if candidates.len() <= 1 {
        return true;
    }

    // Multiple candidates above the auto-apply threshold means ambiguity.
    let above_threshold = candidates.iter().filter(|c| c.score >= threshold).count();
    if above_threshold > 1 {
        return false;
    }

    // Check the gap between the best and the second-best.
    let best_score = candidates[0].score;
    let second_score = candidates[1].score;
    let gap = best_score - second_score;

    // Top two too close (within 0.1) — ambiguous.
    if gap < 0.1 {
        return false;
    }

    // Second-best must be at least 0.15 below the best.
    if gap < 0.15 {
        return false;
    }

    true
}

/// Build a `MetadataQuery` from a book's existing metadata.
fn build_metadata_query(
    book: &Book,
    identifiers: &[Identifier],
    authors: &[String],
) -> MetadataQuery {
    // Prefer ISBN-13, fall back to ISBN-10
    let isbn = identifiers
        .iter()
        .find(|id| id.identifier_type == IdentifierType::Isbn13)
        .or_else(|| {
            identifiers
                .iter()
                .find(|id| id.identifier_type == IdentifierType::Isbn10)
        })
        .map(|id| id.value.clone());

    let asin = identifiers
        .iter()
        .find(|id| id.identifier_type == IdentifierType::Asin)
        .map(|id| id.value.clone());

    MetadataQuery {
        isbn,
        title: Some(book.title.clone()),
        author: authors.first().cloned(),
        asin,
    }
}

#[cfg(test)]
mod tests {
    use archivis_core::models::IdentifierType;

    use super::*;

    #[test]
    fn build_query_prefers_isbn13() {
        let book = Book::new("Dune");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0441172717",
                MetadataSource::Embedded,
                0.9,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780441172719",
                MetadataSource::Embedded,
                0.9,
            ),
        ];
        let authors = vec!["Frank Herbert".to_string()];

        let query = build_metadata_query(&book, &identifiers, &authors);
        assert_eq!(query.isbn.as_deref(), Some("9780441172719"));
        assert_eq!(query.title.as_deref(), Some("Dune"));
        assert_eq!(query.author.as_deref(), Some("Frank Herbert"));
    }

    #[test]
    fn build_query_falls_back_to_isbn10() {
        let book = Book::new("Dune");
        let identifiers = vec![Identifier::new(
            book.id,
            IdentifierType::Isbn10,
            "0441172717",
            MetadataSource::Embedded,
            0.9,
        )];
        let authors = vec![];

        let query = build_metadata_query(&book, &identifiers, &authors);
        assert_eq!(query.isbn.as_deref(), Some("0441172717"));
        assert!(query.author.is_none());
    }

    #[test]
    fn build_query_no_isbn() {
        let book = Book::new("Unknown Book");
        let identifiers = vec![];
        let authors = vec![];

        let query = build_metadata_query(&book, &identifiers, &authors);
        assert!(query.isbn.is_none());
        assert_eq!(query.title.as_deref(), Some("Unknown Book"));
    }

    #[test]
    fn build_query_includes_asin() {
        let book = Book::new("Kindle Book");
        let identifiers = vec![Identifier::new(
            book.id,
            IdentifierType::Asin,
            "B000FA5ZEG",
            MetadataSource::Embedded,
            0.9,
        )];
        let authors = vec![];

        let query = build_metadata_query(&book, &identifiers, &authors);
        assert_eq!(query.asin.as_deref(), Some("B000FA5ZEG"));
    }

    // ── is_clearly_dominant tests ──

    /// Helper to build a minimal `ScoredCandidate` for dominance tests.
    fn make_candidate(score: f32) -> ScoredCandidate {
        ScoredCandidate {
            metadata: ProviderMetadata {
                provider_name: "test".to_string(),
                title: Some("Test Book".to_string()),
                subtitle: None,
                authors: vec![],
                description: None,
                language: None,
                publisher: None,
                publication_date: None,
                identifiers: vec![],
                subjects: vec![],
                series: None,
                page_count: None,
                cover_url: None,
                rating: None,
                confidence: score,
            },
            score,
            provider_name: "test".to_string(),
            match_reasons: vec![],
        }
    }

    #[test]
    fn dominant_single_candidate() {
        let candidates = vec![make_candidate(0.95)];
        assert!(
            is_clearly_dominant(&candidates, 0.85),
            "single candidate should be clearly dominant"
        );
    }

    #[test]
    fn dominant_empty_candidates() {
        let candidates: Vec<ScoredCandidate> = vec![];
        assert!(
            is_clearly_dominant(&candidates, 0.85),
            "empty list should be trivially dominant"
        );
    }

    #[test]
    fn not_dominant_two_above_threshold() {
        let candidates = vec![make_candidate(0.95), make_candidate(0.90)];
        assert!(
            !is_clearly_dominant(&candidates, 0.85),
            "two candidates above threshold should NOT be dominant"
        );
    }

    #[test]
    fn not_dominant_close_scores_within_010() {
        // Best = 0.90, second = 0.82, gap = 0.08 (< 0.1)
        let candidates = vec![make_candidate(0.90), make_candidate(0.82)];
        assert!(
            !is_clearly_dominant(&candidates, 0.85),
            "gap < 0.1 should NOT be dominant"
        );
    }

    #[test]
    fn not_dominant_gap_between_010_and_015() {
        // Best = 0.90, second = 0.78, gap = 0.12 (>= 0.1 but < 0.15)
        let candidates = vec![make_candidate(0.90), make_candidate(0.78)];
        assert!(
            !is_clearly_dominant(&candidates, 0.85),
            "gap between 0.10 and 0.15 should NOT be dominant"
        );
    }

    #[test]
    fn dominant_large_gap() {
        // Best = 0.95, second = 0.60, gap = 0.35 (well above 0.15)
        let candidates = vec![make_candidate(0.95), make_candidate(0.60)];
        assert!(
            is_clearly_dominant(&candidates, 0.85),
            "gap >= 0.15 with only one above threshold should be dominant"
        );
    }

    #[test]
    fn dominant_just_above_015_gap() {
        // Best = 0.90, second = 0.74, gap = 0.16 (just above boundary)
        let candidates = vec![make_candidate(0.90), make_candidate(0.74)];
        assert!(
            is_clearly_dominant(&candidates, 0.85),
            "gap above 0.15 should be dominant"
        );
    }

    #[test]
    fn not_dominant_three_candidates_two_above_threshold() {
        let candidates = vec![
            make_candidate(0.95),
            make_candidate(0.88),
            make_candidate(0.50),
        ];
        assert!(
            !is_clearly_dominant(&candidates, 0.85),
            "two of three candidates above threshold should NOT be dominant"
        );
    }
}
