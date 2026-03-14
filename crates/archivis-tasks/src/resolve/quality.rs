//! Live metadata quality score computation.
//!
//! Evaluates the current canonical state of a book record and produces
//! a quality score reflecting "how complete is this book right now?".

use archivis_core::errors::TaskError;
use archivis_core::scoring::{
    compute_quality_score, is_garbage_author, is_garbage_title, is_valid_identifier_by_type,
    QualitySignals, BALANCED_WEIGHTS,
};
use archivis_db::{BookRepository, BookWithRelations, DbPool};
use tracing::debug;
use uuid::Uuid;

/// Number of richness fields checked for the live score.
///
/// Fields: description, language, publisher, `publication_year`, `page_count`,
/// series (at least one), cover.
const LIVE_RICHNESS_TOTAL: u8 = 7;

/// Extract quality signals from a fully-loaded book record.
fn extract_live_signals(bwr: &BookWithRelations) -> QualitySignals {
    let has_strong_identifier = bwr
        .identifiers
        .iter()
        .any(|id| is_valid_identifier_by_type(id.identifier_type, &id.value));

    let has_title = !is_garbage_title(&bwr.book.title);

    let has_author = bwr
        .authors
        .iter()
        .any(|a| !is_garbage_author(&a.author.name));

    let mut richness_present: u8 = 0;
    if bwr
        .book
        .description
        .as_deref()
        .is_some_and(|d| !d.is_empty())
    {
        richness_present += 1;
    }
    if bwr.book.language.is_some() {
        richness_present += 1;
    }
    if bwr.book.publisher_id.is_some() {
        richness_present += 1;
    }
    if bwr.book.publication_year.is_some() {
        richness_present += 1;
    }
    if bwr.book.page_count.is_some() {
        richness_present += 1;
    }
    if !bwr.series.is_empty() {
        richness_present += 1;
    }
    if bwr.book.cover_path.is_some() {
        richness_present += 1;
    }

    QualitySignals {
        has_title,
        has_author,
        has_strong_identifier,
        richness_present,
        richness_total: LIVE_RICHNESS_TOTAL,
        context_bonus: 0.0,
    }
}

/// Compute and persist the live quality score from a pre-loaded `BookWithRelations`.
///
/// Use this when you already have the BWR loaded (e.g. for building an API
/// response) to avoid a redundant `get_with_relations` round-trip.
pub async fn compute_and_persist_quality_score(
    pool: &DbPool,
    bwr: &BookWithRelations,
) -> Result<f32, TaskError> {
    let signals = extract_live_signals(bwr);
    let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);

    debug!(
        book_id = %bwr.book.id,
        score = score,
        has_isbn = signals.has_strong_identifier,
        has_title = signals.has_title,
        has_author = signals.has_author,
        richness = format!("{}/{}", signals.richness_present, signals.richness_total),
        "computed live metadata quality score"
    );

    BookRepository::update_metadata_quality_score(pool, bwr.book.id, score)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to update quality score: {e}")))?;

    Ok(score)
}

/// Recompute and persist the live metadata quality score for a single book.
///
/// Loads the full `BookWithRelations` internally. When you already have the
/// BWR loaded, prefer [`compute_and_persist_quality_score`] instead.
pub async fn refresh_metadata_quality_score(
    pool: &DbPool,
    book_id: Uuid,
) -> Result<f32, TaskError> {
    let bwr = BookRepository::get_with_relations(pool, book_id)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to load book for quality score: {e}")))?;
    compute_and_persist_quality_score(pool, &bwr).await
}

/// Best-effort refresh: loads the book, computes and persists the live
/// quality score, logging a warning on failure.
pub async fn refresh_quality_score_best_effort(pool: &DbPool, book_id: Uuid) {
    if let Err(e) = refresh_metadata_quality_score(pool, book_id).await {
        tracing::warn!(book_id = %book_id, error = %e, "metadata quality score refresh failed");
    }
}

/// Backfill `metadata_quality_score` for all books that have NULL.
///
/// Processes in batches of 100 until none remain. Returns the total
/// number of books updated.
pub async fn backfill_metadata_quality_scores(pool: &DbPool) -> Result<u32, TaskError> {
    let mut total = 0u32;

    loop {
        let ids = BookRepository::list_ids_without_quality_score(pool, 100)
            .await
            .map_err(|e| {
                TaskError::Failed(format!("failed to list books for quality backfill: {e}"))
            })?;

        if ids.is_empty() {
            break;
        }

        for id in &ids {
            if let Err(e) = refresh_metadata_quality_score(pool, *id).await {
                tracing::warn!(book_id = %id, error = %e, "backfill quality score failed for book");
            } else {
                total += 1;
            }
        }
    }

    if total > 0 {
        tracing::info!(count = total, "backfilled metadata quality scores");
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_signals_full_book() {
        use archivis_core::models::*;
        use archivis_db::{BookAuthorEntry, BookSeriesEntry};

        let mut book = Book::new("Dune");
        book.description = Some("A science fiction masterpiece.".into());
        book.language = Some("en".into());
        book.publisher_id = Some(uuid::Uuid::new_v4());
        book.publication_year = Some(1965);
        book.page_count = Some(412);
        book.cover_path = Some("covers/dune.jpg".into());

        let bwr = BookWithRelations {
            book,
            authors: vec![BookAuthorEntry {
                author: Author::new("Frank Herbert"),
                role: "author".into(),
                position: 0,
            }],
            series: vec![BookSeriesEntry {
                series: Series {
                    id: uuid::Uuid::new_v4(),
                    name: "Dune".into(),
                    description: None,
                },
                position: Some(1.0),
            }],
            files: vec![],
            identifiers: vec![Identifier::new(
                uuid::Uuid::new_v4(),
                IdentifierType::Isbn13,
                "9783161484100",
                MetadataSource::Embedded,
                0.9,
            )],
            tags: vec![],
            publisher_name: Some("Chilton Books".into()),
        };

        let signals = extract_live_signals(&bwr);
        assert!(signals.has_title);
        assert!(signals.has_author);
        assert!(signals.has_strong_identifier);
        assert_eq!(signals.richness_present, 7);
        assert_eq!(signals.richness_total, 7);

        let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
        // 0.4 + 0.3 + 0.30 = 1.0
        assert!((score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn extract_signals_minimal_book() {
        use archivis_core::models::*;

        let book = Book::new("Unknown");

        let bwr = BookWithRelations {
            book,
            authors: vec![],
            series: vec![],
            files: vec![],
            identifiers: vec![],
            tags: vec![],
            publisher_name: None,
        };

        let signals = extract_live_signals(&bwr);
        assert!(!signals.has_title); // "Unknown" is garbage
        assert!(!signals.has_author);
        assert!(!signals.has_strong_identifier);
        assert_eq!(signals.richness_present, 0);

        let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
        assert!(score.abs() < f32::EPSILON);
    }
}
