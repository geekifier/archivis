use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::isbn::{normalize_asin, normalize_isbn, to_isbn13};
use archivis_core::models::metadata_rule::is_trusted_publisher;
use archivis_core::models::{
    ApplyChangeset, Book, CandidateStatus, ChangesetAuthor, ChangesetEntry, ChangesetSeries,
    FieldProvenance, IdentificationCandidate, Identifier, IdentifierType, MetadataProvenance,
    MetadataRule, MetadataSource, MetadataStatus, ResolutionOutcome as BookResolutionOutcome,
    ResolutionRunState, ResolutionState,
};
use archivis_db::{
    AuthorRepository, BookFileRepository, BookRepository, CandidateRepository, DbPool,
    IdentifierRepository, PublisherRepository, ResolutionRunRepository, SeriesRepository,
};
use archivis_formats::{
    sanitize::{sanitize_text, SanitizeOptions},
    CoverData,
};
use archivis_metadata::{
    CandidateMatchTier, ExistingBookMetadata, MetadataQuery, MetadataResolver, ProviderIdentifier,
    ProviderMetadata, ResolverDecision, ResolverResult,
};
use archivis_storage::StorageBackend;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::import::cover;
use crate::import::types::ThumbnailSizes;
use crate::resolve::planner::{
    extract_dispute_reasons, plan_automatic_reconciliation, CoreFieldInput, EnrichmentFieldInput,
    PlannedField, ReconciliationInput, ReconciliationPlan,
};
use crate::resolve::state::{
    persist_recomputed_status, update_status_with_floor, BookSnapshot, StatusContext,
};

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
/// **Enrichment** (subtitle, description, `publication_year`, identifiers,
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

/// The result of `resolve_book`, wrapping the resolver output with the
/// actual service-level auto-apply decision.
///
/// `decision_reason` uses a structured prefix from [`ResolverDecision`]:
/// - `auto_apply_allowed: tier=…, score=…`
/// - `reconciliation_confirmed: tier=…, score=…`
/// - `reconciliation_enriched: tier=…, score=…`
/// - `reconciliation_disputed: tier=…, score=…`
/// - `blocked_no_trusted_id: tier=…, score=…`
/// - `blocked_ambiguous: tier=…, score=…`
/// - `blocked_contradiction: tier=…, score=…`
/// - `blocked_low_tier: tier=…, score=…`
/// - `no_candidates`
/// - `auto_apply_failed: {error}` (resolver said yes, but reconciliation failed)
#[derive(Debug)]
pub struct ResolutionOutcome {
    /// The resolver's scored candidates and recommendation.
    pub resolver_result: ResolverResult,
    /// Durable run-history row for this attempt.
    pub run_id: Uuid,
    /// Whether automated reconciliation actually happened (not just whether
    /// the resolver recommended it — the executor step itself may fail).
    pub auto_applied: bool,
    /// Whether this run was superseded by a newer request before finalize.
    pub superseded: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualActionFailpoint {
    AfterCoverStaged,
    BeforeCommit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolutionPausePoint {
    BeforeFinalizeSupersessionCheck,
}

#[derive(Debug)]
struct StagedCover {
    cover_data: CoverData,
    new_path: String,
    old_path: Option<String>,
    old_bytes: Option<Vec<u8>>,
}

/// Orchestrates the resolution of books using external metadata providers.
pub struct ResolutionService<S: StorageBackend> {
    db_pool: DbPool,
    resolver: Arc<MetadataResolver>,
    storage: S,
    data_dir: PathBuf,
    thumbnail_sizes: ThumbnailSizes,
    #[cfg(test)]
    manual_action_failpoint: std::sync::Mutex<Option<ManualActionFailpoint>>,
    #[cfg(test)]
    resolution_pause: std::sync::Mutex<
        Option<(
            ResolutionPausePoint,
            Arc<tokio::sync::Notify>,
            Arc<tokio::sync::Notify>,
        )>,
    >,
}

impl<S: StorageBackend> ResolutionService<S> {
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
            #[cfg(test)]
            manual_action_failpoint: std::sync::Mutex::new(None),
            #[cfg(test)]
            resolution_pause: std::sync::Mutex::new(None),
        }
    }

    /// Access the database pool (used by workers to load metadata rules).
    pub fn db_pool(&self) -> &DbPool {
        &self.db_pool
    }

    #[cfg(test)]
    fn set_manual_action_failpoint(&self, failpoint: ManualActionFailpoint) {
        *self
            .manual_action_failpoint
            .lock()
            .expect("manual action failpoint lock poisoned") = Some(failpoint);
    }

    #[cfg(test)]
    fn set_resolution_pause(
        &self,
        point: ResolutionPausePoint,
        reached: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self
            .resolution_pause
            .lock()
            .expect("resolution pause lock poisoned") = Some((point, reached, release));
    }

    pub async fn resolve_queued_book(
        &self,
        book_id: Uuid,
        manual_refresh: bool,
        metadata_rules: &[MetadataRule],
    ) -> Result<Option<ResolutionOutcome>, TaskError> {
        let claimed = if manual_refresh {
            BookRepository::claim_manual_resolution(&self.db_pool, book_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to claim manual resolution: {e}")))?
        } else {
            BookRepository::claim_pending_resolution(&self.db_pool, book_id)
                .await
                .map_err(|e| {
                    TaskError::Failed(format!("failed to claim pending resolution: {e}"))
                })?
        };

        if !claimed {
            return Ok(None);
        }

        // Check metadata rules before running full resolution.
        // Manual refresh always bypasses rules so the user can force a provider query.
        if !manual_refresh {
            if let Some(skip_reason) = self.check_metadata_rules(book_id, metadata_rules).await? {
                BookRepository::mark_resolution_skipped(&self.db_pool, book_id, &skip_reason)
                    .await
                    .map_err(|e| {
                        TaskError::Failed(format!("failed to mark resolution skipped: {e}"))
                    })?;
                info!(book_id = %book_id, reason = %skip_reason, "resolution skipped by metadata rule");
                return Ok(None);
            }
        }

        match self.resolve_book(book_id, manual_refresh).await {
            Ok(outcome) => {
                if outcome.superseded {
                    BookRepository::mark_resolution_superseded(&self.db_pool, book_id)
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!(
                                "failed to requeue superseded resolution: {e}"
                            ))
                        })?;
                } else {
                    BookRepository::mark_resolution_done(&self.db_pool, book_id)
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!("failed to mark resolution done: {e}"))
                        })?;
                }
                Ok(Some(outcome))
            }
            Err(error) => {
                if let Err(mark_error) =
                    BookRepository::mark_resolution_failed(&self.db_pool, book_id).await
                {
                    warn!(
                        book_id = %book_id,
                        error = %mark_error,
                        "failed to mark resolution as failed"
                    );
                }
                Err(error)
            }
        }
    }

    /// Check whether metadata rules indicate this book should skip resolution.
    ///
    /// Returns `Some(reason)` if a rule matches, `None` otherwise.
    async fn check_metadata_rules(
        &self,
        book_id: Uuid,
        metadata_rules: &[MetadataRule],
    ) -> Result<Option<String>, TaskError> {
        if metadata_rules.is_empty() {
            return Ok(None);
        }

        let book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;

        let Some(pub_id) = book.publisher_id else {
            return Ok(None);
        };

        let publisher = PublisherRepository::get_by_id(&self.db_pool, pub_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load publisher: {e}")))?;

        if is_trusted_publisher(metadata_rules, &publisher.name) {
            return Ok(Some(format!("trusted:publisher:{}", publisher.name)));
        }

        Ok(None)
    }

    /// Resolve a single book by querying metadata providers.
    ///
    /// Builds a `MetadataQuery` from the book's existing metadata, queries
    /// the resolver, stores all candidates in the database, and optionally
    /// auto-applies the best match.
    ///
    /// When `manual_refresh` is `true`, auto-apply is suppressed so the user
    /// can review candidates and choose which fields (including cover) to apply.
    #[allow(clippy::too_many_lines)]
    pub async fn resolve_book(
        &self,
        book_id: Uuid,
        manual_refresh: bool,
    ) -> Result<ResolutionOutcome, TaskError> {
        // 1. Load book from DB with identifiers
        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
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
            "built metadata query for resolution"
        );

        let mut run = ResolutionRunRepository::start(
            &self.db_pool,
            book_id,
            book.resolution_requested_reason
                .as_deref()
                .unwrap_or("automatic"),
            resolution_query_json(&query),
            "running",
        )
        .await
        .map_err(|e| TaskError::Failed(format!("failed to create resolution run: {e}")))?;

        // 3. Build ExistingBookMetadata for cross-validation
        let publisher_name = if let Some(pid) = book.publisher_id {
            PublisherRepository::get_by_id(&self.db_pool, pid)
                .await
                .ok()
                .map(|p| p.name)
        } else {
            None
        };

        let existing = ExistingBookMetadata {
            title: Some(book.title.clone()),
            authors: authors.clone(),
            publisher: publisher_name,
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
        let best_tier = result.best_match.as_ref().map(|b| b.tier);
        let best_tier_text = best_tier.map(|tier| tier.to_string());
        let mut stored_candidates = Vec::with_capacity(result.candidates.len());

        // 5. Store all candidates bound to this run.
        for scored in &result.candidates {
            let metadata_json = match serde_json::to_value(&scored.metadata) {
                Ok(value) => value,
                Err(e) => {
                    let error = TaskError::Failed(format!("failed to serialize metadata: {e}"));
                    let candidate_count = i64::try_from(stored_candidates.len()).map_err(|_| {
                        TaskError::Failed("candidate count exceeded i64::MAX".into())
                    })?;
                    self.finalize_failed_run(
                        run.id,
                        candidate_count,
                        stored_candidates
                            .first()
                            .map(|stored: &IdentificationCandidate| stored.id),
                        stored_candidates.first().map(|stored| stored.score),
                        best_tier_text.as_deref(),
                        &error.to_string(),
                    )
                    .await;
                    return Err(error);
                }
            };

            let mut candidate = IdentificationCandidate::new(
                book_id,
                &scored.provider_name,
                scored.score,
                metadata_json,
                scored.match_reasons.clone(),
            );
            candidate.run_id = Some(run.id);
            candidate.tier = Some(scored.tier.to_string());
            let candidate_count = i64::try_from(stored_candidates.len())
                .map_err(|_| TaskError::Failed("candidate count exceeded i64::MAX".into()))?;

            self.run_step(
                run.id,
                candidate_count,
                stored_candidates
                    .first()
                    .map(|stored: &IdentificationCandidate| stored.id),
                stored_candidates.first().map(|stored| stored.score),
                best_tier_text.as_deref(),
                CandidateRepository::create(&self.db_pool, &candidate)
                    .await
                    .map_err(|e| TaskError::Failed(format!("failed to store candidate: {e}"))),
            )
            .await?;
            stored_candidates.push(candidate);
        }

        let best_candidate_id = stored_candidates.first().map(|candidate| candidate.id);
        let best_score = stored_candidates.first().map(|candidate| candidate.score);
        let candidate_count = i64::try_from(stored_candidates.len())
            .map_err(|_| TaskError::Failed("candidate count exceeded i64::MAX".into()))?;

        if self
            .run_step(
                run.id,
                candidate_count,
                best_candidate_id,
                best_score,
                best_tier_text.as_deref(),
                self.run_has_been_superseded(book_id, run.started_at).await,
            )
            .await?
        {
            return self
                .finalize_superseded_result(
                    run,
                    result,
                    candidate_count,
                    best_candidate_id,
                    best_score,
                    best_tier_text.clone(),
                    best_tier,
                )
                .await;
        }

        // 5b. Set review baseline when entering review (candidates exist).
        // Only set if not already set (handles second refresh while in review).
        if !stored_candidates.is_empty() && book.review_baseline_metadata_status.is_none() {
            BookRepository::set_review_baseline(
                &self.db_pool,
                book_id,
                Some(book.metadata_status),
                book.resolution_outcome,
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to set review baseline: {e}")))?;
            // Update local copy so persist_recomputed_status sees it
            book.review_baseline_metadata_status = Some(book.metadata_status);
            book.review_baseline_resolution_outcome = book.resolution_outcome;
        }

        // 6. Auto-apply decision.
        //
        // The resolver is the single authoritative source for the auto-apply
        // decision: it enforces tier (StrongIdMatch only), settings-driven
        // threshold, and ambiguity/gap guards.  The service trusts that
        // recommendation and only tracks whether the apply step succeeds.
        let mut auto_applied = false;
        let run_outcome: BookResolutionOutcome;
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

        if result.auto_apply && !manual_refresh {
            if let Some(ref best) = result.best_match {
                info!(
                    book_id = %book_id,
                    score = best.score,
                    tier = %best.tier,
                    provider = %best.provider_name,
                    "auto-applying best match"
                );

                if let Some(best_candidate_id) = best_candidate_id {
                    let series_names = self.load_series_names(book_id).await?;
                    let plan_input = build_reconciliation_input(
                        &book,
                        &authors,
                        &series_names,
                        &identifiers,
                        &best.metadata,
                        best.tier == CandidateMatchTier::StrongIdMatch,
                    );
                    let plan = plan_automatic_reconciliation(&plan_input);

                    // Write dispute reasons to the best candidate before auto-apply
                    // so `persist_recomputed_status` can see them.
                    let dispute_reasons = extract_dispute_reasons(&plan);
                    if !dispute_reasons.is_empty() {
                        if let Err(e) = CandidateRepository::update_disputes(
                            &self.db_pool,
                            best_candidate_id,
                            &dispute_reasons,
                        )
                        .await
                        {
                            warn!(
                                book_id = %book_id,
                                error = %e,
                                "failed to write dispute reasons to candidate"
                            );
                        }
                    }

                    match self
                        .execute_automatic_reconciliation(
                            book_id,
                            best_candidate_id,
                            best.score,
                            &best.metadata,
                            &plan,
                        )
                        .await
                    {
                        Ok(_) => {
                            auto_applied = matches!(
                                plan.outcome,
                                BookResolutionOutcome::Confirmed | BookResolutionOutcome::Enriched
                            );
                            run_outcome = plan.outcome;
                            decision_reason = format_reconciliation_decision(plan.outcome, best);
                        }
                        Err(e) => {
                            warn!(
                                book_id = %book_id,
                                error = %e,
                                "automatic reconciliation failed, candidates stored for manual review"
                            );
                            self.run_step(
                                run.id,
                                candidate_count,
                                Some(best_candidate_id),
                                best_score,
                                best_tier_text.as_deref(),
                                persist_recomputed_status(&self.db_pool, book_id)
                                    .await
                                    .map(|_| ()),
                            )
                            .await?;
                            run_outcome = BookResolutionOutcome::Ambiguous;
                            decision_reason = format!("auto_apply_failed: {e}");
                        }
                    }
                } else {
                    self.run_step(
                        run.id,
                        candidate_count,
                        best_candidate_id,
                        best_score,
                        best_tier_text.as_deref(),
                        persist_recomputed_status(&self.db_pool, book_id)
                            .await
                            .map(|_| ()),
                    )
                    .await?;
                    run_outcome = BookResolutionOutcome::Ambiguous;
                    decision_reason = "auto_apply_failed: no stored candidate found".into();
                }
            } else {
                self.run_step(
                    run.id,
                    candidate_count,
                    best_candidate_id,
                    best_score,
                    best_tier_text.as_deref(),
                    persist_recomputed_status(&self.db_pool, book_id)
                        .await
                        .map(|_| ()),
                )
                .await?;
                run_outcome = BookResolutionOutcome::Ambiguous;
                decision_reason = "auto_apply_failed: no best match".into();
            }
        } else if result.candidates.is_empty() {
            // 0 candidates: skip `persist_recomputed_status` to avoid
            // downgrading `metadata_status` via the simplified heuristic.
            self.run_step(
                run.id,
                candidate_count,
                best_candidate_id,
                best_score,
                best_tier_text.as_deref(),
                Ok(()),
            )
            .await?;
            run_outcome = BookResolutionOutcome::Unmatched;
            decision_reason = format_decision(result.decision, None);
        } else {
            // Candidates exist but auto-apply was not performed (resolver
            // declined, or manual refresh suppressed it).
            self.run_step(
                run.id,
                candidate_count,
                best_candidate_id,
                best_score,
                best_tier_text.as_deref(),
                persist_recomputed_status(&self.db_pool, book_id)
                    .await
                    .map(|_| ()),
            )
            .await?;
            run_outcome = BookResolutionOutcome::Ambiguous;
            decision_reason = if manual_refresh && result.auto_apply {
                format!(
                    "manual_refresh_review: {}",
                    format_decision(result.decision, result.best_match.as_ref())
                )
            } else {
                format_decision(result.decision, result.best_match.as_ref())
            };
        }

        self.pause_resolution_at(ResolutionPausePoint::BeforeFinalizeSupersessionCheck)
            .await;
        if self
            .run_step(
                run.id,
                candidate_count,
                best_candidate_id,
                best_score,
                best_tier_text.as_deref(),
                self.run_has_been_superseded(book_id, run.started_at).await,
            )
            .await?
        {
            return self
                .finalize_superseded_result(
                    run,
                    result,
                    candidate_count,
                    best_candidate_id,
                    best_score,
                    best_tier_text.clone(),
                    best_tier,
                )
                .await;
        }

        self.run_step(
            run.id,
            candidate_count,
            best_candidate_id,
            best_score,
            best_tier_text.as_deref(),
            CandidateRepository::mark_other_runs_superseded(&self.db_pool, book_id, run.id)
                .await
                .map(|_| ())
                .map_err(|e| {
                    TaskError::Failed(format!("failed to supersede older candidates: {e}"))
                }),
        )
        .await?;
        self.run_step(
            run.id,
            candidate_count,
            best_candidate_id,
            best_score,
            best_tier_text.as_deref(),
            ResolutionRunRepository::mark_older_runs_superseded(&self.db_pool, book_id, run.id)
                .await
                .map(|_| ())
                .map_err(|e| TaskError::Failed(format!("failed to supersede older runs: {e}"))),
        )
        .await?;
        run.state = ResolutionRunState::Done;
        run.outcome = Some(run_outcome);
        run.decision_code = decision_code(&decision_reason).into();
        run.candidate_count = candidate_count;
        run.best_candidate_id = best_candidate_id;
        run.best_score = best_score;
        run.best_tier = best_tier_text.clone();
        run.error = None;
        run.finished_at = Some(chrono::Utc::now());
        ResolutionRunRepository::finalize(&self.db_pool, &run)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to finalize resolution run: {e}")))?;
        self.persist_book_resolution_outcome(book_id, run_outcome)
            .await?;

        info!(
            book_id = %book_id,
            candidates = result.candidates.len(),
            auto_applied,
            best_tier = best_tier.as_ref().map_or_else(|| "none".to_string(), ToString::to_string),
            decision_reason = %decision_reason,
            "resolution complete"
        );

        Ok(ResolutionOutcome {
            resolver_result: result,
            run_id: run.id,
            auto_applied,
            superseded: false,
            best_tier,
            decision_reason,
        })
    }

    /// Compatibility shim for legacy callers still using the old service verb.
    async fn finalize_superseded_result(
        &self,
        mut run: archivis_core::models::ResolutionRun,
        result: ResolverResult,
        candidate_count: i64,
        best_candidate_id: Option<Uuid>,
        best_score: Option<f32>,
        best_tier_text: Option<String>,
        best_tier: Option<CandidateMatchTier>,
    ) -> Result<ResolutionOutcome, TaskError> {
        CandidateRepository::mark_run_superseded(&self.db_pool, run.id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to supersede run candidates: {e}")))?;
        run.state = ResolutionRunState::Superseded;
        run.outcome = None;
        run.decision_code = "superseded".into();
        run.candidate_count = candidate_count;
        run.best_candidate_id = best_candidate_id;
        run.best_score = best_score;
        run.best_tier = best_tier_text;
        run.error = None;
        run.finished_at = Some(chrono::Utc::now());
        ResolutionRunRepository::finalize(&self.db_pool, &run)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to finalize superseded run: {e}")))?;

        Ok(ResolutionOutcome {
            resolver_result: result,
            run_id: run.id,
            auto_applied: false,
            superseded: true,
            best_tier,
            decision_reason: "superseded".into(),
        })
    }

    #[allow(clippy::unused_async)]
    async fn pause_resolution_at(&self, point: ResolutionPausePoint) {
        #[cfg(test)]
        {
            let pause = self
                .resolution_pause
                .lock()
                .expect("resolution pause lock poisoned")
                .clone();
            if let Some((configured, reached, release)) = pause {
                if configured == point {
                    reached.notify_waiters();
                    release.notified().await;
                }
            }
        }

        let _ = point;
    }

    async fn run_step<T>(
        &self,
        run_id: Uuid,
        candidate_count: i64,
        best_candidate_id: Option<Uuid>,
        best_score: Option<f32>,
        best_tier: Option<&str>,
        result: Result<T, TaskError>,
    ) -> Result<T, TaskError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.finalize_failed_run(
                    run_id,
                    candidate_count,
                    best_candidate_id,
                    best_score,
                    best_tier,
                    &error.to_string(),
                )
                .await;
                Err(error)
            }
        }
    }

    async fn finalize_failed_run(
        &self,
        run_id: Uuid,
        candidate_count: i64,
        best_candidate_id: Option<Uuid>,
        best_score: Option<f32>,
        best_tier: Option<&str>,
        error: &str,
    ) {
        let run_result = ResolutionRunRepository::get_by_id(&self.db_pool, run_id).await;
        match run_result {
            Ok(Some(mut run)) => {
                run.state = ResolutionRunState::Failed;
                run.outcome = None;
                run.decision_code = "failed".into();
                run.candidate_count = candidate_count;
                run.best_candidate_id = best_candidate_id;
                run.best_score = best_score;
                run.best_tier = best_tier.map(ToOwned::to_owned);
                run.error = Some(error.into());
                run.finished_at = Some(chrono::Utc::now());

                if let Err(finalize_error) =
                    ResolutionRunRepository::finalize(&self.db_pool, &run).await
                {
                    warn!(
                        run_id = %run_id,
                        error = %finalize_error,
                        "failed to finalize failed resolution run"
                    );
                }
            }
            Ok(None) => {
                warn!(run_id = %run_id, "failed to finalize missing resolution run");
            }
            Err(fetch_error) => {
                warn!(
                    run_id = %run_id,
                    error = %fetch_error,
                    "failed to load resolution run for failure finalization"
                );
            }
        }
    }

    async fn persist_book_resolution_outcome(
        &self,
        book_id: Uuid,
        outcome: BookResolutionOutcome,
    ) -> Result<(), TaskError> {
        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| {
                TaskError::Failed(format!("failed to reload book for outcome update: {e}"))
            })?;

        if book.resolution_outcome == Some(outcome) {
            return Ok(());
        }

        book.resolution_outcome = Some(outcome);
        BookRepository::update(&self.db_pool, &book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to persist resolution outcome: {e}")))
    }

    async fn run_has_been_superseded(
        &self,
        book_id: Uuid,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, TaskError> {
        let current = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| {
                TaskError::Failed(format!("failed to reload book for supersession check: {e}"))
            })?;
        Ok(current.resolution_requested_at > started_at)
    }

    async fn list_candidate_peer_group(
        &self,
        book_id: Uuid,
        run_id: Option<Uuid>,
    ) -> Result<Vec<IdentificationCandidate>, TaskError> {
        match run_id {
            Some(run_id) => CandidateRepository::list_by_run(&self.db_pool, run_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to list run candidates: {e}"))),
            None => CandidateRepository::list_legacy_by_book(&self.db_pool, book_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to list legacy candidates: {e}"))),
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn execute_automatic_reconciliation(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
        candidate_score: f32,
        provider_meta: &ProviderMetadata,
        plan: &ReconciliationPlan,
    ) -> Result<Book, TaskError> {
        if !plan.should_apply_candidate {
            return persist_recomputed_status(&self.db_pool, book_id).await;
        }

        // Guard: do not auto-apply if there is already an undoable applied candidate
        let existing_applied = CandidateRepository::find_applied_for_book(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to check existing applied: {e}")))?;
        if existing_applied
            .as_ref()
            .is_some_and(|c| c.apply_changeset.is_some())
        {
            return persist_recomputed_status(&self.db_pool, book_id).await;
        }

        let candidate = CandidateRepository::get_by_id(&self.db_pool, candidate_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load candidate: {e}")))?
            .ok_or_else(|| TaskError::Failed(format!("candidate not found: {candidate_id}")))?;

        if candidate.status != CandidateStatus::Pending {
            return Err(TaskError::Failed(format!(
                "candidate already {}, cannot auto-apply",
                candidate.status
            )));
        }

        let peer_candidates = self
            .list_candidate_peer_group(book_id, candidate.run_id)
            .await?;

        if peer_candidates
            .iter()
            .any(|other| other.status == CandidateStatus::Applied && other.id != candidate_id)
        {
            return Err(TaskError::Failed(
                "another candidate is already applied for this run".into(),
            ));
        }

        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;
        let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

        // Snapshot pre-apply state for changeset construction
        let before_book = book.clone();
        let before_provenance = book.metadata_provenance.clone();
        let before_authors: Vec<ChangesetAuthor> = relations
            .authors
            .iter()
            .map(|entry| ChangesetAuthor {
                author_id: entry.author.id,
                name: entry.author.name.clone(),
                sort_name: entry.author.sort_name.clone(),
                role: entry.role.clone(),
                position: entry.position,
            })
            .collect();
        let before_series: Vec<ChangesetSeries> = relations
            .series
            .iter()
            .map(|entry| ChangesetSeries {
                series_id: entry.series.id,
                name: entry.series.name.clone(),
                position: entry.position,
            })
            .collect();

        let existing_series_link = provider_meta.series.as_ref().and_then(|prov_series| {
            relations
                .series
                .iter()
                .find(|entry| entry.series.name.eq_ignore_ascii_case(&prov_series.name))
                .map(|entry| (entry.series.id, entry.position))
        });
        let provider_provenance = provider_field_provenance(&provider_meta.provider_name);

        if plan.should_apply(PlannedField::Title) {
            if let Some(title) = sanitize_text(
                provider_meta.title.as_deref().unwrap_or_default(),
                &SanitizeOptions::default(),
            ) {
                book.set_title(title);
            }
        }

        if plan.should_apply(PlannedField::Subtitle) {
            book.subtitle = sanitize_text(
                provider_meta.subtitle.as_deref().unwrap_or_default(),
                &SanitizeOptions::default(),
            );
        }

        if plan.should_apply(PlannedField::Description) {
            book.description = sanitize_text(
                provider_meta.description.as_deref().unwrap_or_default(),
                &SanitizeOptions::default(),
            );
        }

        if plan.should_apply(PlannedField::Language) {
            book.language.clone_from(&provider_meta.language);
        }

        if plan.should_apply(PlannedField::PageCount) {
            book.page_count = provider_meta.page_count;
        }

        if plan.should_apply(PlannedField::PublicationYear) {
            book.publication_year = provider_meta.publication_year;
        }

        // Stage cover before the transaction (same pattern as manual-apply path)
        let staged_cover = if plan.should_apply(PlannedField::Cover) {
            if let Some(ref cover_url) = provider_meta.cover_url {
                match self
                    .stage_cover_for_manual_apply(book_id, &book, cover_url)
                    .await
                {
                    Ok(staged) => {
                        book.cover_path = Some(staged.new_path.clone());
                        Some(staged)
                    }
                    Err(e) => {
                        warn!(
                            book_id = %book_id,
                            error = %e,
                            "cover fetch/store failed during automatic reconciliation"
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let should_update_authors = plan.should_apply(PlannedField::Authors);
        let should_update_series = plan.should_apply(PlannedField::Series);

        // Set provenance for all changed fields
        set_scalar_provenance(&before_book, &mut book, &provider_provenance);
        if book.title != before_book.title {
            book.metadata_provenance.title = Some(provider_provenance.clone());
        }
        if should_update_authors {
            book.metadata_provenance.authors = Some(provider_provenance.clone());
        }
        if should_update_series {
            book.metadata_provenance.series = Some(provider_provenance.clone());
        }

        // Build changeset from actual mutations
        let mut changeset = build_scalar_changeset(&before_book, &book, &before_provenance);
        changeset.provider_name = provider_meta.provider_name.clone();
        if book.title != before_book.title {
            changeset.title = Some(ChangesetEntry {
                old_value: before_book.title.clone(),
                old_provenance: before_provenance.title.clone(),
            });
            changeset.sort_title = Some(ChangesetEntry {
                old_value: before_book.sort_title.clone(),
                old_provenance: None,
            });
        }
        if should_update_authors {
            changeset.authors = Some(ChangesetEntry {
                old_value: before_authors,
                old_provenance: before_provenance.authors.clone(),
            });
        }
        if should_update_series {
            changeset.series = Some(ChangesetEntry {
                old_value: before_series,
                old_provenance: before_provenance.series.clone(),
            });
        }

        let changeset_json = serde_json::to_string(&changeset)
            .map_err(|e| TaskError::Failed(format!("failed to serialize apply changeset: {e}")))?;

        // Wrap all DB mutations in a transaction
        let tx_result = async {
            let mut tx = self
                .db_pool
                .begin()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to open transaction: {e}")))?;

            // Commit any existing applied candidate (without changeset)
            if let Some(prev) = &existing_applied {
                CandidateRepository::commit_applied_conn(tx.as_mut(), prev.id)
                    .await
                    .map_err(|e| {
                        TaskError::Failed(format!(
                            "failed to commit previous applied candidate: {e}"
                        ))
                    })?;
            }

            if should_update_authors {
                self.update_authors_from_provider_conn(tx.as_mut(), book_id, provider_meta)
                    .await?;
            }

            if should_update_series {
                self.update_series_from_provider_conn(
                    tx.as_mut(),
                    book_id,
                    provider_meta,
                    existing_series_link,
                )
                .await?;
            }

            // Clear baseline — apply establishes new truth (mirrors manual `apply_candidate`)
            book.review_baseline_metadata_status = None;
            book.review_baseline_resolution_outcome = None;

            BookRepository::update_conn(tx.as_mut(), &book)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;

            if plan.should_apply(PlannedField::Identifiers) {
                self.add_provider_identifiers_conn(
                    tx.as_mut(),
                    book_id,
                    provider_meta,
                    candidate_score,
                )
                .await?;
            }

            CandidateRepository::update_status_conn(
                tx.as_mut(),
                candidate_id,
                CandidateStatus::Applied,
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update candidate status: {e}")))?;

            CandidateRepository::set_apply_changeset_conn(
                tx.as_mut(),
                candidate_id,
                Some(&changeset_json),
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to store apply changeset: {e}")))?;

            for other in &peer_candidates {
                if other.id != candidate_id && other.status == CandidateStatus::Pending {
                    CandidateRepository::update_status_conn(
                        tx.as_mut(),
                        other.id,
                        CandidateStatus::Rejected,
                    )
                    .await
                    .map_err(|e| {
                        TaskError::Failed(format!("failed to reject other candidate: {e}"))
                    })?;
                }
            }

            tx.commit().await.map_err(|e| {
                TaskError::Failed(format!("failed to commit automatic reconciliation: {e}"))
            })?;

            Ok(())
        }
        .await;

        if let Err(error) = tx_result {
            if let Some(staged) = staged_cover.as_ref() {
                self.rollback_staged_cover(book_id, staged).await;
            }
            return Err(error);
        }

        if let Some(staged) = staged_cover {
            self.finalize_staged_cover(book_id, staged).await;
        }

        persist_recomputed_status(&self.db_pool, book_id).await
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
    /// `publication_year`, `authors`, `identifiers`, `series`, `cover`.
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

    pub async fn reject_candidate(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
    ) -> Result<(), TaskError> {
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
                "candidate already {}, cannot reject",
                candidate.status
            )));
        }

        let peer_candidates = self
            .list_candidate_peer_group(book_id, candidate.run_id)
            .await?;
        let mut next_candidates = peer_candidates.clone();
        if let Some(entry) = next_candidates
            .iter_mut()
            .find(|other| other.id == candidate_id)
        {
            entry.status = CandidateStatus::Rejected;
        }

        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;
        let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

        let (ctx, has_applied) = StatusContext::from_candidates(&next_candidates);
        let snapshot = BookSnapshot {
            has_authors: !relations.authors.is_empty(),
            has_identifiers: !relations.identifiers.is_empty(),
            has_applied_candidate: has_applied,
        };
        update_status_with_floor(&mut book, &snapshot, &ctx);

        let tx_result = async {
            let mut tx = self
                .db_pool
                .begin()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to open transaction: {e}")))?;

            CandidateRepository::update_status_conn(
                tx.as_mut(),
                candidate_id,
                CandidateStatus::Rejected,
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to reject candidate: {e}")))?;
            BookRepository::update_conn(tx.as_mut(), &book)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;
            self.maybe_fail_manual_action(ManualActionFailpoint::BeforeCommit)?;
            tx.commit()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to commit rejection: {e}")))?;

            Ok(())
        }
        .await;

        tx_result?;

        Ok(())
    }

    /// Batch-reject multiple candidates for a book in a single transaction.
    pub async fn reject_candidates(
        &self,
        book_id: Uuid,
        candidate_ids: &[Uuid],
    ) -> Result<(), TaskError> {
        if candidate_ids.is_empty() {
            return Ok(());
        }

        // Load and validate all candidates
        let mut to_reject = Vec::with_capacity(candidate_ids.len());
        for &cid in candidate_ids {
            let candidate = CandidateRepository::get_by_id(&self.db_pool, cid)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to load candidate: {e}")))?
                .ok_or_else(|| TaskError::Failed(format!("candidate not found: {cid}")))?;

            if candidate.book_id != book_id {
                return Err(TaskError::Failed(format!(
                    "candidate {cid} does not belong to book {book_id}"
                )));
            }

            if candidate.status != CandidateStatus::Pending {
                return Err(TaskError::Failed(format!(
                    "candidate {cid} already {}, cannot reject",
                    candidate.status
                )));
            }

            to_reject.push(candidate);
        }

        // Use the first candidate's `run_id` to load the peer group for status recomputation
        let peer_candidates = self
            .list_candidate_peer_group(book_id, to_reject[0].run_id)
            .await?;

        let reject_set: std::collections::HashSet<Uuid> = candidate_ids.iter().copied().collect();
        let next_candidates: Vec<IdentificationCandidate> = peer_candidates
            .into_iter()
            .map(|mut c| {
                if reject_set.contains(&c.id) {
                    c.status = CandidateStatus::Rejected;
                }
                c
            })
            .collect();

        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;
        let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

        let (ctx, has_applied) = StatusContext::from_candidates(&next_candidates);
        let snapshot = BookSnapshot {
            has_authors: !relations.authors.is_empty(),
            has_identifiers: !relations.identifiers.is_empty(),
            has_applied_candidate: has_applied,
        };
        update_status_with_floor(&mut book, &snapshot, &ctx);

        let mut tx = self
            .db_pool
            .begin()
            .await
            .map_err(|e| TaskError::Failed(format!("failed to open transaction: {e}")))?;

        for &cid in candidate_ids {
            CandidateRepository::update_status_conn(tx.as_mut(), cid, CandidateStatus::Rejected)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to reject candidate {cid}: {e}")))?;
        }

        BookRepository::update_conn(tx.as_mut(), &book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| TaskError::Failed(format!("failed to commit batch rejection: {e}")))?;

        Ok(())
    }

    /// Keep the current metadata for a book: reject all pending candidates and
    /// mark resolution as done with a `Confirmed` outcome.
    pub async fn keep_current_metadata(&self, book_id: Uuid) -> Result<(), TaskError> {
        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;

        // Load all current reviewable candidates to reject any pending ones
        let candidates = CandidateRepository::list_by_book(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load candidates: {e}")))?;

        let pending_ids: Vec<Uuid> = candidates
            .iter()
            .filter(|c| c.status == CandidateStatus::Pending)
            .map(|c| c.id)
            .collect();

        book.review_baseline_metadata_status = None;
        book.review_baseline_resolution_outcome = None;
        book.metadata_status = MetadataStatus::Identified;
        book.resolution_outcome = Some(BookResolutionOutcome::Confirmed);
        book.resolution_state = ResolutionState::Done;

        let mut tx = self
            .db_pool
            .begin()
            .await
            .map_err(|e| TaskError::Failed(format!("failed to open transaction: {e}")))?;

        for cid in &pending_ids {
            CandidateRepository::update_status_conn(tx.as_mut(), *cid, CandidateStatus::Rejected)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to reject candidate: {e}")))?;
        }

        BookRepository::update_conn(tx.as_mut(), &book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| TaskError::Failed(format!("failed to commit keep-metadata: {e}")))?;

        Ok(())
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

        let provider_meta: ProviderMetadata = serde_json::from_value(candidate.metadata.clone())
            .map_err(|e| {
                TaskError::Failed(format!("failed to deserialize candidate metadata: {e}"))
            })?;

        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;
        let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

        // Snapshot pre-apply state for changeset construction
        let before_book = book.clone();
        let before_authors: Vec<ChangesetAuthor> = relations
            .authors
            .iter()
            .map(|entry| ChangesetAuthor {
                author_id: entry.author.id,
                name: entry.author.name.clone(),
                sort_name: entry.author.sort_name.clone(),
                role: entry.role.clone(),
                position: entry.position,
            })
            .collect();
        let before_series: Vec<ChangesetSeries> = relations
            .series
            .iter()
            .map(|entry| ChangesetSeries {
                series_id: entry.series.id,
                name: entry.series.name.clone(),
                position: entry.position,
            })
            .collect();

        let book_identifiers = relations.identifiers.clone();
        let has_authors_before = !relations.authors.is_empty();
        let existing_series_link = provider_meta.series.as_ref().and_then(|prov_series| {
            relations
                .series
                .iter()
                .find(|entry| entry.series.name.eq_ignore_ascii_case(&prov_series.name))
                .map(|entry| (entry.series.id, entry.position))
        });

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

        let provenance_snapshot = book.metadata_provenance.clone();
        merge_book_fields(
            &mut book,
            &provider_meta,
            exclude_fields,
            &apply_ctx,
            &provenance_snapshot,
        );

        let preserve_existing_cover =
            cover_policy == CoverApplyPolicy::PreserveExisting && book.cover_path.is_some();
        let should_apply_cover = !(exclude_fields.contains("cover") || preserve_existing_cover);
        let should_update_identifiers = !exclude_fields.contains("identifiers");
        let should_update_authors = !exclude_fields.contains("authors")
            && (!is_auto_apply || !is_protected(provenance_snapshot.authors.as_ref()))
            && apply_ctx.may_overwrite_core()
            && !provider_meta.authors.is_empty();
        let should_update_series = !exclude_fields.contains("series")
            && (!is_auto_apply || !is_protected(provenance_snapshot.series.as_ref()))
            && apply_ctx.may_overwrite_core()
            && provider_meta.series.is_some();

        let should_update_publisher = !exclude_fields.contains("publisher")
            && (!is_auto_apply
                || (book.publisher_id.is_none()
                    && !is_protected(provenance_snapshot.publisher.as_ref())))
            && provider_meta.publisher.is_some();

        // Set provenance for changed scalar fields
        let provider_prov = provider_field_provenance(&provider_meta.provider_name);
        if book.title != before_book.title {
            book.metadata_provenance.title = Some(provider_prov.clone());
        }
        set_scalar_provenance(&before_book, &mut book, &provider_prov);
        if should_update_publisher {
            book.metadata_provenance.publisher = Some(provider_prov.clone());
        }
        if should_update_authors {
            book.metadata_provenance.authors = Some(provider_prov.clone());
        }
        if should_update_series {
            book.metadata_provenance.series = Some(provider_prov.clone());
        }

        let staged_cover = if should_apply_cover {
            if let Some(ref cover_url) = provider_meta.cover_url {
                match self
                    .stage_cover_for_manual_apply(book_id, &book, cover_url)
                    .await
                {
                    Ok(staged) => {
                        book.cover_path = Some(staged.new_path.clone());
                        book.metadata_provenance.cover = Some(provider_prov.clone());
                        Some(staged)
                    }
                    Err(error) => {
                        warn!(
                            book_id = %book_id,
                            error = %error,
                            "cover fetch/store failed, continuing without cover"
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        if staged_cover.is_some() {
            if let Err(error) =
                self.maybe_fail_manual_action(ManualActionFailpoint::AfterCoverStaged)
            {
                if let Some(staged) = staged_cover.as_ref() {
                    self.rollback_staged_cover(book_id, staged).await;
                }
                return Err(error);
            }
        }

        // Build changeset from actual mutations
        let mut changeset = build_scalar_changeset(&before_book, &book, &provenance_snapshot);
        changeset.provider_name = provider_meta.provider_name.clone();
        if book.title != before_book.title {
            changeset.title = Some(ChangesetEntry {
                old_value: before_book.title.clone(),
                old_provenance: provenance_snapshot.title.clone(),
            });
            changeset.sort_title = Some(ChangesetEntry {
                old_value: before_book.sort_title.clone(),
                old_provenance: None,
            });
        }
        if should_update_publisher {
            changeset.publisher_id = Some(ChangesetEntry {
                old_value: before_book.publisher_id,
                old_provenance: provenance_snapshot.publisher.clone(),
            });
        }
        if should_update_authors {
            changeset.authors = Some(ChangesetEntry {
                old_value: before_authors,
                old_provenance: provenance_snapshot.authors.clone(),
            });
        }
        if should_update_series {
            changeset.series = Some(ChangesetEntry {
                old_value: before_series,
                old_provenance: provenance_snapshot.series.clone(),
            });
        }

        // Capture pre-apply status/baseline/outcome for undo restore
        changeset.old_metadata_status = Some(book.metadata_status);
        changeset.old_review_baseline_metadata_status = Some(book.review_baseline_metadata_status);
        changeset.old_resolution_outcome = Some(book.resolution_outcome);
        changeset.old_review_baseline_resolution_outcome =
            Some(book.review_baseline_resolution_outcome);

        let changeset_json = serde_json::to_string(&changeset)
            .map_err(|e| TaskError::Failed(format!("failed to serialize apply changeset: {e}")))?;

        let peer_candidates = self
            .list_candidate_peer_group(book_id, candidate.run_id)
            .await?;

        let mut next_candidates = peer_candidates.clone();
        for other in &mut next_candidates {
            if other.id == candidate_id {
                other.status = CandidateStatus::Applied;
            } else if other.status == CandidateStatus::Pending {
                other.status = CandidateStatus::Rejected;
            }
        }

        // Clear baseline — apply establishes new truth
        book.review_baseline_metadata_status = None;
        book.review_baseline_resolution_outcome = None;

        let (ctx, has_applied) = StatusContext::from_candidates(&next_candidates);
        let snapshot = BookSnapshot {
            has_authors: should_update_authors || has_authors_before,
            has_identifiers: identifiers_present_after_apply(
                &book_identifiers,
                &provider_meta,
                should_update_identifiers,
            ),
            has_applied_candidate: has_applied,
        };
        update_status_with_floor(&mut book, &snapshot, &ctx);
        book.resolution_outcome = Some(BookResolutionOutcome::Enriched);

        let tx_result = async {
            let mut tx = self
                .db_pool
                .begin()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to open transaction: {e}")))?;

            // Commit any existing undoable applied candidate for this book
            let existing_applied =
                CandidateRepository::find_applied_for_book(&self.db_pool, book_id)
                    .await
                    .map_err(|e| {
                        TaskError::Failed(format!("failed to check existing applied: {e}"))
                    })?;
            if let Some(prev) = existing_applied {
                if prev.id != candidate_id {
                    CandidateRepository::commit_applied_conn(tx.as_mut(), prev.id)
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!(
                                "failed to commit previous applied candidate: {e}"
                            ))
                        })?;
                }
            }

            if should_update_identifiers {
                self.add_provider_identifiers_conn(
                    tx.as_mut(),
                    book_id,
                    &provider_meta,
                    candidate.score,
                )
                .await?;
            }

            if should_update_authors {
                self.update_authors_from_provider_conn(tx.as_mut(), book_id, &provider_meta)
                    .await?;
            }

            if should_update_series {
                self.update_series_from_provider_conn(
                    tx.as_mut(),
                    book_id,
                    &provider_meta,
                    existing_series_link,
                )
                .await?;
            }

            if should_update_publisher {
                if let Some(ref publisher_name) = provider_meta.publisher {
                    let publisher = if let Some(existing) =
                        PublisherRepository::find_by_name_conn(tx.as_mut(), publisher_name)
                            .await
                            .map_err(|e| {
                                TaskError::Failed(format!("publisher lookup failed: {e}"))
                            })? {
                        existing
                    } else {
                        let new_pub = archivis_core::models::Publisher::new(publisher_name);
                        PublisherRepository::create_conn(tx.as_mut(), &new_pub)
                            .await
                            .map_err(|e| {
                                TaskError::Failed(format!("publisher create failed: {e}"))
                            })?;
                        new_pub
                    };
                    book.publisher_id = Some(publisher.id);
                }
            }

            CandidateRepository::update_status_conn(
                tx.as_mut(),
                candidate_id,
                CandidateStatus::Applied,
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update candidate status: {e}")))?;

            CandidateRepository::set_apply_changeset_conn(
                tx.as_mut(),
                candidate_id,
                Some(&changeset_json),
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to store apply changeset: {e}")))?;

            for other in &peer_candidates {
                if other.id != candidate_id && other.status == CandidateStatus::Pending {
                    CandidateRepository::update_status_conn(
                        tx.as_mut(),
                        other.id,
                        CandidateStatus::Rejected,
                    )
                    .await
                    .map_err(|e| {
                        TaskError::Failed(format!("failed to reject other candidate: {e}"))
                    })?;
                }
            }

            BookRepository::update_conn(tx.as_mut(), &book)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;
            self.maybe_fail_manual_action(ManualActionFailpoint::BeforeCommit)?;
            tx.commit()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to commit candidate apply: {e}")))?;

            Ok(())
        }
        .await;

        if let Err(error) = tx_result {
            if let Some(staged) = staged_cover.as_ref() {
                self.rollback_staged_cover(book_id, staged).await;
            }
            return Err(error);
        }

        if let Some(staged) = staged_cover {
            self.finalize_staged_cover(book_id, staged).await;
        }

        info!(
            book_id = %book_id,
            candidate_id = %candidate_id,
            provider = %provider_meta.provider_name,
            "candidate applied successfully"
        );

        Ok(book)
    }

    /// Undo a previously applied candidate using its stored changeset.
    ///
    /// Restores fields that were changed by the apply, guarded by provenance:
    /// fields edited by the user after the apply are left untouched.
    /// Removes identifiers added by the candidate's provider.
    #[allow(clippy::too_many_lines)]
    pub async fn undo_candidate(
        &self,
        book_id: Uuid,
        candidate_id: Uuid,
    ) -> Result<Book, TaskError> {
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

        let changeset: ApplyChangeset = candidate
            .apply_changeset
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| TaskError::Failed(format!("failed to deserialize changeset: {e}")))?
            .ok_or_else(|| {
                TaskError::Failed(
                    "no changeset — this apply has been committed and cannot be undone".into(),
                )
            })?;

        let mut book = BookRepository::get_by_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book: {e}")))?;

        // Provenance-guarded scalar field restore
        let mut title_restored = false;
        if let Some(ref entry) = changeset.title {
            if provenance_matches_provider(
                book.metadata_provenance.title.as_ref(),
                &changeset.provider_name,
            ) {
                book.title = entry.old_value.clone();
                book.metadata_provenance.title = entry.old_provenance.clone();
                title_restored = true;
            }
        }
        // `sort_title` follows title — only restore if title was actually restored
        if title_restored {
            if let Some(ref entry) = changeset.sort_title {
                book.sort_title = entry.old_value.clone();
            }
        }
        undo_scalar_fields(&mut book, &changeset, &changeset.provider_name);
        if let Some(ref entry) = changeset.publisher_id {
            if provenance_matches_provider(
                book.metadata_provenance.publisher.as_ref(),
                &changeset.provider_name,
            ) {
                book.publisher_id = entry.old_value;
                book.metadata_provenance.publisher = entry.old_provenance.clone();
            }
        }

        // Determine new status for the candidate after undo
        let is_superseded_run = if let Some(run_id) = candidate.run_id {
            ResolutionRunRepository::get_by_id(&self.db_pool, run_id)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to load resolution run: {e}")))?
                .is_some_and(|run| run.state == ResolutionRunState::Superseded)
        } else {
            false
        };

        let new_candidate_status = if is_superseded_run {
            CandidateStatus::Superseded
        } else {
            CandidateStatus::Pending
        };

        let tx_result = async {
            let mut tx = self
                .db_pool
                .begin()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to open transaction: {e}")))?;

            // Restore authors if provenance still matches
            if let Some(ref entry) = changeset.authors {
                if provenance_matches_provider(
                    book.metadata_provenance.authors.as_ref(),
                    &changeset.provider_name,
                ) {
                    BookRepository::clear_authors_conn(tx.as_mut(), book_id)
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!("failed to clear authors for undo: {e}"))
                        })?;

                    #[allow(clippy::cast_possible_truncation)]
                    for author_snapshot in &entry.old_value {
                        // Re-use existing author or create
                        let db_author = if let Some(existing) =
                            AuthorRepository::find_by_name_conn(tx.as_mut(), &author_snapshot.name)
                                .await
                                .map_err(|e| {
                                    TaskError::Failed(format!("author lookup failed: {e}"))
                                })? {
                            existing
                        } else {
                            let new_author =
                                archivis_core::models::Author::new(&author_snapshot.name);
                            AuthorRepository::create_conn(tx.as_mut(), &new_author)
                                .await
                                .map_err(|e| {
                                    TaskError::Failed(format!("author create failed: {e}"))
                                })?;
                            new_author
                        };

                        BookRepository::add_author_conn(
                            tx.as_mut(),
                            book_id,
                            db_author.id,
                            &author_snapshot.role,
                            author_snapshot.position as i32,
                        )
                        .await
                        .map_err(|e| TaskError::Failed(format!("add author failed: {e}")))?;
                    }
                    book.metadata_provenance
                        .authors
                        .clone_from(&entry.old_provenance);
                }
            }

            // Restore series if provenance still matches
            if let Some(ref entry) = changeset.series {
                if provenance_matches_provider(
                    book.metadata_provenance.series.as_ref(),
                    &changeset.provider_name,
                ) {
                    BookRepository::clear_series_conn(tx.as_mut(), book_id)
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!("failed to clear series for undo: {e}"))
                        })?;

                    for series_snapshot in &entry.old_value {
                        let series = SeriesRepository::find_or_create_conn(
                            tx.as_mut(),
                            &series_snapshot.name,
                        )
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!("series find_or_create failed: {e}"))
                        })?;

                        BookRepository::add_series_conn(
                            tx.as_mut(),
                            book_id,
                            series.id,
                            series_snapshot.position,
                        )
                        .await
                        .map_err(|e| TaskError::Failed(format!("add series failed: {e}")))?;
                    }
                    book.metadata_provenance
                        .series
                        .clone_from(&entry.old_provenance);
                }
            }

            // Remove provider identifiers (unchanged from before — additive/selective)
            let removed = IdentifierRepository::delete_by_provider_conn(
                tx.as_mut(),
                book_id,
                &changeset.provider_name,
            )
            .await
            .map_err(|e| {
                TaskError::Failed(format!("failed to remove provider identifiers: {e}"))
            })?;

            debug!(
                book_id = %book_id,
                provider = %changeset.provider_name,
                removed = removed,
                "removed provider identifiers"
            );

            CandidateRepository::update_status_conn(
                tx.as_mut(),
                candidate_id,
                new_candidate_status,
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to reset candidate status: {e}")))?;

            // Clear the changeset
            CandidateRepository::set_apply_changeset_conn(tx.as_mut(), candidate_id, None)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to clear apply changeset: {e}")))?;

            // If candidate goes back to Pending, reset peers from Rejected to Pending
            if new_candidate_status == CandidateStatus::Pending {
                let peer_candidates = self
                    .list_candidate_peer_group(book_id, candidate.run_id)
                    .await?;

                for other in &peer_candidates {
                    if other.id != candidate_id && other.status == CandidateStatus::Rejected {
                        CandidateRepository::update_status_conn(
                            tx.as_mut(),
                            other.id,
                            CandidateStatus::Pending,
                        )
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!("failed to restore candidate status: {e}"))
                        })?;
                    }
                }
            }

            // Restore status, baseline, and outcome from changeset if present,
            // else fall back to persist_recomputed_status (backward compat).
            if let Some(old_status) = changeset.old_metadata_status {
                book.metadata_status = old_status;
            }
            if let Some(old_baseline) = changeset.old_review_baseline_metadata_status {
                book.review_baseline_metadata_status = old_baseline;
            }
            if let Some(old_outcome) = changeset.old_resolution_outcome {
                book.resolution_outcome = old_outcome;
            }
            if let Some(old_baseline_outcome) = changeset.old_review_baseline_resolution_outcome {
                book.review_baseline_resolution_outcome = old_baseline_outcome;
            }

            BookRepository::update_conn(tx.as_mut(), &book)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to update book: {e}")))?;
            self.maybe_fail_manual_action(ManualActionFailpoint::BeforeCommit)?;
            tx.commit()
                .await
                .map_err(|e| TaskError::Failed(format!("failed to commit candidate undo: {e}")))?;

            Ok(())
        }
        .await;

        tx_result?;

        // Fall back to recompute for old changesets that lack status fields.
        // When the changeset has status fields, `book` was already updated
        // and persisted inside the transaction — no reload needed.
        let book = if changeset.old_metadata_status.is_none() {
            persist_recomputed_status(&self.db_pool, book_id).await?
        } else {
            book
        };

        info!(
            book_id = %book_id,
            candidate_id = %candidate_id,
            provider = %changeset.provider_name,
            "candidate application undone"
        );

        Ok(book)
    }

    /// Add new identifiers from the provider metadata.
    #[cfg(test)]
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

    async fn add_provider_identifiers_conn(
        &self,
        conn: &mut sqlx::SqliteConnection,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
        confidence: f32,
    ) -> Result<(), TaskError> {
        let existing = IdentifierRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load identifiers: {e}")))?;

        for prov_id in &provider_meta.identifiers {
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
                IdentifierRepository::create_conn(conn, &identifier)
                    .await
                    .map_err(|e| TaskError::Failed(format!("failed to create identifier: {e}")))?;
            }
        }

        Ok(())
    }

    async fn update_authors_from_provider_conn(
        &self,
        conn: &mut sqlx::SqliteConnection,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
    ) -> Result<(), TaskError> {
        if provider_meta.authors.is_empty() {
            return Ok(());
        }

        BookRepository::clear_authors_conn(conn, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to clear authors: {e}")))?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        for (i, author) in provider_meta.authors.iter().enumerate() {
            let role = author.role.as_deref().unwrap_or("author");
            let db_author = if let Some(existing) =
                AuthorRepository::find_by_name_conn(conn, &author.name)
                    .await
                    .map_err(|e| TaskError::Failed(format!("author lookup failed: {e}")))?
            {
                existing
            } else {
                let new_author = archivis_core::models::Author::new(&author.name);
                AuthorRepository::create_conn(conn, &new_author)
                    .await
                    .map_err(|e| TaskError::Failed(format!("author create failed: {e}")))?;
                new_author
            };

            BookRepository::add_author_conn(conn, book_id, db_author.id, role, i as i32)
                .await
                .map_err(|e| TaskError::Failed(format!("add author failed: {e}")))?;
        }

        Ok(())
    }

    async fn update_series_from_provider_conn(
        &self,
        conn: &mut sqlx::SqliteConnection,
        book_id: Uuid,
        provider_meta: &ProviderMetadata,
        existing_series_link: Option<(Uuid, Option<f64>)>,
    ) -> Result<(), TaskError> {
        if let Some(ref prov_series) = provider_meta.series {
            let series = SeriesRepository::find_or_create_conn(conn, &prov_series.name)
                .await
                .map_err(|e| TaskError::Failed(format!("series find_or_create failed: {e}")))?;

            let position = prov_series.position.map(f64::from);

            if let Some((series_id, existing_position)) = existing_series_link {
                if existing_position.is_none() && position.is_some() {
                    BookRepository::update_series_position_conn(conn, book_id, series_id, position)
                        .await
                        .map_err(|e| {
                            TaskError::Failed(format!("update series position failed: {e}"))
                        })?;
                }
            } else {
                BookRepository::add_series_conn(conn, book_id, series.id, position)
                    .await
                    .map_err(|e| TaskError::Failed(format!("add series failed: {e}")))?;
            }
        }

        Ok(())
    }

    async fn fetch_cover_data(&self, cover_url: &str) -> Result<CoverData, String> {
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

        Ok(CoverData {
            bytes: cover_bytes.to_vec(),
            media_type: content_type,
        })
    }

    async fn stage_cover_for_manual_apply(
        &self,
        book_id: Uuid,
        book: &Book,
        cover_url: &str,
    ) -> Result<StagedCover, String> {
        let cover_data = self.fetch_cover_data(cover_url).await?;
        let book_dir = self.resolve_cover_directory(book_id, book).await?;
        let new_path = cover::cover_storage_path(&book_dir, &cover_data.media_type);
        let old_path = book.cover_path.clone();
        let old_bytes = match old_path.as_deref() {
            Some(path) if path == new_path => Some(
                self.storage
                    .read(path)
                    .await
                    .map_err(|e| format!("failed to back up existing cover: {e}"))?,
            ),
            _ => None,
        };

        self.storage
            .store(&new_path, &cover_data.bytes)
            .await
            .map_err(|e| format!("failed to store cover: {e}"))?;

        Ok(StagedCover {
            cover_data,
            new_path,
            old_path,
            old_bytes,
        })
    }

    async fn rollback_staged_cover(&self, book_id: Uuid, staged_cover: &StagedCover) {
        if let Some(ref old_bytes) = staged_cover.old_bytes {
            if let Err(error) = self.storage.store(&staged_cover.new_path, old_bytes).await {
                warn!(
                    book_id = %book_id,
                    cover_path = %staged_cover.new_path,
                    error = %error,
                    "failed to restore original cover after transaction rollback"
                );
            }
            return;
        }

        if let Err(error) = self.storage.delete(&staged_cover.new_path).await {
            warn!(
                book_id = %book_id,
                cover_path = %staged_cover.new_path,
                error = %error,
                "failed to remove staged cover after transaction rollback"
            );
        }
    }

    async fn finalize_staged_cover(&self, book_id: Uuid, staged_cover: StagedCover) {
        self.refresh_cover_thumbnails(book_id, &staged_cover.cover_data)
            .await;

        if let Some(ref old_path) = staged_cover.old_path {
            if old_path != &staged_cover.new_path {
                if let Err(error) = self.storage.delete(old_path).await {
                    warn!(
                        book_id = %book_id,
                        old_path = %old_path,
                        error = %error,
                        "failed to delete old cover from storage"
                    );
                }
            }
        }
    }

    async fn refresh_cover_thumbnails(&self, book_id: Uuid, cover_data: &CoverData) {
        let cache_dir = self.data_dir.join("covers").join(book_id.to_string());
        if cache_dir.exists() {
            if let Err(error) = tokio::fs::remove_dir_all(&cache_dir).await {
                warn!(
                    book_id = %book_id,
                    error = %error,
                    "failed to remove old thumbnail cache"
                );
            }
        }

        if let Err(error) =
            cover::generate_thumbnails(cover_data, book_id, &self.data_dir, &self.thumbnail_sizes)
                .await
        {
            warn!("thumbnail generation failed: {error}");
        }
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn maybe_fail_manual_action(&self, failpoint: ManualActionFailpoint) -> Result<(), TaskError> {
        #[cfg(test)]
        {
            if self
                .manual_action_failpoint
                .lock()
                .expect("manual action failpoint lock poisoned")
                .is_some_and(|configured| configured == failpoint)
            {
                return Err(TaskError::Failed(format!(
                    "manual action failpoint triggered at {failpoint:?}"
                )));
            }
        }

        let _ = failpoint;
        Ok(())
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

    async fn load_series_names(&self, book_id: Uuid) -> Result<Vec<String>, TaskError> {
        let relations = BookRepository::get_with_relations(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;

        Ok(relations
            .series
            .iter()
            .map(|series| series.series.name.clone())
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

fn identifiers_present_after_apply(
    existing: &[Identifier],
    provider_meta: &ProviderMetadata,
    should_update_identifiers: bool,
) -> bool {
    if !existing.is_empty() {
        return true;
    }

    should_update_identifiers && !provider_meta.identifiers.is_empty()
}

fn build_reconciliation_input(
    book: &Book,
    authors: &[String],
    series_names: &[String],
    identifiers: &[Identifier],
    candidate: &ProviderMetadata,
    has_strong_id_proof: bool,
) -> ReconciliationInput {
    ReconciliationInput {
        metadata_locked: book.metadata_locked,
        has_strong_id_proof,
        has_title_contradiction: candidate
            .title
            .as_deref()
            .is_some_and(|title| titles_contradict(&book.title, title)),
        title: CoreFieldInput {
            proposed: candidate.title.is_some(),
            differs: candidate
                .title
                .as_deref()
                .is_some_and(|title| !titles_equivalent(&book.title, title)),
            protected: is_protected(book.metadata_provenance.title.as_ref()),
        },
        subtitle: EnrichmentFieldInput {
            proposed: candidate.subtitle.is_some(),
            should_apply_if_unlocked: book.subtitle.is_none() && candidate.subtitle.is_some(),
            protected: is_protected(book.metadata_provenance.subtitle.as_ref()),
        },
        description: EnrichmentFieldInput {
            proposed: candidate.description.is_some(),
            should_apply_if_unlocked: book.description.is_none() && candidate.description.is_some(),
            protected: is_protected(book.metadata_provenance.description.as_ref()),
        },
        publication_year: EnrichmentFieldInput {
            proposed: candidate.publication_year.is_some(),
            should_apply_if_unlocked: book.publication_year.is_none()
                && candidate.publication_year.is_some(),
            protected: is_protected(book.metadata_provenance.publication_year.as_ref()),
        },
        language: EnrichmentFieldInput {
            proposed: candidate.language.is_some(),
            should_apply_if_unlocked: book.language.is_none() && candidate.language.is_some(),
            protected: is_protected(book.metadata_provenance.language.as_ref()),
        },
        page_count: EnrichmentFieldInput {
            proposed: candidate.page_count.is_some(),
            should_apply_if_unlocked: book.page_count.is_none() && candidate.page_count.is_some(),
            protected: is_protected(book.metadata_provenance.page_count.as_ref()),
        },
        authors: CoreFieldInput {
            proposed: !candidate.authors.is_empty(),
            differs: candidate_authors_differ(authors, candidate),
            protected: is_protected(book.metadata_provenance.authors.as_ref()),
        },
        identifiers: EnrichmentFieldInput {
            proposed: !candidate.identifiers.is_empty(),
            should_apply_if_unlocked: candidate.identifiers.iter().any(|prov_id| {
                !identifiers.iter().any(|existing| {
                    existing.identifier_type == prov_id.identifier_type
                        && existing.value == prov_id.value
                })
            }),
            protected: false,
        },
        series: CoreFieldInput {
            proposed: candidate.series.is_some(),
            differs: candidate_series_differs(series_names, candidate),
            protected: is_protected(book.metadata_provenance.series.as_ref()),
        },
        cover: EnrichmentFieldInput {
            proposed: candidate.cover_url.is_some(),
            should_apply_if_unlocked: book.cover_path.is_none() && candidate.cover_url.is_some(),
            protected: is_protected(book.metadata_provenance.cover.as_ref()),
        },
    }
}

fn resolution_query_json(query: &MetadataQuery) -> serde_json::Value {
    serde_json::json!({
        "isbn": query.isbn.clone(),
        "title": query.title.clone(),
        "author": query.author.clone(),
        "asin": query.asin.clone(),
    })
}

fn decision_code(decision_reason: &str) -> &str {
    decision_reason
        .split(':')
        .next()
        .unwrap_or("unknown")
        .trim()
}

fn provider_field_provenance(provider_name: &str) -> FieldProvenance {
    FieldProvenance {
        origin: MetadataSource::Provider(provider_name.to_string()),
        protected: false,
    }
}

fn normalized_text(value: &str) -> String {
    value.trim().to_lowercase()
}

fn normalized_author_names(values: &[String]) -> Vec<String> {
    values.iter().map(|value| normalized_text(value)).collect()
}

fn is_protected(provenance: Option<&FieldProvenance>) -> bool {
    provenance.is_some_and(|field| field.protected)
}

/// Check whether the current provenance still matches the provider that applied the field.
///
/// Returns `true` when the field is still owned by the given provider,
/// meaning it's safe to restore on undo. Returns `false` if the user
/// edited the field (provenance is `User`) or another provider overwrote it.
fn provenance_matches_provider(current: Option<&FieldProvenance>, provider_name: &str) -> bool {
    current.is_some_and(
        |fp| matches!(&fp.origin, MetadataSource::Provider(name) if name == provider_name),
    )
}

fn candidate_authors_differ(current_authors: &[String], candidate: &ProviderMetadata) -> bool {
    let candidate_authors: Vec<String> = candidate
        .authors
        .iter()
        .filter(|a| is_author_role(a.role.as_deref()))
        .map(|author| normalized_text(&author.name))
        .collect();

    if candidate_authors.is_empty() {
        return false;
    }

    // Differs only when existing authors are missing from the candidate
    // (contradiction), not when the candidate has extra authors (superset).
    let current = normalized_author_names(current_authors);
    current.iter().any(|name| !candidate_authors.contains(name))
}

fn is_author_role(role: Option<&str>) -> bool {
    matches!(role, None | Some("author"))
}

fn candidate_series_differs(current_series: &[String], candidate: &ProviderMetadata) -> bool {
    let Some(series) = candidate.series.as_ref() else {
        return false;
    };

    let candidate_series = normalized_text(&series.name);
    !current_series
        .iter()
        .map(|name| normalized_text(name))
        .any(|name| name == candidate_series)
}

fn format_reconciliation_decision(
    outcome: BookResolutionOutcome,
    best: &archivis_metadata::ScoredCandidate,
) -> String {
    let prefix = match outcome {
        BookResolutionOutcome::Confirmed => "reconciliation_confirmed",
        BookResolutionOutcome::Enriched => "reconciliation_enriched",
        BookResolutionOutcome::Disputed => "reconciliation_disputed",
        BookResolutionOutcome::Ambiguous => "blocked_ambiguous",
        BookResolutionOutcome::Unmatched => "no_candidates",
    };

    format!("{prefix}: tier={}, score={:.2}", best.tier, best.score)
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

/// Two titles are equivalent if, after normalization, one is a substring
/// of the other (covers article-stripped variants like "The X" vs "X").
fn titles_equivalent(a: &str, b: &str) -> bool {
    let a = normalized_text(a);
    let b = normalized_text(b);
    a == b || a.contains(&b) || b.contains(&a)
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
/// Generates three helper functions for the scalar fields covered by the
/// changeset: `build_scalar_changeset`, `set_scalar_provenance`, and
/// `undo_scalar_fields`.
///
/// Excluded from this macro (special logic):
/// - `title` / `sort_title` — coupled (`sort_title` only restored if title was)
/// - `publisher_id` — relational (needs DB find-or-create inside tx)
/// - `authors`, `series` — junction-table operations
/// - `identifiers` — additive merge
macro_rules! scalar_field_ops {
    ( $( $field:ident { book: $book_field:ident, prov: $prov_field:ident } ),+ $(,)? ) => {
        /// Build changeset entries for scalar fields that differ between
        /// `before` and `after`.
        #[allow(clippy::clone_on_copy)]
        fn build_scalar_changeset(
            before: &Book,
            after: &Book,
            prov: &MetadataProvenance,
        ) -> ApplyChangeset {
            let mut cs = ApplyChangeset::default();
            $(
                if after.$book_field != before.$book_field {
                    cs.$book_field = Some(ChangesetEntry {
                        old_value: before.$book_field.clone(),
                        old_provenance: prov.$prov_field.clone(),
                    });
                }
            )+
            cs
        }

        /// Set provenance for scalar fields that changed.
        fn set_scalar_provenance(
            before: &Book,
            book: &mut Book,
            provider_prov: &FieldProvenance,
        ) {
            $(
                if book.$book_field != before.$book_field {
                    book.metadata_provenance.$prov_field = Some(provider_prov.clone());
                }
            )+
        }

        /// Undo scalar fields from changeset, guarded by provenance.
        #[allow(clippy::clone_on_copy)]
        fn undo_scalar_fields(
            book: &mut Book,
            changeset: &ApplyChangeset,
            provider_name: &str,
        ) {
            $(
                if let Some(ref entry) = changeset.$book_field {
                    if provenance_matches_provider(
                        book.metadata_provenance.$prov_field.as_ref(),
                        provider_name,
                    ) {
                        book.$book_field = entry.old_value.clone();
                        book.metadata_provenance.$prov_field = entry.old_provenance.clone();
                    }
                }
            )+
        }
    };
}

scalar_field_ops! {
    subtitle         { book: subtitle,         prov: subtitle },
    description      { book: description,      prov: description },
    language         { book: language,         prov: language },
    page_count       { book: page_count,       prov: page_count },
    publication_year { book: publication_year, prov: publication_year },
    cover_path       { book: cover_path,       prov: cover },
}

/// Fields are split into two categories:
///
/// **Core identity** (title):
///   Guarded by [`FieldApplyContext::may_overwrite_core`] — auto-apply
///   requires strong ID proof, and a contradiction guard blocks
///   overwrites when the candidate title strongly conflicts with the
///   current title and the candidate lacks strong ID proof.
///
/// **Enrichment** (subtitle, description, language, `page_count`,
///   `publication_year`):
///   Fill-if-empty, always safe to apply.
///
/// Fields listed in `exclude_fields` are skipped entirely.
/// Fields marked as `protected` in `provenance` are skipped (per-field guard).
/// Provider text fields are sanitized before applying.
fn merge_book_fields(
    book: &mut Book,
    provider_meta: &ProviderMetadata,
    exclude_fields: &HashSet<String>,
    ctx: &FieldApplyContext,
    provenance: &MetadataProvenance,
) {
    let sanitize_opts = SanitizeOptions::default();
    let core_allowed = may_overwrite_core_with_log(ctx);

    let manual = !ctx.is_auto_apply;

    // ── Core identity: title ──
    // Manual apply bypasses `is_protected` — the user explicitly chose this candidate.
    if !exclude_fields.contains("title")
        && core_allowed
        && (manual || !is_protected(provenance.title.as_ref()))
    {
        if let Some(ref title) = provider_meta.title {
            if let Some(clean_title) = sanitize_text(title, &sanitize_opts) {
                book.set_title(clean_title);
            }
        }
    }

    // ── Enrichment fields ──
    // Auto-apply: fill-if-empty + skip protected.
    // Manual apply: overwrite existing + ignore protected (user chose the fields).

    // Subtitle (sanitized)
    if !exclude_fields.contains("subtitle")
        && (manual || (book.subtitle.is_none() && !is_protected(provenance.subtitle.as_ref())))
    {
        if let Some(ref subtitle) = provider_meta.subtitle {
            book.subtitle = sanitize_text(subtitle, &sanitize_opts);
        }
    }

    // Description (sanitized)
    if !exclude_fields.contains("description")
        && (manual
            || (book.description.is_none() && !is_protected(provenance.description.as_ref())))
    {
        if let Some(ref desc) = provider_meta.description {
            book.description = sanitize_text(desc, &sanitize_opts);
        }
    }

    // Language
    if !exclude_fields.contains("language")
        && (manual || (book.language.is_none() && !is_protected(provenance.language.as_ref())))
    {
        book.language.clone_from(&provider_meta.language);
    }

    // Page count
    if !exclude_fields.contains("page_count")
        && (manual || (book.page_count.is_none() && !is_protected(provenance.page_count.as_ref())))
    {
        book.page_count = provider_meta.page_count;
    }

    // Publication year
    if !exclude_fields.contains("publication_year")
        && (manual
            || (book.publication_year.is_none()
                && !is_protected(provenance.publication_year.as_ref())))
    {
        book.publication_year = provider_meta.publication_year;
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
    use archivis_core::models::{IdentifierType, ResolutionState};
    use archivis_db::{CandidateRepository, ResolutionRunRepository};
    use archivis_metadata::{MetadataProvider, ProviderAuthor, ProviderError, ProviderRegistry};
    use archivis_storage::local::LocalStorage;
    use async_trait::async_trait;
    use tokio::sync::Notify;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    fn query_same_source_prefers_higher_quality() {
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

    fn tiny_jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        buf.into_inner()
    }

    #[derive(Clone)]
    struct StubProvider {
        name: String,
        search_results: Vec<ProviderMetadata>,
    }

    static STUB_CAPS: archivis_metadata::ProviderCapabilities =
        archivis_metadata::ProviderCapabilities {
            quality: archivis_metadata::ProviderQuality::Community,
            default_rate_limit_rpm: 100,
            supported_id_lookups: &[
                IdentifierType::Isbn13,
                IdentifierType::Isbn10,
                IdentifierType::Asin,
            ],
            features: &[
                archivis_metadata::ProviderFeature::Search,
                archivis_metadata::ProviderFeature::Covers,
            ],
        };

    #[async_trait]
    impl MetadataProvider for StubProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn is_available(&self) -> bool {
            true
        }

        fn capabilities(&self) -> &'static archivis_metadata::ProviderCapabilities {
            &STUB_CAPS
        }

        async fn lookup_isbn(&self, _isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(Vec::new())
        }

        async fn search(
            &self,
            _query: &MetadataQuery,
        ) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(self.search_results.clone())
        }

        async fn fetch_cover(&self, _cover_url: &str) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }
    }

    async fn make_service(
        pool: archivis_db::DbPool,
        storage_dir: &std::path::Path,
        data_dir: &std::path::Path,
        provider_name: &str,
        search_results: Vec<ProviderMetadata>,
    ) -> ResolutionService<LocalStorage> {
        let storage = LocalStorage::new(storage_dir).await.unwrap();
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(StubProvider {
            name: provider_name.into(),
            search_results,
        }));
        let resolver = Arc::new(MetadataResolver::new(
            Arc::new(registry),
            Arc::new(NoOpSettings),
        ));

        ResolutionService::new(pool, resolver, storage, data_dir.to_path_buf())
    }

    fn search_candidate(provider_name: &str, title: &str) -> ProviderMetadata {
        ProviderMetadata {
            provider_name: provider_name.into(),
            title: Some(title.into()),
            subtitle: None,
            authors: vec![],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.9,
        }
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
            publication_year: None,
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
            physical_format: None,
            confidence: 0.95,
        }
    }

    #[tokio::test]
    async fn rerun_preserves_candidate_history_and_current_run_focus() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let data_dir = dir.path().join("data");

        let book = Book::new("Rerun History");
        BookRepository::create(&pool, &book).await.unwrap();

        let first = make_service(
            pool.clone(),
            &storage_dir,
            &data_dir,
            "provider_first",
            vec![search_candidate("provider_first", "First Result")],
        )
        .await;
        let first_outcome = first.resolve_book(book.id, false).await.unwrap();
        assert!(!first_outcome.superseded);

        let second = make_service(
            pool.clone(),
            &storage_dir,
            &data_dir,
            "provider_second",
            vec![search_candidate("provider_second", "Second Result")],
        )
        .await;
        let second_outcome = second.resolve_book(book.id, false).await.unwrap();
        assert!(!second_outcome.superseded);

        let runs = ResolutionRunRepository::list_by_book(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);

        let history = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(history.len(), 2);
        assert!(history
            .iter()
            .any(|candidate| candidate.provider_name == "provider_first"));
        assert!(history
            .iter()
            .any(|candidate| candidate.provider_name == "provider_second"));
        assert!(history.iter().any(|candidate| {
            candidate.provider_name == "provider_first"
                && candidate.status == CandidateStatus::Superseded
        }));

        let current = CandidateRepository::list_by_book(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].provider_name, "provider_second");
    }

    #[tokio::test]
    async fn duplicate_queued_resolve_tasks_noop_after_claim() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let data_dir = dir.path().join("data");

        let book = Book::new("Duplicate Queue");
        BookRepository::create(&pool, &book).await.unwrap();
        BookRepository::mark_resolution_pending(&pool, book.id, "automatic")
            .await
            .unwrap();

        let service = Arc::new(
            make_service(
                pool.clone(),
                &storage_dir,
                &data_dir,
                "provider_duplicate",
                vec![search_candidate(
                    "provider_duplicate",
                    "Duplicate Queue Result",
                )],
            )
            .await,
        );

        let reached = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        service.set_resolution_pause(
            ResolutionPausePoint::BeforeFinalizeSupersessionCheck,
            Arc::clone(&reached),
            Arc::clone(&release),
        );

        let service_for_first = Arc::clone(&service);
        let first = tokio::spawn(async move {
            service_for_first
                .resolve_queued_book(book.id, false, &[])
                .await
                .unwrap()
        });

        reached.notified().await;
        let second = service
            .resolve_queued_book(book.id, false, &[])
            .await
            .unwrap();
        assert!(second.is_none(), "duplicate task should no-op after claim");

        release.notify_waiters();
        let first_outcome = first.await.unwrap().unwrap();
        assert!(!first_outcome.superseded);

        let reloaded = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert_eq!(reloaded.resolution_state, ResolutionState::Done);
    }

    #[tokio::test]
    async fn user_edit_during_finalize_supersedes_stale_run() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let data_dir = dir.path().join("data");

        let book = Book::new("Superseded During Finalize");
        BookRepository::create(&pool, &book).await.unwrap();
        BookRepository::mark_resolution_pending(&pool, book.id, "automatic")
            .await
            .unwrap();

        let service = Arc::new(
            make_service(
                pool.clone(),
                &storage_dir,
                &data_dir,
                "provider_finalize",
                vec![search_candidate(
                    "provider_finalize",
                    "Superseded During Finalize",
                )],
            )
            .await,
        );

        let reached = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        service.set_resolution_pause(
            ResolutionPausePoint::BeforeFinalizeSupersessionCheck,
            Arc::clone(&reached),
            Arc::clone(&release),
        );

        let service_for_run = Arc::clone(&service);
        let run = tokio::spawn(async move {
            service_for_run
                .resolve_queued_book(book.id, false, &[])
                .await
                .unwrap()
                .unwrap()
        });

        reached.notified().await;
        BookRepository::mark_resolution_pending(&pool, book.id, "user_edit")
            .await
            .unwrap();
        release.notify_waiters();

        let outcome = run.await.unwrap();
        assert!(outcome.superseded);

        let recovered_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert_eq!(recovered_book.resolution_state, ResolutionState::Pending);
        assert_eq!(
            recovered_book.resolution_requested_reason.as_deref(),
            Some("user_edit")
        );

        let recovered_run = ResolutionRunRepository::get_by_id(&pool, outcome.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered_run.state, ResolutionRunState::Superseded);

        let history = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        assert!(history
            .iter()
            .any(|candidate| candidate.status == CandidateStatus::Superseded));
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

        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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

    // ── titles_equivalent tests ──

    #[test]
    fn titles_equivalent_article_stripped() {
        assert!(titles_equivalent(
            "The Lady of the Lake",
            "Lady of the Lake"
        ));
        assert!(titles_equivalent(
            "Lady of the Lake",
            "The Lady of the Lake"
        ));
    }

    #[test]
    fn titles_equivalent_identical() {
        assert!(titles_equivalent("Dune", "Dune"));
    }

    #[test]
    fn titles_equivalent_genuinely_different() {
        assert!(!titles_equivalent(
            "Harry Potter and the Philosopher's Stone",
            "Harry Potter and the Sorcerer's Stone"
        ));
    }

    #[test]
    fn titles_equivalent_completely_different() {
        assert!(!titles_equivalent("Dune", "Foundation"));
    }

    // ── candidate_authors_differ tests ──

    #[test]
    fn candidate_authors_differ_excludes_translators() {
        let current = vec!["Andrzej Sapkowski".to_string()];
        let candidate = ProviderMetadata {
            provider_name: "test".into(),
            title: None,
            subtitle: None,
            authors: vec![
                ProviderAuthor {
                    name: "Andrzej Sapkowski".into(),
                    role: Some("author".into()),
                },
                ProviderAuthor {
                    name: "David A. French".into(),
                    role: Some("translator".into()),
                },
            ],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        };
        assert!(
            !candidate_authors_differ(&current, &candidate),
            "translator should be excluded from author comparison"
        );
    }

    #[test]
    fn candidate_authors_differ_detects_real_difference() {
        let current = vec!["Frank Herbert".to_string()];
        let candidate = ProviderMetadata {
            provider_name: "test".into(),
            title: None,
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Isaac Asimov".into(),
                role: Some("author".into()),
            }],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        };
        assert!(candidate_authors_differ(&current, &candidate));
    }

    #[test]
    fn candidate_authors_differ_superset_is_not_different() {
        let current = vec!["Andrzej Sapkowski".to_string()];
        let candidate = ProviderMetadata {
            provider_name: "test".into(),
            title: None,
            subtitle: None,
            authors: vec![
                ProviderAuthor {
                    name: "Andrzej Sapkowski".into(),
                    role: Some("author".into()),
                },
                ProviderAuthor {
                    name: "David A. French".into(),
                    role: Some("author".into()),
                },
            ],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        };
        assert!(
            !candidate_authors_differ(&current, &candidate),
            "superset (candidate adds authors) should not count as a conflict"
        );
    }

    #[test]
    fn candidate_authors_differ_missing_existing_is_different() {
        let current = vec!["Andrzej Sapkowski".to_string()];
        let candidate = ProviderMetadata {
            provider_name: "test".into(),
            title: None,
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "David A. French".into(),
                role: Some("author".into()),
            }],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        };
        assert!(
            candidate_authors_differ(&current, &candidate),
            "existing author missing from candidate should be a conflict"
        );
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
            publication_year: Some(2020),
            identifiers: vec![],
            subjects: Vec::new(),
            series: None,
            page_count: Some(300),
            cover_url: None,
            rating: None,
            physical_format: None,
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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &exclude,
            &ctx,
            &MetadataProvenance::default(),
        );

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

        merge_book_fields(
            &mut book,
            &provider,
            &HashSet::new(),
            &ctx,
            &MetadataProvenance::default(),
        );

        // Core identity blocked, but all enrichment fields applied.
        assert_eq!(book.title, "Some Title");
        assert_eq!(book.subtitle.as_deref(), Some("A Subtitle"));
        assert_eq!(book.description.as_deref(), Some("A description"));
        assert_eq!(book.language.as_deref(), Some("en"));
        assert_eq!(book.page_count, Some(300));
        assert!(book.publication_year.is_some());
    }

    #[test]
    fn merge_book_fields_manual_overwrites_existing() {
        // Manual apply should overwrite filled + protected fields.
        let mut book = Book::new("Original Title");
        book.subtitle = Some("Old Subtitle".to_string());
        book.description = Some("Old description".to_string());
        book.language = Some("de".to_string());
        book.page_count = Some(100);
        book.publication_year = Some(2010);

        let provider = make_provider_meta("New Title");

        let ctx = FieldApplyContext {
            is_auto_apply: false,
            has_strong_id_proof: false,
            has_title_contradiction: false,
        };

        // Mark all fields as protected.
        let provenance = MetadataProvenance {
            title: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            subtitle: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            description: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            language: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            page_count: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            publication_year: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            ..Default::default()
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx, &provenance);

        // Manual apply must overwrite everything despite protection.
        assert_eq!(book.title, "New Title");
        assert_eq!(book.subtitle.as_deref(), Some("A Subtitle"));
        assert_eq!(book.description.as_deref(), Some("A description"));
        assert_eq!(book.language.as_deref(), Some("en"));
        assert_eq!(book.page_count, Some(300));
        assert_eq!(book.publication_year, Some(2020));
    }

    #[test]
    fn auto_apply_respects_protection_and_fill_if_empty() {
        // Auto-apply should NOT overwrite filled or protected fields.
        let mut book = Book::new("Original Title");
        book.subtitle = Some("Old Subtitle".to_string());
        book.description = Some("Old description".to_string());
        book.language = Some("de".to_string());
        book.page_count = Some(100);
        book.publication_year = Some(2010);

        let provider = make_provider_meta("New Title");

        let ctx = FieldApplyContext {
            is_auto_apply: true,
            has_strong_id_proof: true,
            has_title_contradiction: false,
        };

        let provenance = MetadataProvenance {
            title: Some(FieldProvenance {
                origin: MetadataSource::Embedded,
                protected: true,
            }),
            ..Default::default()
        };

        merge_book_fields(&mut book, &provider, &HashSet::new(), &ctx, &provenance);

        // Protected title must NOT be overwritten even with proof.
        assert_eq!(book.title, "Original Title");
        // Fill-if-empty: fields already have values → not overwritten.
        assert_eq!(book.subtitle.as_deref(), Some("Old Subtitle"));
        assert_eq!(book.description.as_deref(), Some("Old description"));
        assert_eq!(book.language.as_deref(), Some("de"));
        assert_eq!(book.page_count, Some(100));
        assert_eq!(book.publication_year, Some(2010));
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
        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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
        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: Some(archivis_metadata::types::ProviderSeries {
                name: "Dune Chronicles".into(),
                position: Some(1.0),
            }),
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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
        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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
            publication_year: None,
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
            physical_format: None,
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
        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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
    async fn protected_authors_blocks_author_update() {
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
        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

        // Book with protected authors provenance.
        let mut book = Book::new("Dune");
        book.metadata_provenance.authors = Some(FieldProvenance {
            origin: MetadataSource::User,
            protected: true,
        });
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
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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

        // Manual apply bypasses protection — the user explicitly chose the candidate.
        let relations = BookRepository::get_with_relations(&pool, book.id)
            .await
            .unwrap();
        let names: Vec<&str> = relations
            .authors
            .iter()
            .map(|a| a.author.name.as_str())
            .collect();
        assert!(
            names.contains(&"Brian Herbert"),
            "manual apply should overwrite protected authors; got: {names:?}"
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
        let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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

    #[tokio::test]
    async fn resolve_cover_manual_apply_rolls_back_cover_and_db_state_on_commit_failure() {
        let (pool, dir) = test_pool().await;
        let storage_dir = dir.path().join("storage");
        let data_dir = dir.path().join("data");
        let service = make_service(
            pool.clone(),
            &storage_dir,
            &data_dir,
            "test_provider",
            vec![],
        )
        .await;
        let storage = LocalStorage::new(&storage_dir).await.unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(tiny_jpeg())
                    .insert_header("content-type", "image/jpeg"),
            )
            .mount(&mock_server)
            .await;

        let mut old_image = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(1, 1, image::Rgb([0, 0, 255]))
            .write_to(&mut old_image, image::ImageFormat::Jpeg)
            .unwrap();
        let old_cover_bytes = old_image.into_inner();

        let mut book = Book::new("Original Title");
        book.cover_path = Some("D/David Allen/Ready for Anything/cover.jpg".into());
        BookRepository::create(&pool, &book).await.unwrap();

        let book_file = archivis_core::models::BookFile::new(
            book.id,
            archivis_core::models::BookFormat::Epub,
            "D/David Allen/Ready for Anything/book.epub",
            1000,
            "abcdef12".repeat(8),
            None,
        );
        archivis_db::BookFileRepository::create(&pool, &book_file)
            .await
            .unwrap();
        storage
            .store(book.cover_path.as_deref().unwrap(), &old_cover_bytes)
            .await
            .unwrap();

        let provider_meta = ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Updated Title".into()),
            subtitle: None,
            authors: vec![],
            description: Some("Updated description".into()),
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: Some(format!("{}/cover.jpg", mock_server.uri())),
            rating: None,
            physical_format: None,
            confidence: 0.93,
        };
        let mut candidate = IdentificationCandidate::new(
            book.id,
            "test_provider",
            0.93,
            serde_json::to_value(&provider_meta).unwrap(),
            vec!["title_match".into()],
        );
        candidate.disputes = vec!["title_conflict".into()];
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        service.set_manual_action_failpoint(ManualActionFailpoint::BeforeCommit);

        let error = service
            .apply_candidate(book.id, candidate.id, &HashSet::new())
            .await
            .expect_err("manual apply should fail at the injected failpoint");
        assert!(
            error.to_string().contains("BeforeCommit"),
            "unexpected error: {error}"
        );

        let reloaded = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert_eq!(reloaded.title, "Original Title");
        assert_eq!(
            reloaded.cover_path.as_deref(),
            Some("D/David Allen/Ready for Anything/cover.jpg")
        );

        let reloaded_candidate = CandidateRepository::get_by_id(&pool, candidate.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded_candidate.status, CandidateStatus::Pending);

        let stored_cover = storage
            .read("D/David Allen/Ready for Anything/cover.jpg")
            .await
            .unwrap();
        assert_eq!(stored_cover, old_cover_bytes);
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
