use archivis_core::errors::TaskError;
use archivis_core::models::{Book, CandidateStatus, MetadataStatus};
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

pub async fn persist_recomputed_status(pool: &DbPool, book_id: Uuid) -> Result<Book, TaskError> {
    let mut relations = BookRepository::get_with_relations(pool, book_id)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to load book relations: {e}")))?;
    let candidates = CandidateRepository::list_by_book(pool, book_id)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to load candidates: {e}")))?;

    let snapshot = BookSnapshot {
        has_authors: !relations.authors.is_empty(),
        has_identifiers: !relations.identifiers.is_empty(),
        has_applied_candidate: candidates
            .iter()
            .any(|candidate| candidate.status == CandidateStatus::Applied),
    };
    let ctx = StatusContext {
        has_ambiguous_candidates: candidates
            .iter()
            .any(|candidate| candidate.status == CandidateStatus::Pending),
        has_disputed_candidates: candidates.iter().any(|candidate| {
            candidate.status == CandidateStatus::Pending && !candidate.disputes.is_empty()
        }),
    };

    let status = recompute_status(&snapshot, &ctx);
    if relations.book.metadata_status != status {
        relations.book.metadata_status = status;
        BookRepository::update(pool, &relations.book)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to update book status: {e}")))?;
    }

    Ok(relations.book)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
