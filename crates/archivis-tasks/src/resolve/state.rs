use archivis_core::errors::TaskError;
use archivis_core::models::{Book, CandidateStatus, IdentificationCandidate, MetadataStatus};
use archivis_db::{BookRepository, CandidateRepository, DbPool};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BookSnapshot {
    pub has_authors: bool,
    pub has_identifiers: bool,
    pub has_applied_candidate: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusContext {
    pub has_ambiguous_candidates: bool,
    pub has_disputed_candidates: bool,
}

impl StatusContext {
    /// Build from a candidate list in a single pass, also returning whether
    /// any candidate has been applied.
    pub fn from_candidates(candidates: &[IdentificationCandidate]) -> (Self, bool) {
        let mut has_applied = false;
        let mut has_ambiguous = false;
        let mut has_disputed = false;
        for c in candidates {
            match c.status {
                CandidateStatus::Applied => has_applied = true,
                CandidateStatus::Pending => {
                    has_ambiguous = true;
                    if !c.disputes.is_empty() {
                        has_disputed = true;
                    }
                }
                _ => {}
            }
        }
        (
            Self {
                has_ambiguous_candidates: has_ambiguous,
                has_disputed_candidates: has_disputed,
            },
            has_applied,
        )
    }
}

pub fn recompute_status(book: &BookSnapshot, ctx: &StatusContext) -> MetadataStatus {
    if ctx.has_ambiguous_candidates || ctx.has_disputed_candidates {
        return MetadataStatus::NeedsReview;
    }

    if book.has_applied_candidate || (book.has_authors && book.has_identifiers) {
        return MetadataStatus::Identified;
    }

    if book.has_authors || book.has_identifiers {
        return MetadataStatus::NeedsReview;
    }

    MetadataStatus::Unidentified
}

/// Apply the review baseline as a floor: when no candidates are pending,
/// the status can never be worse than the baseline.
pub fn apply_review_floor(
    derived: MetadataStatus,
    ctx: &StatusContext,
    baseline: Option<MetadataStatus>,
) -> MetadataStatus {
    if ctx.has_ambiguous_candidates || ctx.has_disputed_candidates {
        return derived; // still in review — floor doesn't apply
    }
    baseline.map_or(derived, |floor| derived.at_least(floor))
}

/// Compute the effective status from `snapshot` + `ctx`, apply the
/// review-baseline floor, then clear the baseline when no pending
/// candidates remain.
pub fn update_status_with_floor(book: &mut Book, snapshot: &BookSnapshot, ctx: &StatusContext) {
    let derived = recompute_status(snapshot, ctx);
    book.metadata_status = apply_review_floor(derived, ctx, book.review_baseline_metadata_status);
    if !ctx.has_ambiguous_candidates && !ctx.has_disputed_candidates {
        if let Some(baseline_status) = book.review_baseline_metadata_status.take() {
            // Review ended without apply — restore exact pre-review state
            book.metadata_status = baseline_status;
            book.resolution_outcome = book.review_baseline_resolution_outcome.take();
        }
    }
}

pub async fn persist_recomputed_status(pool: &DbPool, book_id: Uuid) -> Result<Book, TaskError> {
    let mut relations = BookRepository::get_with_relations(pool, book_id)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;
    let candidates = CandidateRepository::list_by_book(pool, book_id)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to load candidates: {e}")))?;

    let (ctx, has_applied) = StatusContext::from_candidates(&candidates);
    let snapshot = BookSnapshot {
        has_authors: !relations.authors.is_empty(),
        has_identifiers: !relations.identifiers.is_empty(),
        has_applied_candidate: has_applied,
    };

    let old_status = relations.book.metadata_status;
    let old_outcome = relations.book.resolution_outcome;
    let had_baseline = relations.book.review_baseline_metadata_status.is_some();
    update_status_with_floor(&mut relations.book, &snapshot, &ctx);

    if relations.book.metadata_status != old_status
        || relations.book.resolution_outcome != old_outcome
        || (had_baseline && relations.book.review_baseline_metadata_status.is_none())
    {
        BookRepository::update(pool, &relations.book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update book status: {e}")))?;
    }

    Ok(relations.book)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archivis_core::models::ResolutionOutcome;

    #[test]
    fn recompute_status_returns_needs_review_for_open_review_blockers() {
        let snapshot = BookSnapshot {
            has_authors: true,
            has_identifiers: true,
            has_applied_candidate: true,
        };
        let ctx = StatusContext {
            has_ambiguous_candidates: true,
            has_disputed_candidates: false,
        };

        assert_eq!(
            recompute_status(&snapshot, &ctx),
            MetadataStatus::NeedsReview
        );
    }

    #[test]
    fn recompute_status_returns_identified_for_accepted_identity() {
        let snapshot = BookSnapshot {
            has_authors: false,
            has_identifiers: false,
            has_applied_candidate: true,
        };

        assert_eq!(
            recompute_status(&snapshot, &StatusContext::default()),
            MetadataStatus::Identified
        );
    }

    #[test]
    fn recompute_status_returns_needs_review_for_partial_identity() {
        let snapshot = BookSnapshot {
            has_authors: true,
            has_identifiers: false,
            has_applied_candidate: false,
        };

        assert_eq!(
            recompute_status(&snapshot, &StatusContext::default()),
            MetadataStatus::NeedsReview
        );
    }

    #[test]
    fn recompute_status_returns_unidentified_for_empty_snapshot() {
        assert_eq!(
            recompute_status(&BookSnapshot::default(), &StatusContext::default()),
            MetadataStatus::Unidentified
        );
    }

    // ── apply_review_floor ─────────────────────────────────────────

    #[test]
    fn apply_review_floor_prevents_downgrade_when_no_pending() {
        let ctx = StatusContext::default(); // no pending candidates
        let result = apply_review_floor(
            MetadataStatus::NeedsReview,
            &ctx,
            Some(MetadataStatus::Identified),
        );
        assert_eq!(result, MetadataStatus::Identified);
    }

    #[test]
    fn apply_review_floor_ignored_when_pending_candidates_exist() {
        let ctx = StatusContext {
            has_ambiguous_candidates: true,
            has_disputed_candidates: false,
        };
        let result = apply_review_floor(
            MetadataStatus::NeedsReview,
            &ctx,
            Some(MetadataStatus::Identified),
        );
        assert_eq!(result, MetadataStatus::NeedsReview);
    }

    #[test]
    fn apply_review_floor_none_baseline_preserves_derived() {
        let ctx = StatusContext::default();
        let result = apply_review_floor(MetadataStatus::NeedsReview, &ctx, None);
        assert_eq!(result, MetadataStatus::NeedsReview);
    }

    #[test]
    fn apply_review_floor_does_not_downgrade_when_derived_is_higher() {
        let ctx = StatusContext::default();
        let result = apply_review_floor(
            MetadataStatus::Identified,
            &ctx,
            Some(MetadataStatus::NeedsReview),
        );
        assert_eq!(result, MetadataStatus::Identified);
    }

    // ── update_status_with_floor outcome restoration ─────────────

    #[test]
    fn update_status_with_floor_restores_confirmed_outcome_on_review_end() {
        let mut book = Book::new("Test");
        book.metadata_status = MetadataStatus::NeedsReview;
        book.review_baseline_metadata_status = Some(MetadataStatus::Identified);
        book.review_baseline_resolution_outcome = Some(ResolutionOutcome::Confirmed);
        book.resolution_outcome = Some(ResolutionOutcome::Ambiguous);

        let snapshot = BookSnapshot {
            has_authors: true,
            has_identifiers: true,
            has_applied_candidate: true,
        };
        let ctx = StatusContext::default(); // no pending candidates

        update_status_with_floor(&mut book, &snapshot, &ctx);

        assert_eq!(book.metadata_status, MetadataStatus::Identified);
        assert_eq!(book.resolution_outcome, Some(ResolutionOutcome::Confirmed));
        assert!(book.review_baseline_metadata_status.is_none());
        assert!(book.review_baseline_resolution_outcome.is_none());
    }

    #[test]
    fn update_status_with_floor_restores_needs_review_status_on_review_end() {
        // Bug 1 regression: baseline `NeedsReview` must not be promoted to
        // `Identified` just because `recompute_status` returns `Identified`.
        let mut book = Book::new("Test");
        book.metadata_status = MetadataStatus::NeedsReview;
        book.review_baseline_metadata_status = Some(MetadataStatus::NeedsReview);
        book.review_baseline_resolution_outcome = None;
        book.resolution_outcome = Some(ResolutionOutcome::Ambiguous);

        // Snapshot where `recompute_status` would return `Identified`
        let snapshot = BookSnapshot {
            has_authors: true,
            has_identifiers: true,
            has_applied_candidate: false,
        };
        let ctx = StatusContext::default(); // no pending candidates — review ended

        update_status_with_floor(&mut book, &snapshot, &ctx);

        assert_eq!(
            book.metadata_status,
            MetadataStatus::NeedsReview,
            "must restore baseline NeedsReview, not promote to Identified"
        );
        assert!(
            book.resolution_outcome.is_none(),
            "must restore baseline None outcome"
        );
        assert!(book.review_baseline_metadata_status.is_none());
        assert!(book.review_baseline_resolution_outcome.is_none());
    }

    #[test]
    fn update_status_with_floor_restores_enriched_outcome_on_review_end() {
        let mut book = Book::new("Test");
        book.metadata_status = MetadataStatus::NeedsReview;
        book.review_baseline_metadata_status = Some(MetadataStatus::Identified);
        book.review_baseline_resolution_outcome = Some(ResolutionOutcome::Enriched);
        book.resolution_outcome = Some(ResolutionOutcome::Ambiguous);

        let snapshot = BookSnapshot {
            has_authors: true,
            has_identifiers: true,
            has_applied_candidate: true,
        };
        let ctx = StatusContext::default();

        update_status_with_floor(&mut book, &snapshot, &ctx);

        assert_eq!(book.metadata_status, MetadataStatus::Identified);
        assert_eq!(book.resolution_outcome, Some(ResolutionOutcome::Enriched));
        assert!(book.review_baseline_metadata_status.is_none());
        assert!(book.review_baseline_resolution_outcome.is_none());
    }

    #[test]
    fn update_status_with_floor_preserves_outcome_during_active_review() {
        let mut book = Book::new("Test");
        book.metadata_status = MetadataStatus::Identified;
        book.review_baseline_metadata_status = Some(MetadataStatus::Identified);
        book.review_baseline_resolution_outcome = Some(ResolutionOutcome::Confirmed);
        book.resolution_outcome = Some(ResolutionOutcome::Ambiguous);

        let snapshot = BookSnapshot {
            has_authors: true,
            has_identifiers: true,
            has_applied_candidate: true,
        };
        let ctx = StatusContext {
            has_ambiguous_candidates: true,
            has_disputed_candidates: false,
        };

        update_status_with_floor(&mut book, &snapshot, &ctx);

        // Still in review — outcome unchanged, baselines still set
        assert_eq!(book.resolution_outcome, Some(ResolutionOutcome::Ambiguous));
        assert_eq!(
            book.review_baseline_metadata_status,
            Some(MetadataStatus::Identified)
        );
        assert_eq!(
            book.review_baseline_resolution_outcome,
            Some(ResolutionOutcome::Confirmed)
        );
    }

    #[test]
    fn update_status_with_floor_restores_none_outcome_when_original_was_none() {
        let mut book = Book::new("Test");
        book.metadata_status = MetadataStatus::NeedsReview;
        book.review_baseline_metadata_status = Some(MetadataStatus::Unidentified);
        book.review_baseline_resolution_outcome = None;
        book.resolution_outcome = Some(ResolutionOutcome::Ambiguous);

        let snapshot = BookSnapshot::default();
        let ctx = StatusContext::default();

        update_status_with_floor(&mut book, &snapshot, &ctx);

        assert!(book.resolution_outcome.is_none());
        assert!(book.review_baseline_metadata_status.is_none());
        assert!(book.review_baseline_resolution_outcome.is_none());
    }
}
