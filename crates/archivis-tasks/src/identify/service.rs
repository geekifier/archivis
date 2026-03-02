use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::isbn::{normalize_asin, normalize_isbn, to_isbn13};
use archivis_core::models::{
    Book, CandidateStatus, IdentificationCandidate, Identifier, IdentifierType, MetadataSource,
    MetadataStatus,
};
use archivis_db::{
    AuthorRepository, BookFileRepository, BookRepository, CandidateRepository, DbPool,
    IdentifierRepository, SeriesRepository,
};
use archivis_formats::sanitize::{sanitize_text, SanitizeOptions};
use archivis_metadata::{
    CandidateMatchTier, ExistingBookMetadata, MetadataQuery, MetadataResolver, ProviderIdentifier,
    ProviderMetadata, ResolverDecision, ResolverResult,
};
use archivis_storage::StorageBackend;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::import::cover;
use crate::import::types::ThumbnailSizes;

// ── Field-category policy for apply merges ──────────────────────────

/// Context passed to merge helpers so they can decide which fields are
/// safe to overwrite.
///
/// Fields fall into two categories with different overwrite policies:
///
/// **Core identity** (title, authors, series): guarded by
/// [`may_overwrite_core`](Self::may_overwrite_core). Auto-apply always
/// requires strong ID proof; both auto and manual apply block overwrites
/// when a title contradiction is detected without strong ID proof.
///
/// **Enrichment** (subtitle, description, `publication_date`, identifiers,
/// cover): freely overwritten when the candidate's source outranks the
/// current value's source.
#[derive(Clone, Debug)]
struct FieldApplyContext {
    /// True when the merge is triggered by the auto-apply path (no user
    /// confirmation).  Core identity fields require stronger proof here.
    is_auto_apply: bool,
    /// True when the candidate's identifiers match a trusted (User /
    /// Embedded) identifier on the book.
    has_strong_id_proof: bool,
    /// True when the candidate's title strongly contradicts the book's
    /// current title (computed once at context-build time).
    has_title_contradiction: bool,
}

impl FieldApplyContext {
    /// Whether core identity fields (title, authors, series) may be
    /// overwritten in this context.
    ///
    /// Rules:
    /// - Auto-apply always requires strong ID proof.
    /// - Both auto and manual apply block core overwrites when a title
    ///   contradiction is detected and there is no strong ID proof.
    fn may_overwrite_core(&self) -> bool {
        if self.is_auto_apply && !self.has_strong_id_proof {
            return false;
        }
        if self.has_title_contradiction && !self.has_strong_id_proof {
            return false;
        }
        true
    }
}

/// The result of `identify_book`, wrapping the resolver output with the
/// actual service-level auto-apply decision.
///
/// `decision_reason` uses a structured prefix from [`ResolverDecision`]:
/// - `auto_apply_allowed: tier=…, score=…`
/// - `blocked_no_trusted_id: tier=…, score=…`
/// - `blocked_ambiguous: tier=…, score=…`
/// - `blocked_contradiction: tier=…, score=…`
/// - `blocked_low_tier: tier=…, score=…`
/// - `no_candidates`
/// - `auto_apply_failed: {error}` (resolver said yes, but apply step failed)
#[derive(Debug)]
pub struct IdentificationOutcome {
    /// The resolver's scored candidates and recommendation.
    pub resolver_result: ResolverResult,
    /// Whether auto-apply actually happened (not just whether the resolver
    /// recommended it — the apply step itself may fail).
    pub auto_applied: bool,
    /// Tier of the best match, if any.
    pub best_tier: Option<CandidateMatchTier>,
    /// Short human-readable reason for the final decision.
    pub decision_reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoverApplyPolicy {
    UserChoice,
    PreserveExisting,
}

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
    #[allow(clippy::too_many_lines)]
    pub async fn identify_book(&self, book_id: Uuid) -> Result<IdentificationOutcome, TaskError> {
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
            trusted_identifiers: filter_trusted_identifiers(&identifiers)
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

        // 6. Auto-apply decision.
        //
        // The resolver is the single authoritative source for the auto-apply
        // decision: it enforces tier (StrongIdMatch only), settings-driven
        // threshold, and ambiguity/gap guards.  The service trusts that
        // recommendation and only tracks whether the apply step succeeds.
        let best_tier = result.best_match.as_ref().map(|b| b.tier);
        let mut auto_applied = false;
        let decision_reason;

        // Build a structured decision reason string from the resolver's
        // decision code and the best candidate's tier/score.
        let format_decision = |decision: ResolverDecision,
                               best: Option<&archivis_metadata::ScoredCandidate>|
         -> String {
            best.map_or_else(
                || format!("{decision}"),
                |b| format!("{decision}: tier={}, score={:.2}", b.tier, b.score),
            )
        };

        if result.auto_apply {
            if let Some(ref best) = result.best_match {
                info!(
                    book_id = %book_id,
                    score = best.score,
                    tier = %best.tier,
                    provider = %best.provider_name,
                    "auto-applying best match"
                );

                // Find the candidate we just stored that matches the best match
                let candidates = CandidateRepository::list_by_book(&self.db_pool, book_id)
                    .await
                    .map_err(|e| TaskError::Failed(format!("failed to list candidates: {e}")))?;

                if let Some(best_candidate) = candidates.first() {
                    match self
                        .apply_candidate_with_policy(
                            book_id,
                            best_candidate.id,
                            &HashSet::new(),
                            CoverApplyPolicy::PreserveExisting,
                            true, // auto-apply
                        )
                        .await
                    {
                        Ok(_) => {
                            auto_applied = true;
                            decision_reason =
                                format_decision(result.decision, result.best_match.as_ref());
                        }
                        Err(e) => {
                            warn!(
                                book_id = %book_id,
                                error = %e,
                                "auto-apply failed, candidates stored for manual review"
                            );
                            decision_reason = format!("auto_apply_failed: {e}");
                        }
                    }
                } else {
                    decision_reason = "auto_apply_failed: no stored candidate found".into();
                }
            } else {
                decision_reason = "auto_apply_failed: no best match".into();
            }
        } else if result.candidates.is_empty() {
            decision_reason = format_decision(result.decision, None);
        } else {
            // Candidates exist but resolver declined auto-apply.
            let mut book = book;
            book.metadata_status = MetadataStatus::NeedsReview;
            BookRepository::update(&self.db_pool, &book)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to update book status: {e}")))?;

            decision_reason = format_decision(result.decision, result.best_match.as_ref());
        }

        info!(
            book_id = %book_id,
            candidates = result.candidates.len(),
            auto_applied,
            best_tier = best_tier.as_ref().map_or_else(|| "none".to_string(), ToString::to_string),
            decision_reason = %decision_reason,
            "identification complete"
        );

        Ok(IdentificationOutcome {
            resolver_result: result,
            auto_applied,
            best_tier,
            decision_reason,
        })
    }

    /// Apply a candidate's metadata to a book (manual / user-initiated).
    ///
    /// Overwrites only fields that are from lower-trust sources; never
    /// overwrites user-edited metadata.  Core identity fields (title,
    /// authors, series) are additionally guarded by a contradiction
    /// check: if the candidate title strongly conflicts with the
    /// current trusted title and the candidate lacks strong ID proof,
    /// the core overwrite is blocked.
    ///
    /// When `exclude_fields` is non-empty, the named fields are skipped.
    /// Valid field names: `title`, `subtitle`, `description`,
    /// `publication_date`, `authors`, `identifiers`, `series`, `cover`.
    pub async fn apply_candidate(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
        exclude_fields: &HashSet<String>,
    ) -> Result<Book, TaskError> {
        self.apply_candidate_with_policy(
            book_id,
            candidate_id,
            exclude_fields,
            CoverApplyPolicy::UserChoice,
            false, // manual apply
        )
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_candidate_with_policy(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
        exclude_fields: &HashSet<String>,
        cover_policy: CoverApplyPolicy,
        is_auto_apply: bool,
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

        // Build the field-apply context: check candidate IDs against
        // the book's trusted identifiers.
        let book_identifiers = IdentifierRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load identifiers: {e}")))?;

        let trusted = filter_trusted_identifiers(&book_identifiers);

        let has_strong_id_proof =
            candidate_has_strong_id_proof(&provider_meta.identifiers, &trusted);

        let has_title_contradiction = provider_meta
            .title
            .as_deref()
            .is_some_and(|t| titles_contradict(&book.title, t));

        let apply_ctx = FieldApplyContext {
            is_auto_apply,
            has_strong_id_proof,
            has_title_contradiction,
        };

        // Check current metadata source to decide overwrite behavior
        let current_is_user_edited =
            book.metadata_status == MetadataStatus::Identified && book.metadata_confidence >= 1.0;

        if !current_is_user_edited {
            merge_book_fields(&mut book, &provider_meta, exclude_fields, &apply_ctx);
        }

        // 5. Update metadata_status and metadata_confidence.
        // Preserve user-lock: when the user has set confidence to 1.0
        // (manual curation), keep it so subsequent applies cannot
        // erode the lock.
        book.metadata_status = MetadataStatus::Identified;
        if !current_is_user_edited {
            book.metadata_confidence = candidate.score;
        }

        // 4. Fetch and store cover when appropriate
        let preserve_existing_cover =
            cover_policy == CoverApplyPolicy::PreserveExisting && book.cover_path.is_some();
        let should_apply_cover = !(exclude_fields.contains("cover") || preserve_existing_cover);

        if should_apply_cover {
            if let Some(ref cover_url) = provider_meta.cover_url {
                let old_cover_path = book.cover_path.clone();
                match self.fetch_and_store_cover(book_id, cover_url, &book).await {
                    Ok(new_path) => {
                        book.cover_path = Some(new_path.clone());
                        // Clean up old cover ONLY after new one is safely stored,
                        // and only when the paths differ (same path means the
                        // new file overwrote the old one in place).
                        if let Some(ref old_path) = old_cover_path {
                            if *old_path != new_path {
                                if let Err(e) = self.storage.delete(old_path).await {
                                    warn!(
                                        book_id = %book_id,
                                        old_path = %old_path,
                                        error = %e,
                                        "failed to delete old cover from storage"
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            book_id = %book_id,
                            error = %e,
                            "cover fetch/store failed, continuing without cover"
                        );
                        // Old cover preserved — no broken state
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

        // Update authors from provider.
        // Core identity field: gated by apply context and user-lock.
        if !exclude_fields.contains("authors")
            && !current_is_user_edited
            && apply_ctx.may_overwrite_core()
        {
            self.update_authors_from_provider(book_id, &provider_meta)
                .await?;
        }

        // Update series from provider.
        // Core identity field: gated by apply context.
        if !exclude_fields.contains("series") && apply_ctx.may_overwrite_core() {
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

    /// Update authors from provider.
    ///
    /// The caller is responsible for all policy gating (`exclude_fields`,
    /// user-locked metadata, `may_overwrite_core`).  This function
    /// unconditionally replaces the book's author list with the
    /// provider's when the provider supplies at least one author.
    async fn update_authors_from_provider(
        &self,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
    ) -> Result<(), TaskError> {
        if provider_meta.authors.is_empty() {
            return Ok(());
        }

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

        // Build a storage path for the cover.
        let book_dir = self.resolve_cover_directory(book_id, book).await?;

        // Store the cover
        let cover_path = cover::store_cover(&self.storage, &book_dir, &cover_data).await?;

        // Clear old thumbnail cache before generating fresh thumbnails.
        let cache_dir = self.data_dir.join("covers").join(book_id.to_string());
        if cache_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await {
                warn!(
                    book_id = %book_id,
                    error = %e,
                    "failed to remove old thumbnail cache"
                );
            }
        }

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

    /// Determine the storage directory for a cover image.
    ///
    /// Priority:
    /// 1. First book file directory (canonical layout).
    /// 2. Existing cover directory (preserves previous placement).
    /// 3. Generated path from actual author names and title.
    async fn resolve_cover_directory(&self, book_id: Uuid, book: &Book) -> Result<String, String> {
        let files = BookFileRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| format!("failed to load book files for cover path: {e}"))?;

        if let Some(dir) = files.first().and_then(|f| {
            f.storage_path
                .rfind('/')
                .map(|idx| f.storage_path[..idx].to_string())
        }) {
            return Ok(dir);
        }

        if let Some(dir) = book
            .cover_path
            .as_ref()
            .and_then(|p| p.rfind('/').map(|idx| p[..idx].to_string()))
        {
            return Ok(dir);
        }

        let authors = self
            .load_author_names(book_id)
            .await
            .map_err(|e| format!("failed to load authors for cover path: {e}"))?;
        let author_for_path = authors.first().map_or("Unknown Author", String::as_str);

        let generated =
            archivis_storage::path::generate_book_path(author_for_path, &book.title, "cover.jpg");
        Ok(match generated.rsplit_once('/') {
            Some((dir, _)) => dir.to_string(),
            None => generated,
        })
    }
}

// ── Contradiction detection helpers ──────────────────────────────────

/// Filter identifiers down to the trusted set for strong-ID proof.
///
/// Trusted sources: `User`, `Embedded`, and a single `ContentScan` ISBN
/// (a lone scan ISBN is almost certainly the book's own, not bibliography
/// noise). When 2+ scan ISBNs exist, all are excluded.
fn filter_trusted_identifiers(identifiers: &[Identifier]) -> Vec<Identifier> {
    let scan_isbn_count = identifiers
        .iter()
        .filter(|id| {
            id.source == MetadataSource::ContentScan
                && matches!(
                    id.identifier_type,
                    IdentifierType::Isbn13 | IdentifierType::Isbn10 | IdentifierType::Asin
                )
        })
        .count();
    let trust_single_scan = scan_isbn_count == 1;

    identifiers
        .iter()
        .filter(|id| {
            let source_trusted =
                matches!(id.source, MetadataSource::User | MetadataSource::Embedded)
                    || (trust_single_scan && id.source == MetadataSource::ContentScan);
            source_trusted
                && matches!(
                    id.identifier_type,
                    IdentifierType::Isbn13 | IdentifierType::Isbn10 | IdentifierType::Asin
                )
        })
        .cloned()
        .collect()
}

/// Check whether the candidate's identifiers match any trusted
/// identifier on the book (User- or Embedded-source ISBN/ASIN).
///
/// Mirrors the resolver's `has_trusted_id_proof` logic:
/// - ISBN/ASIN values are normalized (strip whitespace/hyphens, uppercase).
/// - ISBN-10 ↔ ISBN-13 cross-matching via conversion.
fn candidate_has_strong_id_proof(
    candidate_identifiers: &[ProviderIdentifier],
    trusted_identifiers: &[Identifier],
) -> bool {
    for trusted in trusted_identifiers {
        for cand in candidate_identifiers {
            // Same-type normalized match.
            if cand.identifier_type == trusted.identifier_type {
                let matched = match trusted.identifier_type {
                    IdentifierType::Isbn13 | IdentifierType::Isbn10 => {
                        normalize_isbn(&trusted.value) == normalize_isbn(&cand.value)
                    }
                    IdentifierType::Asin => {
                        normalize_asin(&trusted.value) == normalize_asin(&cand.value)
                    }
                    _ => false,
                };
                if matched {
                    return true;
                }
            }

            // Cross-type ISBN-10 ↔ ISBN-13 matching.
            if matches!(
                (trusted.identifier_type, cand.identifier_type),
                (IdentifierType::Isbn10, IdentifierType::Isbn13)
                    | (IdentifierType::Isbn13, IdentifierType::Isbn10)
            ) {
                let trusted_13 = to_isbn13(&trusted.value, trusted.identifier_type);
                let cand_13 = to_isbn13(&cand.value, cand.identifier_type);
                if let (Some(t), Some(c)) = (trusted_13, cand_13) {
                    if t == c {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Heuristic: does `title` look like it was derived from a filename?
fn is_likely_filename(title: &str) -> bool {
    let extensions = [
        ".epub", ".pdf", ".mobi", ".azw", ".azw3", ".fb2", ".djvu", ".cbr", ".cbz", ".txt",
    ];
    let lower = title.to_lowercase();
    extensions.iter().any(|ext| lower.ends_with(ext))
}

/// Returns `true` when the current book title and the candidate title
/// are sufficiently different that overwriting the current title would
/// be destructive without strong ID proof.
///
/// NOT a contradiction when:
/// - The current title looks like a filename (always safe to replace).
/// - Either title is empty.
/// - One contains the other as a substring (e.g. "Dune" vs "Dune (Dune Chronicles #1)").
/// - Both titles share at least one significant word (len > 2).
fn titles_contradict(current: &str, candidate: &str) -> bool {
    if current.is_empty() || candidate.is_empty() {
        return false;
    }
    if is_likely_filename(current) {
        return false;
    }

    let a = current.to_lowercase();
    let b = candidate.to_lowercase();

    // Substring containment: not a contradiction.
    if a.contains(&b) || b.contains(&a) {
        return false;
    }

    // Shared significant-word check (words longer than 2 chars).
    let a_words: HashSet<&str> = a.split_whitespace().filter(|w| w.len() > 2).collect();
    let b_words: HashSet<&str> = b.split_whitespace().filter(|w| w.len() > 2).collect();

    a_words.is_disjoint(&b_words)
}

/// Decide whether the merge should touch core identity fields.
///
/// Logs the reason when the overwrite is blocked.
fn may_overwrite_core_with_log(ctx: &FieldApplyContext) -> bool {
    if ctx.may_overwrite_core() {
        return true;
    }
    if ctx.is_auto_apply && !ctx.has_strong_id_proof {
        debug!("core identity overwrite blocked: auto-apply without strong ID proof");
    } else if ctx.has_title_contradiction && !ctx.has_strong_id_proof {
        debug!("core identity overwrite blocked: title contradiction without strong ID proof");
    }
    false
}

/// Merge provider metadata fields into a book.
///
/// Fields are split into two categories:
///
/// **Core identity** (title):
///   Guarded by [`FieldApplyContext::may_overwrite_core`] — auto-apply
///   requires strong ID proof, and a contradiction guard blocks
///   overwrites when the candidate title strongly conflicts with the
///   current title and the candidate lacks strong ID proof.
///
/// **Enrichment** (subtitle, description, language, `page_count`,
///   `publication_date`):
///   Fill-if-empty, always safe to apply.
///
/// Fields listed in `exclude_fields` are skipped entirely.
/// Provider text fields are sanitized before applying.
fn merge_book_fields(
    book: &mut Book,
    provider_meta: &ProviderMetadata,
    exclude_fields: &HashSet<String>,
    ctx: &FieldApplyContext,
) {
    let sanitize_opts = SanitizeOptions::default();
    let core_allowed = may_overwrite_core_with_log(ctx);

    // ── Core identity: title ──
    if !exclude_fields.contains("title") && core_allowed {
        if let Some(ref title) = provider_meta.title {
            if let Some(clean_title) = sanitize_text(title, &sanitize_opts) {
                book.set_title(clean_title);
            }
        }
    }

    // ── Enrichment fields (fill-if-empty) ──

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

/// Trust rank for a `MetadataSource`.
///
/// Lower rank = higher trust. Ordering:
/// `User` > `Embedded` > `ContentScan` > `Provider` > `Filename`.
fn source_trust_rank(source: &MetadataSource) -> u8 {
    match source {
        MetadataSource::User => 0,
        MetadataSource::Embedded => 1,
        MetadataSource::ContentScan => 2,
        MetadataSource::Provider(_) => 3,
        MetadataSource::Filename => 4,
    }
}

/// Sorting key for deterministic, trust-aware identifier ranking.
///
/// Primary: source trust (lower rank = higher trust).
/// Secondary: confidence descending (negated for ascending sort).
/// Tiebreak: value string (lexicographic, for determinism).
fn identifier_sort_key(id: &Identifier) -> (u8, impl Ord + '_, &str) {
    let trust = source_trust_rank(&id.source);
    // Negate confidence so higher confidence sorts first in ascending order.
    // Convert to ordered integer to avoid float comparison issues.
    let neg_confidence = std::cmp::Reverse(ordered_confidence(id.confidence));
    (trust, neg_confidence, id.value.as_str())
}

/// Convert a 0.0–1.0 confidence to an ordered integer for
/// deterministic comparison without floating-point issues.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ordered_confidence(c: f32) -> u32 {
    // Input is clamped to [0.0, 1.0], so the result fits in u32 and is non-negative.
    (c.clamp(0.0, 1.0) * 10_000.0).round() as u32
}

/// Select the best identifier of a given type from a ranked list.
///
/// Returns the highest-trust, highest-confidence identifier matching
/// the requested type.
fn best_identifier_of_type(
    identifiers: &[Identifier],
    id_type: IdentifierType,
) -> Option<&Identifier> {
    identifiers
        .iter()
        .filter(|id| id.identifier_type == id_type)
        .min_by_key(|id| identifier_sort_key(id))
}

/// Type preference for ISBN selection.  ISBN-13 preferred over ISBN-10.
fn isbn_type_rank(id_type: IdentifierType) -> u8 {
    match id_type {
        IdentifierType::Isbn13 => 0,
        IdentifierType::Isbn10 => 1,
        _ => u8::MAX,
    }
}

/// Select the best ISBN identifier across both ISBN-13 and ISBN-10.
///
/// Trust is the primary sort, so a user/embedded ISBN-10 always beats
/// a provider ISBN-13.  Within the same trust level, ISBN-13 is preferred.
fn best_isbn(identifiers: &[Identifier]) -> Option<&Identifier> {
    identifiers
        .iter()
        .filter(|id| {
            matches!(
                id.identifier_type,
                IdentifierType::Isbn13 | IdentifierType::Isbn10
            )
        })
        .min_by_key(|id| {
            let trust = source_trust_rank(&id.source);
            let type_pref = isbn_type_rank(id.identifier_type);
            let neg_confidence = std::cmp::Reverse(ordered_confidence(id.confidence));
            (trust, type_pref, neg_confidence, id.value.as_str())
        })
}

/// Build a `MetadataQuery` from a book's existing metadata.
///
/// Identifier selection is deterministic and trust-aware:
/// - Primary: source trust (`User` > `Embedded` > `ContentScan` > `Provider` > `Filename`)
/// - Secondary: type preference (ISBN-13 > ISBN-10, within same trust level)
/// - Tertiary: confidence descending
/// - Tiebreak: lexicographic value (for stable ordering)
///
/// A higher-trust ISBN-10 always beats a lower-trust ISBN-13.
fn build_metadata_query(
    book: &Book,
    identifiers: &[Identifier],
    authors: &[String],
) -> MetadataQuery {
    let isbn = best_isbn(identifiers).map(|id| id.value.clone());

    let asin =
        best_identifier_of_type(identifiers, IdentifierType::Asin).map(|id| id.value.clone());

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

    // ── source_trust_rank tests ──

    #[test]
    fn trust_rank_ordering() {
        assert!(
            source_trust_rank(&MetadataSource::User) < source_trust_rank(&MetadataSource::Embedded)
        );
        assert!(
            source_trust_rank(&MetadataSource::Embedded)
                < source_trust_rank(&MetadataSource::ContentScan)
        );
        assert!(
            source_trust_rank(&MetadataSource::ContentScan)
                < source_trust_rank(&MetadataSource::Provider("any".into()))
        );
        assert!(
            source_trust_rank(&MetadataSource::Provider("any".into()))
                < source_trust_rank(&MetadataSource::Filename)
        );
    }

    #[test]
    fn trust_rank_provider_variants_equal() {
        assert_eq!(
            source_trust_rank(&MetadataSource::Provider("open_library".into())),
            source_trust_rank(&MetadataSource::Provider("hardcover".into())),
        );
    }

    // ── query ID ranking tests ──

    #[test]
    fn query_prefers_user_isbn_over_provider_isbn() {
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Provider("open_library".into()),
                1.0,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000002",
                MetadataSource::User,
                0.8,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780000000002"),
            "user ISBN must be selected over provider ISBN regardless of confidence"
        );
    }

    #[test]
    fn query_prefers_embedded_isbn_over_provider_isbn() {
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Provider("hardcover".into()),
                0.99,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000002",
                MetadataSource::Embedded,
                0.7,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780000000002"),
            "embedded ISBN must outrank provider ISBN"
        );
    }

    #[test]
    fn query_prefers_content_scan_isbn_over_provider_isbn() {
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Provider("google".into()),
                0.95,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000002",
                MetadataSource::ContentScan,
                0.6,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780000000002"),
            "content-scan ISBN must outrank provider ISBN"
        );
    }

    #[test]
    fn query_same_source_prefers_higher_confidence() {
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Embedded,
                0.7,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000002",
                MetadataSource::Embedded,
                0.9,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780000000002"),
            "higher-confidence identifier should win within same trust level"
        );
    }

    #[test]
    fn query_deterministic_tiebreak_by_value() {
        let book = Book::new("Test Book");
        // Same source, same confidence — tiebreak is lexicographic on value
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000099",
                MetadataSource::Embedded,
                0.9,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000011",
                MetadataSource::Embedded,
                0.9,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780000000011"),
            "same trust + confidence must tiebreak on value for determinism"
        );
    }

    #[test]
    fn query_deterministic_across_shuffled_input() {
        let book = Book::new("Test Book");

        let make_ids = |order: &[usize]| -> Vec<Identifier> {
            let all = [
                Identifier::new(
                    book.id,
                    IdentifierType::Isbn13,
                    "9780000000001",
                    MetadataSource::Provider("ol".into()),
                    1.0,
                ),
                Identifier::new(
                    book.id,
                    IdentifierType::Isbn13,
                    "9780000000002",
                    MetadataSource::Embedded,
                    0.9,
                ),
                Identifier::new(
                    book.id,
                    IdentifierType::Isbn13,
                    "9780000000003",
                    MetadataSource::User,
                    0.5,
                ),
                Identifier::new(
                    book.id,
                    IdentifierType::Asin,
                    "B00AAAA0001",
                    MetadataSource::Provider("amz".into()),
                    1.0,
                ),
                Identifier::new(
                    book.id,
                    IdentifierType::Asin,
                    "B00AAAA0002",
                    MetadataSource::Embedded,
                    0.8,
                ),
            ];
            order.iter().map(|&i| all[i].clone()).collect()
        };

        let orderings: &[&[usize]] = &[
            &[0, 1, 2, 3, 4],
            &[4, 3, 2, 1, 0],
            &[2, 0, 4, 1, 3],
            &[3, 1, 0, 4, 2],
        ];

        let mut results: Vec<(Option<String>, Option<String>)> = Vec::new();
        for order in orderings {
            let ids = make_ids(order);
            let query = build_metadata_query(&book, &ids, &[]);
            results.push((query.isbn, query.asin));
        }

        // All orderings must produce the same result
        for (i, result) in results.iter().enumerate().skip(1) {
            assert_eq!(
                &results[0], result,
                "ordering {i} produced different result than ordering 0"
            );
        }

        // User ISBN must win (trust rank 0, even with lowest confidence)
        assert_eq!(results[0].0.as_deref(), Some("9780000000003"));
        // Embedded ASIN must win over provider ASIN
        assert_eq!(results[0].1.as_deref(), Some("B00AAAA0002"));
    }

    #[test]
    fn query_provider_id_cannot_outrank_embedded_for_asin() {
        let book = Book::new("Kindle Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Asin,
                "B00PROVIDER1",
                MetadataSource::Provider("amazon".into()),
                1.0,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Asin,
                "B00EMBEDDED1",
                MetadataSource::Embedded,
                0.5,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.asin.as_deref(),
            Some("B00EMBEDDED1"),
            "provider ASIN must not outrank embedded ASIN"
        );
    }

    #[test]
    fn query_isbn13_user_beats_isbn10_provider() {
        // Both ISBN-13 candidates exist at user and provider trust.
        // User ISBN-13 should win (same trust as ISBN-10, but type preferred).
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Provider("ol".into()),
                1.0,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000002",
                MetadataSource::User,
                0.7,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0000000001",
                MetadataSource::User,
                1.0,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        // User ISBN-13 wins: same trust as User ISBN-10, but ISBN-13 preferred
        assert_eq!(query.isbn.as_deref(), Some("9780000000002"));
    }

    #[test]
    fn query_embedded_isbn10_beats_provider_isbn13() {
        // Cross-type trust invariant: a higher-trust ISBN-10 must beat
        // a lower-trust ISBN-13. Trust always outranks type preference.
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Provider("ol".into()),
                1.0,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0441172717",
                MetadataSource::Embedded,
                0.8,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("0441172717"),
            "embedded ISBN-10 must beat provider ISBN-13 (trust > type preference)"
        );
    }

    #[test]
    fn query_user_isbn10_beats_provider_isbn13() {
        // Strongest cross-type case: user ISBN-10 vs provider ISBN-13
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::Provider("hardcover".into()),
                1.0,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0441172717",
                MetadataSource::User,
                0.5,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("0441172717"),
            "user ISBN-10 must beat provider ISBN-13"
        );
    }

    #[test]
    fn query_same_trust_prefers_isbn13_over_isbn10() {
        // Within the same trust level, ISBN-13 is preferred over ISBN-10
        let book = Book::new("Test Book");
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

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780441172719"),
            "same trust level should prefer ISBN-13 over ISBN-10"
        );
    }

    #[test]
    fn query_falls_back_to_isbn10_when_no_isbn13() {
        let book = Book::new("Test Book");
        let identifiers = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0000000002",
                MetadataSource::Provider("ol".into()),
                1.0,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0000000001",
                MetadataSource::Embedded,
                0.8,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Asin,
                "B00AAAA0001",
                MetadataSource::Embedded,
                0.9,
            ),
        ];

        let query = build_metadata_query(&book, &identifiers, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("0000000001"),
            "should fall back to ISBN-10 with embedded source winning"
        );
    }

    // ── Integration: identifier attribution after cross-provider merge ──

    /// Minimal `SettingsReader` for test resolver construction.
    struct NoOpSettings;
    impl archivis_core::settings::SettingsReader for NoOpSettings {
        fn get_setting(&self, _key: &str) -> Option<serde_json::Value> {
            None
        }
    }

    /// Create a fresh test database.
    async fn test_pool() -> (archivis_db::DbPool, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&pool).await.unwrap();
        (pool, dir)
    }

    /// Build a `ProviderMetadata` simulating a cross-provider merged candidate.
    ///
    /// After merge the winner keeps: ISBNs (portable) + its own native IDs.
    /// The loser's native IDs are already stripped by the resolver.
    fn merged_provider_metadata() -> ProviderMetadata {
        ProviderMetadata {
            provider_name: "open_library".to_string(),
            title: Some("Dune".to_string()),
            subtitle: None,
            authors: vec![],
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![
                ProviderIdentifier {
                    identifier_type: IdentifierType::Isbn13,
                    value: "9780441172719".to_string(),
                },
                ProviderIdentifier {
                    identifier_type: IdentifierType::Isbn10,
                    value: "0441172717".to_string(),
                },
                ProviderIdentifier {
                    identifier_type: IdentifierType::OpenLibrary,
                    value: "OL123456M".to_string(),
                },
            ],
            subjects: Vec::new(),
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.95,
        }
    }

    #[tokio::test]
    async fn merged_candidate_stored_identifiers_correct() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));

        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        // Create a book in the DB.
        let book = Book::new("Dune");
        archivis_db::BookRepository::create(&pool, &book)
            .await
            .unwrap();

        // Simulate applying a merged candidate's identifiers.
        let merged_meta = merged_provider_metadata();
        service
            .add_provider_identifiers(book.id, &merged_meta, 0.95)
            .await
            .unwrap();

        // Verify stored identifiers.
        let stored = IdentifierRepository::get_by_book_id(&pool, book.id)
            .await
            .unwrap();

        // Should have exactly 3 identifiers: ISBN-13, ISBN-10, OpenLibrary.
        assert_eq!(
            stored.len(),
            3,
            "expected 3 identifiers (2 ISBNs + 1 winner native), got {}: {:?}",
            stored.len(),
            stored
                .iter()
                .map(|id| format!("{}={}", id.identifier_type, id.value))
                .collect::<Vec<_>>()
        );

        // All identifiers should be attributed to the winner's provider.
        for id in &stored {
            assert_eq!(
                id.source,
                MetadataSource::Provider("open_library".to_string()),
                "identifier {}={} has wrong source {:?}, expected Provider(\"open_library\")",
                id.identifier_type,
                id.value,
                id.source
            );
        }

        // Verify expected types are present.
        let types: HashSet<IdentifierType> = stored.iter().map(|id| id.identifier_type).collect();
        assert!(types.contains(&IdentifierType::Isbn13));
        assert!(types.contains(&IdentifierType::Isbn10));
        assert!(types.contains(&IdentifierType::OpenLibrary));

        // No hardcover IDs should be present (loser's native IDs stripped by resolver).
        assert!(
            !types.contains(&IdentifierType::Hardcover),
            "loser's native IDs should not be stored"
        );
    }

    // ── Field-apply context: may_overwrite_core tests ──

    #[test]
    fn auto_apply_without_proof_blocks_core() {
        let ctx = FieldApplyContext {
            is_auto_apply: true,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };
        assert!(!ctx.may_overwrite_core());
    }

    #[test]
    fn auto_apply_with_proof_allows_core() {
        let ctx = FieldApplyContext {
            is_auto_apply: true,
            has_strong_id_proof: true,
            has_title_contradiction: false,
        };
        assert!(ctx.may_overwrite_core());
    }

    #[test]
    fn manual_apply_no_contradiction_allows_core() {
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };
        assert!(ctx.may_overwrite_core());
    }

    #[test]
    fn manual_apply_contradiction_no_proof_blocks_core() {
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: false,
            has_title_contradiction: true,
        };
        assert!(!ctx.may_overwrite_core());
    }

    #[test]
    fn manual_apply_contradiction_with_proof_allows_core() {
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: true,
            has_title_contradiction: true,
        };
        assert!(ctx.may_overwrite_core());
    }

    // ── titles_contradict tests ──

    #[test]
    fn identical_titles_no_contradiction() {
        assert!(!titles_contradict("Dune", "Dune"));
    }

    #[test]
    fn substring_titles_no_contradiction() {
        assert!(!titles_contradict("Dune", "Dune (Dune Chronicles #1)"));
        assert!(!titles_contradict("Dune (Dune Chronicles #1)", "Dune"));
    }

    #[test]
    fn shared_word_no_contradiction() {
        assert!(!titles_contradict("The Dune Encyclopedia", "Dune Messiah"));
    }

    #[test]
    fn completely_different_titles_contradiction() {
        assert!(titles_contradict("The Great Gatsby", "Dune"));
    }

    #[test]
    fn empty_title_no_contradiction() {
        assert!(!titles_contradict("", "Dune"));
        assert!(!titles_contradict("Dune", ""));
    }

    #[test]
    fn filename_title_no_contradiction() {
        assert!(!titles_contradict("978044117271.epub", "Dune"));
        assert!(!titles_contradict(
            "some_book.pdf",
            "Completely Different Title"
        ));
    }

    #[test]
    fn case_insensitive_no_contradiction() {
        assert!(!titles_contradict("DUNE", "dune"));
        assert!(!titles_contradict("The DUNE Chronicles", "dune messiah"));
    }

    // ── is_likely_filename tests ──

    #[test]
    fn detects_common_ebook_extensions() {
        assert!(is_likely_filename("book.epub"));
        assert!(is_likely_filename("document.pdf"));
        assert!(is_likely_filename("book.mobi"));
        assert!(is_likely_filename("BOOK.EPUB"));
        assert!(!is_likely_filename("Dune"));
        assert!(!is_likely_filename("The Great Gatsby"));
    }

    // ── candidate_has_strong_id_proof tests ──

    #[test]
    fn matching_isbn_is_strong_proof() {
        let book_id = Uuid::new_v4();
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780441172719".to_string(),
        }];
        let trusted = vec![Identifier::new(
            book_id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        )];
        assert!(candidate_has_strong_id_proof(&candidate_ids, &trusted));
    }

    #[test]
    fn non_matching_isbn_no_proof() {
        let book_id = Uuid::new_v4();
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780000000001".to_string(),
        }];
        let trusted = vec![Identifier::new(
            book_id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        )];
        assert!(!candidate_has_strong_id_proof(&candidate_ids, &trusted));
    }

    #[test]
    fn no_trusted_identifiers_no_proof() {
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780441172719".to_string(),
        }];
        assert!(!candidate_has_strong_id_proof(&candidate_ids, &[]));
    }

    #[test]
    fn isbn10_cross_matches_isbn13_for_proof() {
        let book_id = Uuid::new_v4();
        // Candidate has ISBN-13, book has ISBN-10 (same edition).
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780441172719".to_string(),
        }];
        let trusted = vec![Identifier::new(
            book_id,
            IdentifierType::Isbn10,
            "0441172717",
            MetadataSource::Embedded,
            0.9,
        )];
        assert!(candidate_has_strong_id_proof(&candidate_ids, &trusted));
    }

    #[test]
    fn isbn_with_hyphens_matches_normalized() {
        let book_id = Uuid::new_v4();
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "978-0-441-17271-9".to_string(),
        }];
        let trusted = vec![Identifier::new(
            book_id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        )];
        assert!(candidate_has_strong_id_proof(&candidate_ids, &trusted));
    }

    #[test]
    fn asin_matches_case_insensitive() {
        let book_id = Uuid::new_v4();
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Asin,
            value: "b000fa5zeg".to_string(),
        }];
        let trusted = vec![Identifier::new(
            book_id,
            IdentifierType::Asin,
            "B000FA5ZEG",
            MetadataSource::User,
            1.0,
        )];
        assert!(candidate_has_strong_id_proof(&candidate_ids, &trusted));
    }

    // ── merge_book_fields with field policy tests ──

    fn make_provider_meta(title: &str) -> ProviderMetadata {
        ProviderMetadata {
            provider_name: "test_provider".to_string(),
            title: Some(title.to_string()),
            subtitle: Some("A Subtitle".to_string()),
            authors: vec![],
            description: Some("A description".to_string()),
            language: Some("en".to_string()),
            publisher: None,
            publication_date: Some("2020-01-01".to_string()),
            identifiers: vec![],
            subjects: Vec::new(),
            series: None,
            page_count: Some(300),
            cover_url: None,
            rating: None,
            confidence: 0.95,
        }
    }

    #[test]
    fn auto_apply_no_proof_blocks_title_overwrite() {
        let mut book = Book::new("Original Title");
        let provider = make_provider_meta("Provider Title");
        let ctx = FieldApplyContext {
            is_auto_apply: true,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        // Title must NOT be overwritten (auto-apply, no proof).
        assert_eq!(book.title, "Original Title");
        // Enrichment fields still applied.
        assert_eq!(book.description.as_deref(), Some("A description"));
        assert_eq!(book.language.as_deref(), Some("en"));
        assert_eq!(book.page_count, Some(300));
    }

    #[test]
    fn auto_apply_with_proof_allows_title_overwrite() {
        let mut book = Book::new("Original Title");
        let provider = make_provider_meta("Better Title");
        let ctx = FieldApplyContext {
            is_auto_apply: true,
            has_strong_id_proof: true,
            has_title_contradiction: false,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        assert_eq!(book.title, "Better Title");
    }

    #[test]
    fn manual_apply_no_proof_contradiction_blocks_title() {
        let mut book = Book::new("The Great Gatsby");
        let provider = make_provider_meta("Dune");
        // "The Great Gatsby" vs "Dune" → true contradiction.
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: false,
            has_title_contradiction: true,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        // Contradiction detected, no proof → title blocked.
        assert_eq!(book.title, "The Great Gatsby");
        // Enrichment fields still applied.
        assert_eq!(book.description.as_deref(), Some("A description"));
    }

    #[test]
    fn manual_apply_with_proof_allows_contradicting_title() {
        let mut book = Book::new("The Great Gatsby");
        let provider = make_provider_meta("Dune");
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: true,
            has_title_contradiction: true,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        // Proof present → title overwritten despite contradiction.
        assert_eq!(book.title, "Dune");
    }

    #[test]
    fn manual_apply_no_contradiction_allows_title() {
        let mut book = Book::new("Dune");
        let provider = make_provider_meta("Dune (Dune Chronicles #1)");
        // "Dune" ⊂ "Dune (Dune Chronicles #1)" → not a contradiction.
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        // No contradiction (substring) → title overwritten.
        assert_eq!(book.title, "Dune (Dune Chronicles #1)");
    }

    #[test]
    fn filename_title_always_overwritten() {
        let mut book = Book::new("978044117271.epub");
        let provider = make_provider_meta("Dune");
        // Filename → titles_contradict returns false.
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        // Filename title is never a contradiction → overwritten.
        assert_eq!(book.title, "Dune");
    }

    #[test]
    fn exclude_title_still_respected() {
        let mut book = Book::new("Original");
        let provider = make_provider_meta("New Title");
        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: true,
            has_title_contradiction: true,
        };
        let mut exclude = HashSet::new();
        exclude.insert("title".to_string());

        merge_book_fields(&mut book, &provider, &exclude, &ctx);

        assert_eq!(book.title, "Original");
    }

    #[test]
    fn enrichment_fields_apply_regardless_of_tier() {
        let mut book = Book::new("Some Title");
        let provider = make_provider_meta("Different Title");
        let ctx = FieldApplyContext {
            is_auto_apply: true,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx);

        // Core identity blocked, but all enrichment fields applied.
        assert_eq!(book.title, "Some Title");
        assert_eq!(book.subtitle.as_deref(), Some("A Subtitle"));
        assert_eq!(book.description.as_deref(), Some("A description"));
        assert_eq!(book.language.as_deref(), Some("en"));
        assert_eq!(book.page_count, Some(300));
        assert!(book.publication_date.is_some());
    }

    // ── Contradiction blocks ALL core identity fields ──

    #[tokio::test]
    async fn contradiction_blocks_authors_on_manual_apply() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));
        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        // Create a book with existing author (NOT "Unknown Author").
        let book = Book::new("The Great Gatsby");
        BookRepository::create(&pool, &book).await.unwrap();

        let author = archivis_core::models::Author::new("F. Scott Fitzgerald");
        AuthorRepository::create(&pool, &author).await.unwrap();
        BookRepository::add_author(&pool, book.id, author.id, "author", 0)
            .await
            .unwrap();

        // Candidate has completely different title and no matching ISBN.
        // This triggers the contradiction guard.
        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![archivis_metadata::types::ProviderAuthor {
                name: "Frank Herbert".into(),
                role: Some("author".into()),
            }],
            description: Some("Desert planet saga".into()),
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.90,
        };

        let meta_json = serde_json::to_value(&provider_meta).unwrap();
        let candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.90,
            meta_json,
            vec!["Title fuzzy match".into()],
        );
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        // Manual apply (no strong ID proof, title contradiction).
        let updated = service
            .apply_candidate(book.id, candidate.id, &HashSet::new())
            .await
            .unwrap();

        // Title must NOT be overwritten (contradiction, no proof).
        assert_eq!(updated.title, "The Great Gatsby");

        // Author must NOT be changed.
        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        let names: Vec<&str> = relations
            .authors
            .iter()
            .map(|a| a.author.name.as_str())
            .collect();
        assert!(
            names.contains(&"F. Scott Fitzgerald"),
            "author should be preserved under contradiction; got: {names:?}"
        );
        assert!(
            !names.contains(&"Frank Herbert"),
            "contradicting author should not be applied; got: {names:?}"
        );

        // Enrichment fields ARE applied.
        assert_eq!(updated.description.as_deref(), Some("Desert planet saga"));
    }

    #[tokio::test]
    async fn contradiction_blocks_series_on_manual_apply() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));
        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        let book = Book::new("The Great Gatsby");
        BookRepository::create(&pool, &book).await.unwrap();

        // Candidate with completely different title (contradiction)
        // and series info. No ISBN match.
        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![],
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![],
            subjects: vec![],
            series: Some(archivis_metadata::types::ProviderSeries {
                name: "Dune Chronicles".into(),
                position: Some(1.0),
            }),
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.80,
        };

        let meta_json = serde_json::to_value(&provider_meta).unwrap();
        let candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.80,
            meta_json,
            vec!["Title fuzzy match".into()],
        );
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        service
            .apply_candidate(book.id, candidate.id, &HashSet::new())
            .await
            .unwrap();

        // Series must NOT be added (contradiction, no proof).
        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        assert!(
            relations.series.is_empty(),
            "series should not be added under contradiction; got: {:?}",
            relations
                .series
                .iter()
                .map(|s| &s.series.name)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn strong_proof_allows_all_core_despite_contradiction() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));
        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        let book = Book::new("Wrong Title From Bad Import");
        BookRepository::create(&pool, &book).await.unwrap();

        // Add a trusted ISBN to the book.
        let isbn = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        );
        IdentifierRepository::create(&pool, &isbn).await.unwrap();

        // Candidate with matching ISBN (strong ID proof) but different title.
        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![],
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: Some(archivis_metadata::types::ProviderSeries {
                name: "Dune Chronicles".into(),
                position: Some(1.0),
            }),
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.95,
        };

        let meta_json = serde_json::to_value(&provider_meta).unwrap();
        let candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.95,
            meta_json,
            vec!["ISBN exact match".into()],
        );
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        let updated = service
            .apply_candidate(book.id, candidate.id, &HashSet::new())
            .await
            .unwrap();

        // Strong ID proof → title overwritten despite contradiction.
        assert_eq!(updated.title, "Dune");

        // Series applied.
        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(
            relations.series.len(),
            1,
            "series should be applied with strong ID proof"
        );
        assert_eq!(relations.series[0].series.name, "Dune Chronicles");
    }

    // ── Author trust/evidence policy tests ──

    /// Strong-ID proof allows correcting a stale wrong author even when
    /// the title contradicts (which would otherwise block core fields).
    /// Uses a contradicting title so that WITHOUT the ISBN match the
    /// author update would be blocked — proving causal dependence on
    /// strong-ID evidence.
    #[tokio::test]
    async fn strong_id_proof_corrects_stale_wrong_author() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));
        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        // Book with a wrong (stale) author and a title that will
        // contradict the provider's title.
        let book = Book::new("Wrong Title From Bad Import");
        BookRepository::create(&pool, &book).await.unwrap();

        let wrong_author = archivis_core::models::Author::new("Isaac Asimov");
        AuthorRepository::create(&pool, &wrong_author)
            .await
            .unwrap();
        BookRepository::add_author(&pool, book.id, wrong_author.id, "author", 0)
            .await
            .unwrap();

        // Trusted ISBN on the book.
        let isbn = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        );
        IdentifierRepository::create(&pool, &isbn).await.unwrap();

        // Candidate with matching ISBN, correct author, and a
        // completely different title (triggers contradiction).
        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![archivis_metadata::types::ProviderAuthor {
                name: "Frank Herbert".into(),
                role: Some("author".into()),
            }],
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.95,
        };

        let meta_json = serde_json::to_value(&provider_meta).unwrap();
        let candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.95,
            meta_json,
            vec!["ISBN exact match".into()],
        );
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        service
            .apply_candidate(book.id, candidate.id, &HashSet::new())
            .await
            .unwrap();

        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        let names: Vec<&str> = relations
            .authors
            .iter()
            .map(|a| a.author.name.as_str())
            .collect();
        assert!(
            names.contains(&"Frank Herbert"),
            "strong ID proof should correct stale author despite contradiction; got: {names:?}"
        );
        assert!(
            !names.contains(&"Isaac Asimov"),
            "stale wrong author should be replaced; got: {names:?}"
        );
    }

    #[tokio::test]
    async fn user_locked_metadata_blocks_author_update() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));
        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        // Book marked as user-locked (Identified + confidence 1.0).
        let mut book = Book::new("Dune");
        book.metadata_status = MetadataStatus::Identified;
        book.metadata_confidence = 1.0;
        BookRepository::create(&pool, &book).await.unwrap();

        let user_author = archivis_core::models::Author::new("Frank Herbert");
        AuthorRepository::create(&pool, &user_author).await.unwrap();
        BookRepository::add_author(&pool, book.id, user_author.id, "author", 0)
            .await
            .unwrap();

        // Trusted ISBN on the book.
        let isbn = Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        );
        IdentifierRepository::create(&pool, &isbn).await.unwrap();

        // Candidate with matching ISBN but different author.
        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![archivis_metadata::types::ProviderAuthor {
                name: "Brian Herbert".into(),
                role: Some("author".into()),
            }],
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.95,
        };

        let meta_json = serde_json::to_value(&provider_meta).unwrap();
        let candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.95,
            meta_json,
            vec!["ISBN exact match".into()],
        );
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        service
            .apply_candidate(book.id, candidate.id, &HashSet::new())
            .await
            .unwrap();

        // User-locked: author must NOT be replaced.
        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        let names: Vec<&str> = relations
            .authors
            .iter()
            .map(|a| a.author.name.as_str())
            .collect();
        assert!(
            names.contains(&"Frank Herbert"),
            "user-locked author should be preserved; got: {names:?}"
        );
        assert!(
            !names.contains(&"Brian Herbert"),
            "provider author should not replace user-locked; got: {names:?}"
        );

        // User-lock durability: confidence must stay at 1.0 so the
        // lock cannot be eroded by repeated applies.
        let updated = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert!(
            (updated.metadata_confidence - 1.0).abs() < f32::EPSILON,
            "user-lock confidence must be preserved across applies"
        );
    }

    #[tokio::test]
    async fn exclude_fields_authors_blocks_author_update() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");

        let registry = archivis_metadata::ProviderRegistry::new();
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));
        let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

        let book = Book::new("Dune");
        BookRepository::create(&pool, &book).await.unwrap();

        let original_author = archivis_core::models::Author::new("Frank Herbert");
        AuthorRepository::create(&pool, &original_author)
            .await
            .unwrap();
        BookRepository::add_author(&pool, book.id, original_author.id, "author", 0)
            .await
            .unwrap();

        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![archivis_metadata::types::ProviderAuthor {
                name: "Brian Herbert".into(),
                role: Some("author".into()),
            }],
            description: Some("A description".into()),
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.90,
        };

        let meta_json = serde_json::to_value(&provider_meta).unwrap();
        let candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.90,
            meta_json,
            vec!["Title match".into()],
        );
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        // Manual apply with "authors" excluded.
        let mut exclude = HashSet::new();
        exclude.insert("authors".to_string());
        let updated = service
            .apply_candidate(book.id, candidate.id, &exclude)
            .await
            .unwrap();

        // Enrichment fields applied.
        assert_eq!(updated.description.as_deref(), Some("A description"));

        // Authors must NOT be replaced.
        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        let names: Vec<&str> = relations
            .authors
            .iter()
            .map(|a| a.author.name.as_str())
            .collect();
        assert!(
            names.contains(&"Frank Herbert"),
            "excluded author should be preserved; got: {names:?}"
        );
        assert!(
            !names.contains(&"Brian Herbert"),
            "provider author should not be applied when excluded; got: {names:?}"
        );
    }

    // ── Content-scan evidence safety tests ──────────────────────────

    /// Alias for `filter_trusted_identifiers` (production function) in tests.
    fn filter_trusted(identifiers: &[Identifier]) -> Vec<Identifier> {
        filter_trusted_identifiers(identifiers)
    }

    /// Multiple `ContentScan` ISBNs must be excluded from `trusted_identifiers`
    /// — the noise guard still applies when 2+ scan ISBNs exist.
    #[test]
    fn multiple_scan_isbns_excluded_from_trusted() {
        let book = Book::new("Test Book");
        let identifiers = [
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780441172719",
                MetadataSource::ContentScan,
                0.5,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn10,
                "0441172717",
                MetadataSource::ContentScan,
                0.4,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Asin,
                "B000FA5ZEG",
                MetadataSource::ContentScan,
                0.3,
            ),
        ];

        let trusted = filter_trusted(&identifiers);
        assert!(
            trusted.is_empty(),
            "multiple ContentScan ISBNs must all be excluded from trusted_identifiers"
        );
    }

    /// A single scan ISBN is trusted — it's the book's own ISBN, not
    /// bibliography noise.
    #[test]
    fn single_scan_isbn_is_trusted() {
        let book = Book::new("Test Book");
        let identifiers = [Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::ContentScan,
            0.5,
        )];

        let trusted = filter_trusted(&identifiers);
        assert_eq!(trusted.len(), 1, "single scan ISBN should be trusted");
        assert_eq!(trusted[0].value, "9780441172719");
    }

    /// A single scan ISBN provides strong ID proof for auto-apply when the
    /// candidate matches.
    #[test]
    fn single_scan_isbn_provides_strong_id_proof() {
        let book = Book::new("Test Book");
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780441172719".to_string(),
        }];
        let identifiers = [Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::ContentScan,
            0.5,
        )];

        let trusted = filter_trusted(&identifiers);
        assert!(
            candidate_has_strong_id_proof(&candidate_ids, &trusted),
            "single scan ISBN must provide strong ID proof for auto-apply"
        );
    }

    /// When mixed sources exist, Embedded always provides proof. A single
    /// `ContentScan` ISBN also provides proof alongside Embedded.
    #[test]
    fn mixed_sources_trusted_provides_proof() {
        let book_id = Uuid::new_v4();
        let candidate_ids = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780441172719".to_string(),
        }];

        // Embedded identifier always counts.
        let trusted_embedded = vec![Identifier::new(
            book_id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::Embedded,
            0.9,
        )];
        assert!(
            candidate_has_strong_id_proof(&candidate_ids, &trusted_embedded),
            "embedded ISBN should provide proof"
        );

        // Single scan ISBN also provides proof after filter_trusted promotion.
        let scan_ids = [Identifier::new(
            book_id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::ContentScan,
            0.5,
        )];
        let trusted_scan = filter_trusted(&scan_ids);
        assert!(
            candidate_has_strong_id_proof(&candidate_ids, &trusted_scan),
            "single scan ISBN should also provide proof"
        );
    }

    /// Scan ISBNs are used for query building (higher trust rank than
    /// Provider). With a single scan ISBN it also provides proof; with
    /// a second scan ISBN present neither provides proof.
    #[test]
    fn scan_isbn_used_for_query_and_single_provides_proof() {
        let book = Book::new("Dune");

        // Single scan ISBN: used for query AND trusted.
        let single_scan = vec![Identifier::new(
            book.id,
            IdentifierType::Isbn13,
            "9780441172719",
            MetadataSource::ContentScan,
            0.5,
        )];
        let query = build_metadata_query(&book, &single_scan, &[]);
        assert_eq!(
            query.isbn.as_deref(),
            Some("9780441172719"),
            "scan ISBN must be preferred for query lookup"
        );
        let trusted = filter_trusted(&single_scan);
        assert_eq!(trusted.len(), 1, "single scan ISBN should be trusted");

        // Two scan ISBNs: still used for query, but neither is trusted.
        let multi_scan = vec![
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780441172719",
                MetadataSource::ContentScan,
                0.5,
            ),
            Identifier::new(
                book.id,
                IdentifierType::Isbn13,
                "9780000000001",
                MetadataSource::ContentScan,
                0.4,
            ),
        ];
        let trusted_multi = filter_trusted(&multi_scan);
        assert!(
            trusted_multi.is_empty(),
            "multiple scan ISBNs must not be trusted"
        );
    }
}
