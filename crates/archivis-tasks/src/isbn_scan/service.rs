//! ISBN content-scan service.
//!
//! Extracts ISBNs from the text content of ebook files and stores them as
//! `Identifier` records with `MetadataSource::ContentScan`.
//!
//! ## Scan evidence semantics
//!
//! - **Confidence weighting**: scanned ISBNs receive a configurable base
//!   confidence (default 0.5), well below embedded metadata (1.0). When
//!   multiple distinct ISBNs are found in the same book a per-ISBN noise
//!   discount is applied (multi-ISBN penalty).
//! - **Capped storage**: at most [`MAX_SCAN_ISBNS`] ISBNs are stored per
//!   book to prevent bibliography/footnote noise from flooding identifiers.
//! - **Trust boundary**: when multiple scan ISBNs are found, `ContentScan`
//!   identifiers are excluded from the resolver's `trusted_identifiers` set
//!   (bibliography noise guard). However, a **single** scan ISBN is promoted
//!   to trusted — it is almost certainly the book's own ISBN (copyright page
//!   or spine), not noise.

use archivis_core::errors::TaskError;
use archivis_core::models::{Identifier, IdentifierType, MetadataSource};
use archivis_db::{BookFileRepository, DbPool, IdentifierRepository};
use archivis_formats::content_text::ContentScanConfig;
use archivis_formats::isbn_scan::{scan_text_for_isbns, ScannedIsbn};
use archivis_storage::StorageBackend;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Maximum ISBNs stored per book from content scanning.
///
/// Bibliographies, footnotes, and reference sections can produce many ISBNs
/// that belong to *other* books.  Capping prevents noise from flooding the
/// identifier table while preserving the most useful results (document-order
/// priority: ISBN-13 first, then ISBN-10, earlier in text wins).
const MAX_SCAN_ISBNS: usize = 5;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Task-level configuration for the ISBN content-scan pipeline.
#[derive(Debug, Clone)]
pub struct IsbnScanConfig {
    /// Confidence value assigned to ISBNs found via content scanning (0.0-1.0).
    pub confidence: f32,
    /// Skip scanning if any existing ISBN has confidence >= this threshold.
    pub skip_threshold: f32,
    /// Passed through to per-format text extractors.
    pub text_config: ContentScanConfig,
}

impl Default for IsbnScanConfig {
    fn default() -> Self {
        Self {
            confidence: 0.5,
            skip_threshold: 0.95,
            text_config: ContentScanConfig::default(),
        }
    }
}

impl IsbnScanConfig {
    /// Build from individual app-config values without requiring the caller
    /// to depend on `archivis_formats` directly.
    pub fn from_app_config(
        confidence: f32,
        skip_threshold: f32,
        epub_spine_items: usize,
        pdf_pages: usize,
        fb2_sections: usize,
        txt_bytes: usize,
        mobi_bytes: usize,
    ) -> Self {
        Self {
            confidence,
            skip_threshold,
            text_config: ContentScanConfig {
                epub_spine_items,
                pdf_pages,
                fb2_sections,
                txt_bytes,
                mobi_bytes,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// Summary returned after scanning a single book.
#[derive(Debug, Clone)]
pub struct IsbnScanResult {
    pub book_id: Uuid,
    /// Total unique ISBNs found across all files.
    pub isbns_found: usize,
    /// How many of those were actually stored as new `Identifier` records.
    pub isbns_stored: usize,
    /// Number of files that were read and scanned.
    pub files_scanned: usize,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Scans book file content for ISBNs and stores them as `Identifier` records.
pub struct IsbnScanService<S: StorageBackend> {
    db_pool: DbPool,
    storage: S,
    config: IsbnScanConfig,
}

impl<S: StorageBackend> IsbnScanService<S> {
    pub fn new(db_pool: DbPool, storage: S, config: IsbnScanConfig) -> Self {
        Self {
            db_pool,
            storage,
            config,
        }
    }

    /// Scan all files belonging to `book_id` for ISBNs and store new ones.
    ///
    /// Skips scanning entirely when the book already has at least one ISBN
    /// identifier with confidence >= `skip_threshold`.
    #[allow(clippy::too_many_lines)] // linear sequence of steps, splitting would hurt readability
    pub async fn scan_book(&self, book_id: Uuid) -> Result<IsbnScanResult, TaskError> {
        // 1. Check existing identifiers — skip if any ISBN has high confidence.
        let existing = IdentifierRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load identifiers: {e}")))?;

        let has_high_confidence_isbn = existing.iter().any(|id| {
            (id.identifier_type == IdentifierType::Isbn13
                || id.identifier_type == IdentifierType::Isbn10)
                && id.confidence >= self.config.skip_threshold
        });

        if has_high_confidence_isbn {
            debug!(
                book_id = %book_id,
                threshold = self.config.skip_threshold,
                "skipping ISBN scan: existing ISBN meets confidence threshold"
            );
            return Ok(IsbnScanResult {
                book_id,
                isbns_found: 0,
                isbns_stored: 0,
                files_scanned: 0,
            });
        }

        // 2. Load all BookFile records for this book.
        let files = BookFileRepository::get_by_book_id(&self.db_pool, book_id)
            .await
            .map_err(|e| TaskError::Failed(format!("failed to load book files: {e}")))?;

        if files.is_empty() {
            debug!(book_id = %book_id, "no files found for ISBN scan");
            return Ok(IsbnScanResult {
                book_id,
                isbns_found: 0,
                isbns_stored: 0,
                files_scanned: 0,
            });
        }

        // 3. For each file: read from storage -> extract text -> scan for ISBNs.
        let mut all_isbns: Vec<ScannedIsbn> = Vec::new();
        let mut files_scanned: usize = 0;

        for file in &files {
            let data = match self.storage.read(&file.storage_path).await {
                Ok(data) => data,
                Err(e) => {
                    warn!(
                        book_id = %book_id,
                        file_id = %file.id,
                        path = %file.storage_path,
                        error = %e,
                        "failed to read file for ISBN scan, skipping"
                    );
                    continue;
                }
            };

            let text = match archivis_formats::content_text::extract_content_text(
                &data,
                file.format,
                &self.config.text_config,
            ) {
                Ok(Some(text)) => text,
                Ok(None) => {
                    debug!(
                        file_id = %file.id,
                        format = %file.format,
                        "format not supported for content scanning, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        book_id = %book_id,
                        file_id = %file.id,
                        format = %file.format,
                        error = %e,
                        "failed to extract text for ISBN scan, skipping"
                    );
                    continue;
                }
            };

            files_scanned += 1;

            if text.is_empty() {
                continue;
            }

            let found = scan_text_for_isbns(&text, true);
            debug!(
                file_id = %file.id,
                format = %file.format,
                isbns = found.len(),
                "scanned file for ISBNs"
            );

            for isbn in found {
                let already = all_isbns
                    .iter()
                    .any(|s| s.identifier_type == isbn.identifier_type && s.value == isbn.value);
                if !already {
                    all_isbns.push(isbn);
                }
            }
        }

        // 4. Evidence weighting: discount confidence when multiple ISBNs are
        //    found (more ISBNs = higher probability of bibliography noise).
        let effective_confidence =
            scan_evidence_confidence(self.config.confidence, all_isbns.len());

        debug!(
            book_id = %book_id,
            total_isbns = all_isbns.len(),
            base_confidence = self.config.confidence,
            effective_confidence,
            "scan evidence weighting applied"
        );

        // 5. Cap stored ISBNs.  Natural ordering (ISBN-13 first, document
        //    position second) is already ideal: earlier-in-text ISBNs are
        //    more likely to be the book's own identifier.
        let capped = all_isbns.len().min(MAX_SCAN_ISBNS);
        if all_isbns.len() > MAX_SCAN_ISBNS {
            debug!(
                book_id = %book_id,
                total = all_isbns.len(),
                kept = MAX_SCAN_ISBNS,
                "capping scan ISBNs to prevent noise"
            );
        }

        // 6. Store new ISBNs as Identifier records.
        let mut isbns_stored: usize = 0;

        for isbn in &all_isbns[..capped] {
            // Check if this exact identifier already exists for the book.
            let id_type_str = serde_json::to_value(isbn.identifier_type)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();

            let exists = IdentifierRepository::exists_for_book(
                &self.db_pool,
                book_id,
                &id_type_str,
                &isbn.value,
            )
            .await
            .map_err(|e| TaskError::Failed(format!("failed to check existing identifier: {e}")))?;

            if exists {
                debug!(
                    book_id = %book_id,
                    isbn = %isbn.value,
                    "ISBN already exists for book, skipping"
                );
                continue;
            }

            let identifier = Identifier::new(
                book_id,
                isbn.identifier_type,
                &isbn.value,
                MetadataSource::ContentScan,
                effective_confidence,
            );

            IdentifierRepository::create(&self.db_pool, &identifier)
                .await
                .map_err(|e| TaskError::Failed(format!("failed to store identifier: {e}")))?;

            isbns_stored += 1;
        }

        info!(
            book_id = %book_id,
            files_scanned = files_scanned,
            isbns_found = all_isbns.len(),
            isbns_stored = isbns_stored,
            "ISBN content scan complete"
        );

        Ok(IsbnScanResult {
            book_id,
            isbns_found: all_isbns.len(),
            isbns_stored,
            files_scanned,
        })
    }
}

/// Evidence weighting for content-scan ISBNs.
///
/// When multiple ISBNs are discovered in a book's content, each individual
/// ISBN is less likely to be the book's *own* primary identifier — the extras
/// are often bibliographic citations, references to other editions, or noise.
///
/// | Unique ISBNs found | Discount factor | Effective (base 0.5) |
/// |--------------------|-----------------|----------------------|
/// |        1           |     1.0         |  0.50                |
/// |        2           |     0.8         |  0.40                |
/// |       3+           |     0.6         |  0.30                |
fn scan_evidence_confidence(base_confidence: f32, isbn_count: usize) -> f32 {
    let factor = match isbn_count {
        0 => return 0.0,
        1 => 1.0,
        2 => 0.8,
        _ => 0.6,
    };
    (base_confidence * factor).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_confidence_below_auto_apply_threshold() {
        let config = IsbnScanConfig::default();
        // Scan confidence must stay well below the auto-apply threshold (0.85)
        // so scan-only evidence never reaches auto-apply territory.
        assert!(
            config.confidence <= 0.5,
            "default scan confidence ({}) should be <= 0.5",
            config.confidence
        );
    }

    #[test]
    fn evidence_weighting_single_isbn() {
        let conf = scan_evidence_confidence(0.5, 1);
        assert!(
            (conf - 0.5).abs() < f32::EPSILON,
            "expected 0.5, got {conf}"
        );
    }

    #[test]
    fn evidence_weighting_two_isbns() {
        let conf = scan_evidence_confidence(0.5, 2);
        assert!(
            (conf - 0.4).abs() < f32::EPSILON,
            "expected 0.4, got {conf}"
        );
    }

    #[test]
    fn evidence_weighting_many_isbns() {
        let conf = scan_evidence_confidence(0.5, 5);
        assert!(
            (conf - 0.3).abs() < f32::EPSILON,
            "expected 0.3, got {conf}"
        );

        // 10+ ISBNs get same discount as 3+
        let conf10 = scan_evidence_confidence(0.5, 10);
        assert!(
            (conf - conf10).abs() < f32::EPSILON,
            "3+ all get same discount"
        );
    }

    #[test]
    fn evidence_weighting_zero_isbns() {
        let conf = scan_evidence_confidence(0.5, 0);
        assert!(conf.abs() < f32::EPSILON, "expected 0.0, got {conf}");
    }

    #[test]
    fn evidence_weighting_clamped() {
        // Even with high base confidence, result stays in [0, 1]
        assert!(scan_evidence_confidence(1.0, 1) <= 1.0);
    }

    #[test]
    fn max_scan_isbns_is_bounded() {
        // Compile-time guarantee that cap stays in a reasonable range.
        const _: () = {
            assert!(MAX_SCAN_ISBNS <= 10, "cap should be small to limit noise");
            assert!(MAX_SCAN_ISBNS >= 3, "cap should allow a few for discovery");
        };
    }
}
