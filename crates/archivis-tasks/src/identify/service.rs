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
use archivis_metadata::{
    ExistingBookMetadata, MetadataQuery, MetadataResolver, ProviderIdentifier, ProviderMetadata,
    ResolverResult,
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

        // 6. If auto_apply is true, apply the best match
        if result.auto_apply {
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
                    if let Err(e) = self.apply_candidate(book_id, best_candidate.id).await {
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
    pub async fn apply_candidate(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
    ) -> Result<Book, TaskError> {
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
            merge_book_fields(&mut book, &provider_meta);
        }

        // 5. Update metadata_status and metadata_confidence
        book.metadata_status = MetadataStatus::Identified;
        book.metadata_confidence = candidate.score;

        // 4. If candidate has cover_url and book has no cover: fetch and store
        if book.cover_path.is_none() {
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
        self.add_provider_identifiers(book_id, &provider_meta, candidate.score)
            .await?;

        // Update authors from provider if book has only "Unknown Author"
        self.update_authors_from_provider(book_id, &provider_meta)
            .await?;

        // Update series from provider
        self.update_series_from_provider(book_id, &provider_meta)
            .await?;

        // 6. Mark candidate as Applied
        CandidateRepository::update_status(&self.db_pool, candidate_id, CandidateStatus::Applied)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update candidate status: {e}")))?;

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

    /// Update series from provider if book currently has no series.
    async fn update_series_from_provider(
        &self,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
    ) -> Result<(), TaskError> {
        if let Some(ref prov_series) = provider_meta.series {
            // Check if book already has a series
            let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

            if relations.series.is_empty() {
                let series = archivis_core::models::Series::new(&prov_series.name);
                SeriesRepository::create(&self.db_pool, &series)
                    .await
                    .map_err(|e| TaskError::Failed(format!("series create failed: {e}")))?;

                let position = prov_series.position.map(f64::from);
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
fn merge_book_fields(book: &mut Book, provider_meta: &ProviderMetadata) {
    // Title: overwrite with provider data
    if let Some(ref title) = provider_meta.title {
        if !title.is_empty() {
            book.set_title(title);
        }
    }

    // Description: fill if empty
    if book.description.is_none() {
        if let Some(ref desc) = provider_meta.description {
            if !desc.is_empty() {
                book.description = Some(desc.clone());
            }
        }
    }

    // Language: fill if empty
    if book.language.is_none() {
        book.language.clone_from(&provider_meta.language);
    }

    // Page count: fill if empty
    if book.page_count.is_none() {
        book.page_count = provider_meta.page_count;
    }

    // Publication date: fill if empty
    if book.publication_date.is_none() {
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
}
