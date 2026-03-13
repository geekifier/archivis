use std::fmt::Write as _;
use std::path::Path;

use archivis_core::models::metadata_rule::is_trusted_publisher;
use archivis_core::models::{
    Book, BookFile, BookFormat, DuplicateLink, FieldProvenance, Identifier, MetadataProvenance,
    MetadataRule, MetadataSource, MetadataStatus,
};
use archivis_db::{
    AuthorRepository, BookFileRepository, BookRepository, DbPool, DuplicateRepository,
    IdentifierRepository, PublisherRepository, SeriesRepository, SettingRepository, TagRepository,
};
use archivis_formats::sanitize::sanitize_text;
use archivis_formats::{ExtractedMetadata, ParsedFilename};
use archivis_storage::StorageBackend;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use super::cover;
use super::types::{DuplicateInfo, ImportConfig, ImportError, ImportResult};

/// Orchestrates the import of a single ebook file into the library.
pub struct ImportService<S: StorageBackend> {
    db_pool: DbPool,
    storage: S,
    config: ImportConfig,
    /// Serializes fuzzy-check through book-creation to prevent concurrent
    /// imports from missing each other's uncommitted books (TOCTOU race).
    import_mutex: tokio::sync::Mutex<()>,
}

impl<S: StorageBackend> ImportService<S> {
    pub fn new(db_pool: DbPool, storage: S, config: ImportConfig) -> Self {
        Self {
            db_pool,
            storage,
            config,
            import_mutex: tokio::sync::Mutex::new(()),
        }
    }

    /// Access the database pool (used by `BulkImportService` to load rules).
    pub fn db_pool(&self) -> &DbPool {
        &self.db_pool
    }

    /// Import a single file from `source_path` into the library.
    ///
    /// When `metadata_rules` is non-empty, books whose publisher matches a
    /// `TrustMetadata` rule are promoted to `Identified` regardless of ISBN
    /// presence, with a confidence floor of 0.8.
    ///
    /// Returns an [`ImportResult`] on success, describing the created or matched
    /// book and file records.
    #[allow(clippy::too_many_lines)]
    pub async fn import_file(
        &self,
        source_path: &Path,
        metadata_rules: &[MetadataRule],
    ) -> Result<ImportResult, ImportError> {
        // 1-5: Read, detect, extract, parse, score
        let data = tokio::fs::read(source_path).await?;
        let format = archivis_formats::detect::detect(&data)?;
        if format == BookFormat::Unknown {
            return Err(ImportError::InvalidFile(
                "unsupported or unrecognised file format".into(),
            ));
        }
        let mut embedded = extract_metadata(format, &data);
        embedded.authors = archivis_formats::authors::normalize_authors(embedded.authors);
        let parsed = archivis_formats::filename::parse_path(source_path);
        let mut score = archivis_formats::scoring::score_metadata(
            &embedded,
            Some(&parsed),
            &self.config.scoring_profile,
        );

        // Trusted publisher boost: if the embedded publisher matches a
        // `TrustMetadata` rule, promote to `Identified` with a confidence floor.
        if let Some(ref publisher_name) = embedded.publisher {
            if is_trusted_publisher(metadata_rules, publisher_name) {
                score.status = MetadataStatus::Identified;
                score.confidence = score.confidence.max(0.8);
                info!(publisher = %publisher_name, "trusted publisher — boosted to Identified");
            }
        }

        // 6: Hash check and ISBN duplicate check
        let hash = compute_sha256(&data);
        if let Some(existing) = BookFileRepository::get_by_hash(&self.db_pool, &hash).await? {
            return Err(ImportError::DuplicateFile {
                existing_book_id: existing.book_id,
                hash,
            });
        }
        if let Some(dup) = self.check_isbn_duplicates(&embedded, format).await? {
            return Ok(ImportResult {
                book_id: match &dup {
                    DuplicateInfo::SameIsbn {
                        existing_book_id, ..
                    }
                    | DuplicateInfo::ExactHash { existing_book_id }
                    | DuplicateInfo::FuzzyMatch {
                        existing_book_id, ..
                    } => *existing_book_id,
                },
                book_file_id: uuid::Uuid::nil(),
                status: score.status,
                confidence: score.confidence,
                duplicate: Some(dup),
                cover_extracted: false,
            });
        }

        // 7: Store file and cover to storage (I/O-heavy, runs concurrently)
        let title = resolve_title(&embedded, &parsed, source_path);
        let author = resolve_author(&embedded, &parsed);
        let (stored, cover_path) = self
            .store_file_and_cover(&data, &hash, &title, &author, source_path, &embedded)
            .await?;

        // Serialize fuzzy-check through DB-commit so concurrent imports
        // cannot both miss each other's uncommitted books (TOCTOU race).
        let import_guard = self.import_mutex.lock().await;

        // Fuzzy title+author duplicate check (soft — does not block import)
        let fuzzy_duplicate = self
            .check_fuzzy_duplicates(&embedded, &parsed, source_path)
            .await?;
        if let Some(ref dup) = fuzzy_duplicate {
            let DuplicateInfo::FuzzyMatch {
                existing_book_id,
                title_similarity,
                author_similarity,
            } = dup
            else {
                unreachable!()
            };
            info!(
                existing_book_id = %existing_book_id,
                title_sim = title_similarity,
                author_sim = author_similarity,
                "fuzzy duplicate detected (soft — import continues)"
            );
        }

        // Determine target book: existing (different format, same ISBN) → fuzzy auto-link → new
        let isbn_link = self
            .find_isbn_book_different_format(&embedded, format)
            .await?;
        let fuzzy_link = if isbn_link.is_some() {
            None
        } else {
            self.find_fuzzy_book_different_format(fuzzy_duplicate.as_ref(), format)
                .await?
        };
        let (book_id, is_new_book) = isbn_link
            .or(fuzzy_link)
            .map_or_else(|| (uuid::Uuid::new_v4(), true), |id| (id, false));

        // 9: Create DB records
        let book_file = self
            .create_db_records(
                book_id,
                is_new_book,
                format,
                &stored,
                cover_path,
                &score,
                &embedded,
                &parsed,
                &title,
            )
            .await?;

        // When auto-linked via fuzzy match, the file was cleanly attached —
        // no duplicate to record or surface.
        let report_duplicate = if fuzzy_link.is_some() {
            None
        } else {
            // Record fuzzy duplicate relationship for later review
            self.record_fuzzy_duplicate(fuzzy_duplicate.as_ref(), book_id)
                .await?;
            fuzzy_duplicate
        };

        BookRepository::mark_resolution_pending(&self.db_pool, book_id, "import").await?;

        // Release the import lock before I/O-heavy thumbnail generation.
        drop(import_guard);

        info!(book_id = %book_id, format = %format, status = %score.status, "imported file");

        // 8: Generate thumbnails (needs `book_id`, but not the lock)
        if let Some(ref cover_data) = embedded.cover_image {
            let cache_dir = self.config.data_dir.clone();
            if let Err(e) = cover::generate_thumbnails(
                cover_data,
                book_id,
                &cache_dir,
                &self.config.thumbnail_sizes,
            )
            .await
            {
                warn!("thumbnail generation failed: {e}");
            }
        }

        Ok(ImportResult {
            book_id,
            book_file_id: book_file,
            status: score.status,
            confidence: score.confidence,
            duplicate: report_duplicate,
            cover_extracted: embedded.cover_image.is_some(),
        })
    }

    /// If a fuzzy duplicate was detected, create a `DuplicateLink` record for later review.
    async fn record_fuzzy_duplicate(
        &self,
        fuzzy_duplicate: Option<&DuplicateInfo>,
        book_id: uuid::Uuid,
    ) -> Result<(), ImportError> {
        if let Some(DuplicateInfo::FuzzyMatch {
            existing_book_id,
            title_similarity,
            author_similarity,
        }) = fuzzy_duplicate
        {
            // Guard: never create a self-referential duplicate link
            if *existing_book_id == book_id {
                return Ok(());
            }
            let confidence = (title_similarity + author_similarity) / 2.0;
            if !DuplicateRepository::exists(&self.db_pool, *existing_book_id, book_id).await? {
                let link = DuplicateLink::new(*existing_book_id, book_id, "fuzzy", confidence);
                DuplicateRepository::create(&self.db_pool, &link).await?;
            }
        }
        Ok(())
    }

    /// Store the book file and its cover image (if present) in the storage backend.
    ///
    /// Thumbnail generation is intentionally NOT done here — it requires
    /// `book_id` which is only known after the duplicate check, so the caller
    /// handles it separately.
    async fn store_file_and_cover(
        &self,
        data: &[u8],
        precomputed_hash: &str,
        title: &str,
        author: &str,
        source_path: &Path,
        embedded: &ExtractedMetadata,
    ) -> Result<(archivis_storage::StoredFile, Option<String>), ImportError> {
        let filename = source_path
            .file_name()
            .map_or_else(|| "book".to_string(), |n| n.to_string_lossy().into_owned());
        let storage_path = archivis_storage::path::generate_book_path(author, title, &filename);
        let stored = self
            .storage
            .store_with_hash(&storage_path, data, precomputed_hash.to_owned())
            .await?;

        let mut cover_path = None;
        if let Some(ref cover_data) = embedded.cover_image {
            let book_dir = storage_path
                .rsplit_once('/')
                .map_or(&*storage_path, |(dir, _)| dir);

            match cover::store_cover(&self.storage, book_dir, cover_data).await {
                Ok(path) => cover_path = Some(path),
                Err(e) => warn!("cover storage failed, continuing without cover: {e}"),
            }
        }

        Ok((stored, cover_path))
    }

    /// Create all database records for an imported book file.
    #[allow(clippy::too_many_arguments)]
    async fn create_db_records(
        &self,
        book_id: uuid::Uuid,
        is_new_book: bool,
        format: BookFormat,
        stored: &archivis_storage::StoredFile,
        cover_path: Option<String>,
        score: &archivis_formats::scoring::MetadataScore,
        embedded: &ExtractedMetadata,
        parsed: &ParsedFilename,
        title: &str,
    ) -> Result<uuid::Uuid, ImportError> {
        let book_file = BookFile::new(
            book_id,
            format,
            &stored.path,
            #[allow(clippy::cast_possible_wrap)]
            {
                stored.size as i64
            },
            &stored.hash,
            embedded.format_version.clone(),
        );
        let book_file_id = book_file.id;

        if is_new_book {
            // Sanitize text fields from embedded metadata
            let sanitize_opts = &self.config.sanitize_options;
            let clean_title =
                sanitize_text(title, sanitize_opts).unwrap_or_else(|| title.to_string());
            let clean_description = embedded
                .description
                .as_deref()
                .and_then(|d| sanitize_text(d, sanitize_opts));

            let mut book = Book::new(&clean_title);
            book.id = book_id;
            book.subtitle = embedded
                .subtitle
                .as_deref()
                .and_then(|s| sanitize_text(s, sanitize_opts));
            book.description = clean_description;
            book.language = embedded
                .language
                .as_deref()
                .and_then(archivis_core::language::normalize_language)
                .map(String::from);
            book.page_count = embedded.page_count;
            book.publication_year = embedded.publication_year;
            if let Some(ref publisher_name) = embedded.publisher {
                let publisher =
                    PublisherRepository::find_or_create(&self.db_pool, publisher_name).await?;
                book.publisher_id = Some(publisher.id);
            }
            book.metadata_status = score.status;
            book.ingest_quality_score = score.confidence;
            book.cover_path = cover_path;
            book.metadata_provenance = initial_metadata_provenance(&book, embedded, parsed);

            BookRepository::create(&self.db_pool, &book).await?;
            BookFileRepository::create(&self.db_pool, &book_file).await?;
            self.create_authors(book_id, embedded, parsed).await?;
            self.create_identifiers(book_id, embedded).await?;
            self.create_tags(book_id, embedded, sanitize_opts).await?;
            self.create_series(book_id, embedded, parsed, sanitize_opts)
                .await?;
        } else {
            BookFileRepository::create(&self.db_pool, &book_file).await?;
        }

        Ok(book_file_id)
    }

    /// Check if any extracted ISBN matches an existing book that already has the same format.
    async fn check_isbn_duplicates(
        &self,
        embedded: &ExtractedMetadata,
        format: BookFormat,
    ) -> Result<Option<DuplicateInfo>, ImportError> {
        for ident in &embedded.identifiers {
            let type_str = identifier_type_to_db_str(ident.identifier_type);
            let matches =
                IdentifierRepository::find_by_value(&self.db_pool, type_str, &ident.value).await?;

            for matched in &matches {
                let files =
                    BookFileRepository::get_by_book_id(&self.db_pool, matched.book_id).await?;
                if files.iter().any(|f| f.format == format) {
                    return Ok(Some(DuplicateInfo::SameIsbn {
                        existing_book_id: matched.book_id,
                        isbn: ident.value.clone(),
                    }));
                }
            }
        }
        Ok(None)
    }

    /// Check for fuzzy title+author duplicates among existing books.
    ///
    /// Returns a `FuzzyMatch` if a likely duplicate is found. This is a *soft*
    /// duplicate: the import continues regardless; the caller decides whether
    /// to surface the match to the user.
    async fn check_fuzzy_duplicates(
        &self,
        embedded: &ExtractedMetadata,
        parsed: &ParsedFilename,
        source_path: &Path,
    ) -> Result<Option<DuplicateInfo>, ImportError> {
        use archivis_formats::similarity;

        let title = resolve_title(embedded, parsed, source_path);
        let norm_title = similarity::normalize_title(&title);
        if norm_title.is_empty() {
            return Ok(None);
        }

        let candidates =
            BookRepository::find_potential_duplicates(&self.db_pool, &title, 20).await?;

        let author = resolve_author(embedded, parsed);
        let norm_author = similarity::normalize_author(&author);

        let mut best_full: Option<(DuplicateInfo, f32)> = None;
        let mut best_title_only: Option<(DuplicateInfo, f32)> = None;

        for candidate in &candidates {
            let cand_norm_title = similarity::normalize_title(&candidate.book.title);
            let title_sim = similarity::trigram_similarity(&norm_title, &cand_norm_title);

            if title_sim < similarity::TITLE_MATCH_THRESHOLD {
                continue;
            }

            // Compare authors: find best match across all candidate authors.
            let author_sim = if norm_author.is_empty() && candidate.author_names.is_empty() {
                // Both have no author info — treat as matching.
                1.0
            } else if candidate.author_names.is_empty() || norm_author.is_empty() {
                // One side has no author — can't confirm or deny.
                0.0
            } else {
                candidate
                    .author_names
                    .iter()
                    .map(|ca| {
                        let norm_ca = similarity::normalize_author(ca);
                        similarity::trigram_similarity(&norm_author, &norm_ca)
                    })
                    .fold(0.0_f32, f32::max)
            };

            let info = DuplicateInfo::FuzzyMatch {
                existing_book_id: candidate.book.id,
                title_similarity: title_sim,
                author_similarity: author_sim,
            };

            if author_sim >= similarity::AUTHOR_MATCH_THRESHOLD {
                let score = title_sim + author_sim;
                if best_full.as_ref().map_or(true, |(_, s)| score > *s) {
                    best_full = Some((info, score));
                }
            } else if title_sim >= similarity::TITLE_ONLY_DUPLICATE_THRESHOLD
                && best_title_only
                    .as_ref()
                    .map_or(true, |(_, s)| title_sim > *s)
            {
                best_title_only = Some((info, title_sim));
            }
        }

        Ok(best_full.or(best_title_only).map(|(info, _)| info))
    }

    /// Find an existing book (via ISBN match) that does NOT have the given format yet.
    async fn find_isbn_book_different_format(
        &self,
        embedded: &ExtractedMetadata,
        format: BookFormat,
    ) -> Result<Option<uuid::Uuid>, ImportError> {
        for ident in &embedded.identifiers {
            let type_str = identifier_type_to_db_str(ident.identifier_type);
            let matches =
                IdentifierRepository::find_by_value(&self.db_pool, type_str, &ident.value).await?;

            for matched in &matches {
                let files =
                    BookFileRepository::get_by_book_id(&self.db_pool, matched.book_id).await?;
                if !files.iter().any(|f| f.format == format) {
                    info!(
                        book_id = %matched.book_id,
                        isbn = %ident.value,
                        "linking as additional format to existing book"
                    );
                    return Ok(Some(matched.book_id));
                }
            }
        }
        Ok(None)
    }

    /// Check if a fuzzy duplicate match qualifies for automatic format linking.
    ///
    /// Returns `Some(existing_book_id)` when:
    /// - The `import.auto_link_formats` setting is enabled (default: true)
    /// - The fuzzy match exceeds the auto-link thresholds (stricter than soft detection)
    /// - The existing book does not already have the incoming format
    async fn find_fuzzy_book_different_format(
        &self,
        fuzzy_duplicate: Option<&DuplicateInfo>,
        format: BookFormat,
    ) -> Result<Option<uuid::Uuid>, ImportError> {
        use archivis_formats::similarity;

        let Some(DuplicateInfo::FuzzyMatch {
            existing_book_id,
            title_similarity,
            author_similarity,
        }) = fuzzy_duplicate
        else {
            return Ok(None);
        };

        // Check if auto-linking is enabled (default from config, overridable via DB setting)
        let enabled = SettingRepository::get(&self.db_pool, "import.auto_link_formats")
            .await?
            .map_or(self.config.auto_link_formats, |v| v != "false");
        if !enabled {
            return Ok(None);
        }

        // Both similarities must exceed the stricter auto-link thresholds
        if *title_similarity < similarity::TITLE_AUTO_LINK_THRESHOLD
            || *author_similarity < similarity::AUTHOR_AUTO_LINK_THRESHOLD
        {
            return Ok(None);
        }

        // Same-format guard: don't auto-link if the existing book already has this format
        let files = BookFileRepository::get_by_book_id(&self.db_pool, *existing_book_id).await?;
        if files.iter().any(|f| f.format == format) {
            return Ok(None);
        }

        info!(
            book_id = %existing_book_id,
            title_sim = title_similarity,
            author_sim = author_similarity,
            "auto-linking as additional format to existing book (fuzzy match)"
        );

        Ok(Some(*existing_book_id))
    }

    async fn create_authors(
        &self,
        book_id: uuid::Uuid,
        embedded: &ExtractedMetadata,
        parsed: &ParsedFilename,
    ) -> Result<(), ImportError> {
        let mut authors: Vec<String> = embedded.authors.clone();
        if authors.is_empty() {
            if let Some(ref author_name) = parsed.author {
                authors.push(author_name.clone());
            }
        }
        if authors.is_empty() {
            authors.push("Unknown Author".into());
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        for (i, name) in authors.iter().enumerate() {
            let author = if let Some(existing) =
                AuthorRepository::find_by_name(&self.db_pool, name).await?
            {
                existing
            } else {
                let new_author = archivis_core::models::Author::new(name);
                AuthorRepository::create(&self.db_pool, &new_author).await?;
                new_author
            };
            BookRepository::add_author(&self.db_pool, book_id, author.id, "author", i as i32)
                .await?;
        }
        Ok(())
    }

    async fn create_identifiers(
        &self,
        book_id: uuid::Uuid,
        embedded: &ExtractedMetadata,
    ) -> Result<(), ImportError> {
        for ident in &embedded.identifiers {
            let identifier = Identifier::new(
                book_id,
                ident.identifier_type,
                &ident.value,
                MetadataSource::Embedded,
                0.9,
            );
            IdentifierRepository::create(&self.db_pool, &identifier).await?;
        }
        Ok(())
    }

    async fn create_tags(
        &self,
        book_id: uuid::Uuid,
        embedded: &ExtractedMetadata,
        sanitize_opts: &archivis_formats::sanitize::SanitizeOptions,
    ) -> Result<(), ImportError> {
        for subject in &embedded.subjects {
            let clean_subject =
                sanitize_text(subject, sanitize_opts).unwrap_or_else(|| subject.clone());
            let tag = TagRepository::find_or_create(&self.db_pool, &clean_subject, Some("subject"))
                .await?;
            BookRepository::add_tag(&self.db_pool, book_id, tag.id).await?;
        }
        Ok(())
    }

    async fn create_series(
        &self,
        book_id: uuid::Uuid,
        embedded: &ExtractedMetadata,
        parsed: &ParsedFilename,
        sanitize_opts: &archivis_formats::sanitize::SanitizeOptions,
    ) -> Result<(), ImportError> {
        let series_name = embedded.series.as_deref().or(parsed.series.as_deref());

        if let Some(name) = series_name {
            let clean_name = sanitize_text(name, sanitize_opts).unwrap_or_else(|| name.to_string());
            let series = SeriesRepository::find_or_create(&self.db_pool, &clean_name).await?;

            let position = embedded
                .series_position
                .or(parsed.series_position)
                .map(f64::from);

            BookRepository::add_series(&self.db_pool, book_id, series.id, position).await?;
        }
        Ok(())
    }
}

/// Extract metadata based on the detected format.
fn extract_metadata(format: BookFormat, data: &[u8]) -> ExtractedMetadata {
    match format {
        BookFormat::Epub => {
            archivis_formats::epub::extract_epub_metadata(data).unwrap_or_else(|e| {
                warn!("EPUB metadata extraction failed: {e}");
                ExtractedMetadata::default()
            })
        }
        BookFormat::Pdf => archivis_formats::pdf::extract_pdf_metadata(data).unwrap_or_else(|e| {
            warn!("PDF metadata extraction failed: {e}");
            ExtractedMetadata::default()
        }),
        BookFormat::Mobi | BookFormat::Azw3 => archivis_formats::mobi::extract_mobi_metadata(data)
            .unwrap_or_else(|e| {
                warn!("MOBI metadata extraction failed: {e}");
                ExtractedMetadata::default()
            }),
        _ => ExtractedMetadata::default(),
    }
}

/// Determine the best title from available metadata, falling back to the filename.
fn resolve_title(embedded: &ExtractedMetadata, parsed: &ParsedFilename, path: &Path) -> String {
    if let Some(ref title) = embedded.title {
        return title.clone();
    }
    if let Some(ref title) = parsed.title {
        return title.clone();
    }
    path.file_stem()
        .map_or_else(|| "Unknown".into(), |s| s.to_string_lossy().into_owned())
}

/// Determine the best author name from available metadata.
fn resolve_author(embedded: &ExtractedMetadata, parsed: &ParsedFilename) -> String {
    embedded
        .authors
        .first()
        .map(String::as_str)
        .or(parsed.author.as_deref())
        .unwrap_or("Unknown Author")
        .to_string()
}

fn initial_metadata_provenance(
    book: &Book,
    embedded: &ExtractedMetadata,
    parsed: &ParsedFilename,
) -> MetadataProvenance {
    MetadataProvenance {
        title: Some(protected_field(title_source(embedded))),
        subtitle: book
            .subtitle
            .as_ref()
            .and(embedded.subtitle.as_ref())
            .map(|_| protected_field(embedded.source.clone())),
        description: book
            .description
            .as_ref()
            .and(embedded.description.as_ref())
            .map(|_| protected_field(embedded.source.clone())),
        authors: authors_source(embedded, parsed).map(protected_field),
        series: series_source(embedded, parsed).map(protected_field),
        publisher: embedded
            .publisher
            .as_ref()
            .map(|_| protected_field(embedded.source.clone())),
        publication_year: embedded
            .publication_year
            .map(|_| protected_field(embedded.source.clone())),
        language: book
            .language
            .as_ref()
            .map(|_| protected_field(embedded.source.clone())),
        page_count: book
            .page_count
            .map(|_| protected_field(embedded.source.clone())),
        cover: book
            .cover_path
            .as_ref()
            .map(|_| protected_field(embedded.source.clone())),
    }
}

fn title_source(embedded: &ExtractedMetadata) -> MetadataSource {
    if embedded.title.is_some() {
        embedded.source.clone()
    } else {
        MetadataSource::Filename
    }
}

fn authors_source(embedded: &ExtractedMetadata, parsed: &ParsedFilename) -> Option<MetadataSource> {
    if !embedded.authors.is_empty() {
        Some(embedded.source.clone())
    } else if parsed.author.is_some() {
        Some(MetadataSource::Filename)
    } else {
        None
    }
}

fn series_source(embedded: &ExtractedMetadata, parsed: &ParsedFilename) -> Option<MetadataSource> {
    if embedded.series.is_some() {
        Some(embedded.source.clone())
    } else if parsed.series.is_some() {
        Some(MetadataSource::Filename)
    } else {
        None
    }
}

fn protected_field(origin: MetadataSource) -> FieldProvenance {
    FieldProvenance {
        origin,
        protected: true,
    }
}

/// Compute the SHA-256 hash of data, returning a hex-encoded string.
fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in result {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Convert an `IdentifierType` to the string representation used in the database.
fn identifier_type_to_db_str(id_type: archivis_core::models::IdentifierType) -> &'static str {
    use archivis_core::models::IdentifierType;
    match id_type {
        IdentifierType::Isbn13 => "isbn13",
        IdentifierType::Isbn10 => "isbn10",
        IdentifierType::Asin => "asin",
        IdentifierType::GoogleBooks => "google_books",
        IdentifierType::OpenLibrary => "open_library",
        IdentifierType::Hardcover => "hardcover",
        IdentifierType::Lccn => "lccn",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_value() {
        let hash = compute_sha256(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn resolve_title_prefers_embedded() {
        let embedded = ExtractedMetadata {
            title: Some("Dune".into()),
            ..ExtractedMetadata::default()
        };
        let parsed = ParsedFilename {
            title: Some("dune_book".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_title(&embedded, &parsed, Path::new("file.epub")),
            "Dune"
        );
    }

    #[test]
    fn resolve_title_falls_back_to_filename() {
        let embedded = ExtractedMetadata::default();
        let parsed = ParsedFilename::default();
        assert_eq!(
            resolve_title(&embedded, &parsed, Path::new("/books/my_book.epub")),
            "my_book"
        );
    }

    #[test]
    fn resolve_author_prefers_embedded() {
        let embedded = ExtractedMetadata {
            authors: vec!["Frank Herbert".into()],
            ..ExtractedMetadata::default()
        };
        let parsed = ParsedFilename {
            author: Some("Herbert".into()),
            ..Default::default()
        };
        assert_eq!(resolve_author(&embedded, &parsed), "Frank Herbert");
    }

    #[test]
    fn resolve_author_falls_back_to_unknown() {
        let embedded = ExtractedMetadata::default();
        let parsed = ParsedFilename::default();
        assert_eq!(resolve_author(&embedded, &parsed), "Unknown Author");
    }
}
