//! Multi-provider resolver with confidence scoring.
//!
//! Queries multiple metadata providers, scores and ranks results,
//! cross-validates between sources, and selects the best match.

use std::sync::Arc;

use archivis_core::models::{IdentifierType, MetadataSource};
use tracing::{debug, warn};

use crate::provider::MetadataProvider;
use crate::registry::ProviderRegistry;
use crate::similarity;
use crate::types::{MetadataQuery, ProviderIdentifier, ProviderMetadata};

// ── Scoring constants ───────────────────────────────────────────────

/// Bonus for ISBN exact match between query and candidate.
const ISBN_MATCH_BONUS: f32 = 0.2;

/// Maximum bonus for title similarity.
const TITLE_SIMILARITY_MAX_BONUS: f32 = 0.15;

/// Maximum bonus for author similarity.
const AUTHOR_MATCH_MAX_BONUS: f32 = 0.1;

/// Bonus when multiple providers return the same book.
const CROSS_PROVIDER_BONUS: f32 = 0.1;

/// Penalty when candidate title is very different from existing title.
const CONTRADICTION_PENALTY: f32 = 0.15;

/// Title similarity below which a contradiction penalty is applied.
const CONTRADICTION_THRESHOLD: f32 = 0.3;

/// Default auto-apply threshold.
const DEFAULT_AUTO_APPLY_THRESHOLD: f32 = 0.85;

// ── Public types ────────────────────────────────────────────────────

/// Result of resolving metadata across multiple providers.
#[derive(Debug, Clone)]
pub struct ResolverResult {
    /// All candidates, sorted by score descending.
    pub candidates: Vec<ScoredCandidate>,
    /// The best match (highest score), if any.
    pub best_match: Option<ScoredCandidate>,
    /// Whether the best match score meets the auto-apply threshold.
    pub auto_apply: bool,
}

/// A candidate result with a composite confidence score.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    /// The provider metadata for this candidate.
    pub metadata: ProviderMetadata,
    /// Composite confidence score (0.0-1.0).
    pub score: f32,
    /// Name of the provider that returned this candidate.
    pub provider_name: String,
    /// Human-readable reasons for the score.
    pub match_reasons: Vec<String>,
}

/// What we already know about a book (from import/embedded metadata).
///
/// Used for cross-validation against provider results.
#[derive(Debug, Clone)]
pub struct ExistingBookMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub identifiers: Vec<ProviderIdentifier>,
    pub metadata_source: MetadataSource,
}

impl Default for ExistingBookMetadata {
    fn default() -> Self {
        Self {
            title: None,
            authors: Vec::new(),
            identifiers: Vec::new(),
            metadata_source: MetadataSource::Embedded,
        }
    }
}

/// The multi-provider metadata resolver.
///
/// Queries all available providers, scores results, cross-validates,
/// deduplicates, and determines whether to auto-apply.
pub struct MetadataResolver {
    registry: Arc<ProviderRegistry>,
    auto_apply_threshold: f32,
}

impl MetadataResolver {
    /// Create a new resolver with the given provider registry and
    /// auto-apply threshold.
    pub fn new(registry: Arc<ProviderRegistry>, auto_apply_threshold: f32) -> Self {
        Self {
            registry,
            auto_apply_threshold,
        }
    }

    /// Create a resolver with the default auto-apply threshold (0.85).
    pub fn with_defaults(registry: Arc<ProviderRegistry>) -> Self {
        Self::new(registry, DEFAULT_AUTO_APPLY_THRESHOLD)
    }

    /// Resolve metadata for a book using all available providers.
    ///
    /// Tries ISBN lookup first (cheapest, highest confidence), then falls
    /// back to title+author search.
    pub async fn resolve(
        &self,
        query: &MetadataQuery,
        existing: Option<&ExistingBookMetadata>,
    ) -> ResolverResult {
        let providers = self.registry.available();
        if providers.is_empty() {
            debug!("no available metadata providers");
            return empty_result();
        }

        // Phase 1+2: Gather candidates from providers.
        let mut all_candidates = self.gather_candidates(query, &providers).await;

        if all_candidates.is_empty() {
            return empty_result();
        }

        // Phase 3: Score each candidate.
        for candidate in &mut all_candidates {
            score_candidate(candidate, query, existing);
        }

        // Phase 4: Cross-provider deduplication and bonuses.
        apply_cross_provider_bonuses(&mut all_candidates);

        // Phase 5: Sort by score descending.
        all_candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Phase 6: Determine auto-apply.
        let best_match = all_candidates.first().cloned();
        let auto_apply = best_match
            .as_ref()
            .is_some_and(|m| m.score >= self.auto_apply_threshold && has_multi_signal(m));

        ResolverResult {
            candidates: all_candidates,
            best_match,
            auto_apply,
        }
    }

    /// Gather candidates from all providers via ISBN lookup and/or search.
    async fn gather_candidates(
        &self,
        query: &MetadataQuery,
        providers: &[Arc<dyn MetadataProvider>],
    ) -> Vec<ScoredCandidate> {
        // ISBN lookup (concurrent across providers).
        let candidates = if let Some(isbn) = &query.isbn {
            debug!(isbn = %isbn, "resolving metadata via ISBN lookup");
            query_providers_isbn(providers, isbn).await
        } else {
            Vec::new()
        };

        // Title+author search fallback.
        if !candidates.is_empty() {
            return candidates;
        }

        if query.title.is_some() {
            debug!("no ISBN results, falling back to title+author search");
            query_providers_search(providers, query).await
        } else {
            debug!("no ISBN and no title in query, cannot search");
            Vec::new()
        }
    }
}

/// Query all providers by ISBN concurrently, returning scored candidates.
async fn query_providers_isbn(
    providers: &[Arc<dyn MetadataProvider>],
    isbn: &str,
) -> Vec<ScoredCandidate> {
    let futs: Vec<_> = providers
        .iter()
        .map(|p| {
            let provider = Arc::clone(p);
            let isbn = isbn.to_string();
            async move {
                let name = provider.name().to_string();
                match provider.lookup_isbn(&isbn).await {
                    Ok(results) => {
                        debug!(provider = %name, count = results.len(), "ISBN lookup results");
                        results
                            .into_iter()
                            .map(|m| (name.clone(), m))
                            .collect::<Vec<_>>()
                    }
                    Err(e) => {
                        warn!(provider = %name, error = %e, "ISBN lookup failed");
                        Vec::new()
                    }
                }
            }
        })
        .collect();

    let results = futures::future::join_all(futs).await;
    results
        .into_iter()
        .flatten()
        .map(|(provider_name, metadata)| ScoredCandidate {
            provider_name,
            score: metadata.confidence,
            match_reasons: Vec::new(),
            metadata,
        })
        .collect()
}

/// Query all providers by title+author search concurrently.
async fn query_providers_search(
    providers: &[Arc<dyn MetadataProvider>],
    query: &MetadataQuery,
) -> Vec<ScoredCandidate> {
    let futs: Vec<_> = providers
        .iter()
        .map(|p| {
            let provider = Arc::clone(p);
            let q = query.clone();
            async move {
                let name = provider.name().to_string();
                match provider.search(&q).await {
                    Ok(results) => {
                        debug!(provider = %name, count = results.len(), "search results");
                        results
                            .into_iter()
                            .map(|m| (name.clone(), m))
                            .collect::<Vec<_>>()
                    }
                    Err(e) => {
                        warn!(provider = %name, error = %e, "search failed");
                        Vec::new()
                    }
                }
            }
        })
        .collect();

    let results = futures::future::join_all(futs).await;
    results
        .into_iter()
        .flatten()
        .map(|(provider_name, metadata)| ScoredCandidate {
            provider_name,
            score: metadata.confidence,
            match_reasons: Vec::new(),
            metadata,
        })
        .collect()
}

/// Build an empty resolver result.
fn empty_result() -> ResolverResult {
    ResolverResult {
        candidates: Vec::new(),
        best_match: None,
        auto_apply: false,
    }
}

// ── Scoring helpers ─────────────────────────────────────────────────

/// Score an individual candidate based on the query and existing metadata.
fn score_candidate(
    candidate: &mut ScoredCandidate,
    query: &MetadataQuery,
    existing: Option<&ExistingBookMetadata>,
) {
    let mut score = candidate.metadata.confidence;
    let mut reasons = Vec::new();

    // ── ISBN exact match bonus ──
    if let Some(query_isbn) = &query.isbn {
        if candidate_has_isbn(&candidate.metadata, query_isbn) {
            score += ISBN_MATCH_BONUS;
            reasons.push("ISBN exact match".to_string());
        }
    }

    // ── Title similarity bonus/penalty ──
    if let Some(existing) = existing {
        if let (Some(existing_title), Some(candidate_title)) =
            (&existing.title, &candidate.metadata.title)
        {
            let title_sim = similarity::similarity(existing_title, candidate_title);

            if title_sim >= CONTRADICTION_THRESHOLD {
                let bonus = title_sim * TITLE_SIMILARITY_MAX_BONUS;
                score += bonus;
                reasons.push(format!("Title fuzzy match ({:.0}%)", title_sim * 100.0));
            } else {
                // Contradiction: candidate title very different from existing.
                score -= CONTRADICTION_PENALTY;
                reasons.push(format!(
                    "Title contradiction ({:.0}% similarity)",
                    title_sim * 100.0
                ));
            }
        }

        // ── Author match bonus ──
        if !existing.authors.is_empty() {
            let candidate_authors: Vec<String> = candidate
                .metadata
                .authors
                .iter()
                .map(|a| a.name.clone())
                .collect();

            if !candidate_authors.is_empty() {
                let author_sim =
                    similarity::author_similarity(&existing.authors, &candidate_authors);
                let bonus = author_sim * AUTHOR_MATCH_MAX_BONUS;
                score += bonus;
                if author_sim > 0.5 {
                    reasons.push(format!("Author match ({:.0}%)", author_sim * 100.0));
                }
            }
        }
    }

    // Clamp to 0.0-1.0.
    score = score.clamp(0.0, 1.0);

    candidate.score = score;
    candidate.match_reasons = reasons;
}

/// Check whether a candidate's identifiers contain the given ISBN.
fn candidate_has_isbn(metadata: &ProviderMetadata, isbn: &str) -> bool {
    let normalized_query = isbn.replace('-', "");
    metadata.identifiers.iter().any(|id| {
        matches!(
            id.identifier_type,
            IdentifierType::Isbn13 | IdentifierType::Isbn10
        ) && id.value.replace('-', "") == normalized_query
    })
}

/// Apply cross-provider bonuses and deduplicate.
///
/// If multiple providers returned results for the same book (matched by
/// ISBN or fuzzy title+author), boost the higher-scored one and merge
/// unique data.
fn apply_cross_provider_bonuses(candidates: &mut Vec<ScoredCandidate>) {
    if candidates.len() < 2 {
        return;
    }

    // Find pairs that refer to the same book.
    let len = candidates.len();
    let mut merged_indices: Vec<bool> = vec![false; len];

    for i in 0..len {
        if merged_indices[i] {
            continue;
        }
        for j in (i + 1)..len {
            if merged_indices[j] {
                continue;
            }
            // Same provider — skip.
            if candidates[i].provider_name == candidates[j].provider_name {
                continue;
            }

            if are_same_book(&candidates[i], &candidates[j]) {
                // Cross-provider corroboration found.
                debug!(
                    provider_a = %candidates[i].provider_name,
                    provider_b = %candidates[j].provider_name,
                    "cross-provider match found"
                );

                // Boost the higher-scored candidate and merge data.
                let (winner_idx, loser_idx) = if candidates[i].score >= candidates[j].score {
                    (i, j)
                } else {
                    (j, i)
                };

                // Clone the loser's data before mutating the winner to
                // avoid simultaneous mutable and immutable borrows.
                let loser_name = candidates[loser_idx].provider_name.clone();
                let loser_metadata = candidates[loser_idx].metadata.clone();

                candidates[winner_idx].score =
                    (candidates[winner_idx].score + CROSS_PROVIDER_BONUS).min(1.0);
                candidates[winner_idx]
                    .match_reasons
                    .push(format!("Cross-provider match ({loser_name})"));

                // Merge unique data from the loser into the winner.
                merge_metadata(&mut candidates[winner_idx].metadata, &loser_metadata);

                merged_indices[loser_idx] = true;
            }
        }
    }

    // Remove merged (deduplicated) candidates.
    let mut idx = 0;
    candidates.retain(|_| {
        let keep = !merged_indices[idx];
        idx += 1;
        keep
    });
}

/// Determine if two candidates refer to the same book.
///
/// Match by ISBN first, then by fuzzy title+author.
fn are_same_book(a: &ScoredCandidate, b: &ScoredCandidate) -> bool {
    // Match by ISBN.
    for a_id in &a.metadata.identifiers {
        if !matches!(
            a_id.identifier_type,
            IdentifierType::Isbn13 | IdentifierType::Isbn10
        ) {
            continue;
        }
        for b_id in &b.metadata.identifiers {
            if !matches!(
                b_id.identifier_type,
                IdentifierType::Isbn13 | IdentifierType::Isbn10
            ) {
                continue;
            }
            if a_id.value.replace('-', "") == b_id.value.replace('-', "") {
                return true;
            }
        }
    }

    // Match by fuzzy title + author.
    if let (Some(a_title), Some(b_title)) = (&a.metadata.title, &b.metadata.title) {
        let title_sim = similarity::similarity(a_title, b_title);
        if title_sim > 0.85 {
            let a_authors: Vec<String> =
                a.metadata.authors.iter().map(|a| a.name.clone()).collect();
            let b_authors: Vec<String> =
                b.metadata.authors.iter().map(|a| a.name.clone()).collect();
            if a_authors.is_empty() && b_authors.is_empty() {
                return true;
            }
            let author_sim = similarity::author_similarity(&a_authors, &b_authors);
            if author_sim > 0.7 {
                return true;
            }
        }
    }

    false
}

/// Merge unique data from `source` into `target`.
///
/// Only fills in fields that are `None` or empty in the target.
fn merge_metadata(target: &mut ProviderMetadata, source: &ProviderMetadata) {
    if target.description.is_none() {
        target.description.clone_from(&source.description);
    }
    if target.language.is_none() {
        target.language.clone_from(&source.language);
    }
    if target.publisher.is_none() {
        target.publisher.clone_from(&source.publisher);
    }
    if target.publication_date.is_none() {
        target.publication_date.clone_from(&source.publication_date);
    }
    if target.series.is_none() {
        target.series.clone_from(&source.series);
    }
    if target.page_count.is_none() {
        target.page_count = source.page_count;
    }
    if target.cover_url.is_none() {
        target.cover_url.clone_from(&source.cover_url);
    }
    if target.rating.is_none() {
        target.rating = source.rating;
    }
    if target.subjects.is_empty() {
        target.subjects.clone_from(&source.subjects);
    }

    // Merge identifiers that the target doesn't already have.
    for source_id in &source.identifiers {
        let already_has = target.identifiers.iter().any(|t_id| {
            t_id.identifier_type == source_id.identifier_type && t_id.value == source_id.value
        });
        if !already_has {
            target.identifiers.push(source_id.clone());
        }
    }
}

/// Check whether a candidate has multiple corroborating signals.
///
/// A single signal (ISBN alone without title cross-validation) should NOT
/// auto-apply per the design doc.
fn has_multi_signal(candidate: &ScoredCandidate) -> bool {
    let has_isbn_match = candidate
        .match_reasons
        .iter()
        .any(|r| r.contains("ISBN exact match"));
    let has_title_match = candidate
        .match_reasons
        .iter()
        .any(|r| r.contains("Title fuzzy match") || r.contains("Title match"));
    let has_cross_provider = candidate
        .match_reasons
        .iter()
        .any(|r| r.contains("Cross-provider"));
    let has_author_match = candidate
        .match_reasons
        .iter()
        .any(|r| r.contains("Author match"));

    // Need at least two distinct signals.
    let signal_count = usize::from(has_isbn_match)
        + usize::from(has_title_match)
        + usize::from(has_cross_provider)
        + usize::from(has_author_match);

    signal_count >= 2
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::errors::ProviderError;
    use crate::provider::MetadataProvider;
    use crate::types::{ProviderAuthor, ProviderSeries};

    /// Helper to build a minimal `ProviderMetadata`.
    fn make_metadata(
        provider: &str,
        title: &str,
        authors: &[&str],
        isbn13: Option<&str>,
        confidence: f32,
    ) -> ProviderMetadata {
        let mut identifiers = Vec::new();
        if let Some(isbn) = isbn13 {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: isbn.to_string(),
            });
        }
        ProviderMetadata {
            provider_name: provider.to_string(),
            title: Some(title.to_string()),
            authors: authors
                .iter()
                .map(|a| ProviderAuthor {
                    name: (*a).to_string(),
                    role: Some("author".to_string()),
                })
                .collect(),
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers,
            subjects: Vec::new(),
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence,
        }
    }

    /// A configurable stub provider for testing the resolver.
    struct StubProvider {
        name: String,
        isbn_results: Vec<ProviderMetadata>,
        search_results: Vec<ProviderMetadata>,
    }

    impl StubProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                isbn_results: Vec::new(),
                search_results: Vec::new(),
            }
        }

        fn with_isbn_results(mut self, results: Vec<ProviderMetadata>) -> Self {
            self.isbn_results = results;
            self
        }

        fn with_search_results(mut self, results: Vec<ProviderMetadata>) -> Self {
            self.search_results = results;
            self
        }
    }

    #[async_trait]
    impl MetadataProvider for StubProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn lookup_isbn(&self, _isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(self.isbn_results.clone())
        }

        async fn search(
            &self,
            _query: &MetadataQuery,
        ) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(self.search_results.clone())
        }

        async fn fetch_cover(&self, _cover_url: &str) -> Result<Vec<u8>, ProviderError> {
            Ok(vec![])
        }
    }

    fn make_registry(providers: Vec<Arc<dyn MetadataProvider>>) -> Arc<ProviderRegistry> {
        let mut registry = ProviderRegistry::new();
        for p in providers {
            registry.register(p);
        }
        Arc::new(registry)
    }

    // ── Tests ──

    #[tokio::test]
    async fn two_providers_same_book_deduplication() {
        let ol = StubProvider::new("open_library").with_isbn_results(vec![make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        )]);
        let hc = StubProvider::new("hardcover").with_isbn_results(vec![make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        )]);

        let registry = make_registry(vec![Arc::new(ol), Arc::new(hc)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        // Should deduplicate to a single candidate.
        assert_eq!(
            result.candidates.len(),
            1,
            "expected 1 candidate after dedup, got {}",
            result.candidates.len()
        );

        // Cross-provider bonus should be applied.
        let best = result.best_match.as_ref().unwrap();
        assert!(
            best.match_reasons
                .iter()
                .any(|r| r.contains("Cross-provider")),
            "expected cross-provider match reason"
        );
    }

    #[tokio::test]
    async fn isbn_match_plus_title_match_auto_applies() {
        let ol = StubProvider::new("open_library").with_isbn_results(vec![make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        )]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        assert!(result.best_match.is_some());
        let best = result.best_match.as_ref().unwrap();

        // Score should be high: 0.95 base + 0.2 ISBN + title + author bonuses.
        assert!(
            best.score >= 0.85,
            "expected score >= 0.85, got {}",
            best.score
        );
        assert!(result.auto_apply, "expected auto_apply to be true");
    }

    #[tokio::test]
    async fn isbn_match_but_title_mismatch_no_auto_apply() {
        // Use titles with very low Jaro-Winkler similarity to trigger contradiction.
        // Short, completely different strings ensure similarity < 0.3.
        let ol = StubProvider::new("open_library").with_isbn_results(vec![make_metadata(
            "open_library",
            "XYZ",
            &["Somebody Else"],
            Some("9780441172719"),
            0.95,
        )]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        assert!(result.best_match.is_some());
        let best = result.best_match.as_ref().unwrap();

        // Contradiction penalty should reduce confidence.
        assert!(
            best.match_reasons
                .iter()
                .any(|r| r.contains("contradiction")),
            "expected contradiction reason, got: {:?}",
            best.match_reasons
        );

        // Should NOT auto-apply despite ISBN match because of title contradiction.
        assert!(
            !result.auto_apply,
            "should not auto-apply when title contradicts"
        );
    }

    #[tokio::test]
    async fn title_only_search_no_auto_apply() {
        // Provider returns moderate confidence (0.5) — typical for search results.
        let ol = StubProvider::new("open_library").with_search_results(vec![make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            None,
            0.5,
        )]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        assert!(result.best_match.is_some());
        let best = result.best_match.as_ref().unwrap();

        // Score = 0.5 base + 0.15 title + 0.1 author = 0.75.
        // Below 0.85 threshold, should not auto-apply.
        assert!(
            best.score < 0.85,
            "expected score < 0.85 for search-only result, got {}",
            best.score
        );
        assert!(
            !result.auto_apply,
            "title-only search should not auto-apply without cross-validation"
        );
    }

    #[tokio::test]
    async fn no_providers_returns_empty() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };

        let result = resolver.resolve(&query, None).await;
        assert!(result.candidates.is_empty());
        assert!(result.best_match.is_none());
        assert!(!result.auto_apply);
    }

    #[tokio::test]
    async fn no_isbn_no_title_returns_empty() {
        let ol = StubProvider::new("open_library");
        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery::default();

        let result = resolver.resolve(&query, None).await;
        assert!(result.candidates.is_empty());
    }

    #[tokio::test]
    async fn candidates_sorted_by_score() {
        let ol = StubProvider::new("open_library").with_search_results(vec![
            make_metadata("open_library", "Dune", &["Frank Herbert"], None, 0.5),
            make_metadata(
                "open_library",
                "Dune Messiah",
                &["Frank Herbert"],
                None,
                0.8,
            ),
        ]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            title: Some("Dune Messiah".to_string()),
            ..Default::default()
        };

        let result = resolver.resolve(&query, None).await;
        assert!(result.candidates.len() >= 2);

        // Verify descending order.
        for window in result.candidates.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "candidates not sorted by score: {} < {}",
                window[0].score,
                window[1].score
            );
        }
    }

    // ── Scoring unit tests ──

    #[test]
    fn candidate_has_isbn_normalizes_dashes() {
        let meta = make_metadata("test", "Dune", &[], Some("978-0-441-17271-9"), 0.9);
        assert!(candidate_has_isbn(&meta, "9780441172719"));
        assert!(candidate_has_isbn(&meta, "978-0-441-17271-9"));
    }

    #[test]
    fn candidate_has_isbn_no_match() {
        let meta = make_metadata("test", "Dune", &[], Some("9780441172719"), 0.9);
        assert!(!candidate_has_isbn(&meta, "9781234567890"));
    }

    #[test]
    fn are_same_book_by_isbn() {
        let a = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], Some("9780441172719"), 0.9),
            score: 0.9,
            provider_name: "open_library".to_string(),
            match_reasons: Vec::new(),
        };
        let b = ScoredCandidate {
            metadata: make_metadata("hc", "Dune", &["Frank Herbert"], Some("9780441172719"), 0.9),
            score: 0.9,
            provider_name: "hardcover".to_string(),
            match_reasons: Vec::new(),
        };
        assert!(are_same_book(&a, &b));
    }

    #[test]
    fn are_same_book_by_fuzzy_title_author() {
        let a = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.9),
            score: 0.9,
            provider_name: "open_library".to_string(),
            match_reasons: Vec::new(),
        };
        let b = ScoredCandidate {
            metadata: make_metadata("hc", "Dune", &["Herbert, Frank"], None, 0.9),
            score: 0.9,
            provider_name: "hardcover".to_string(),
            match_reasons: Vec::new(),
        };
        assert!(are_same_book(&a, &b));
    }

    #[test]
    fn are_not_same_book_different_titles() {
        let a = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.9),
            score: 0.9,
            provider_name: "open_library".to_string(),
            match_reasons: Vec::new(),
        };
        let b = ScoredCandidate {
            metadata: make_metadata("hc", "Foundation", &["Isaac Asimov"], None, 0.9),
            score: 0.9,
            provider_name: "hardcover".to_string(),
            match_reasons: Vec::new(),
        };
        assert!(!are_same_book(&a, &b));
    }

    #[test]
    fn has_multi_signal_requires_two_signals() {
        let candidate = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &[], None, 0.95),
            score: 0.95,
            provider_name: "open_library".to_string(),
            match_reasons: vec!["ISBN exact match".to_string()],
        };
        assert!(
            !has_multi_signal(&candidate),
            "single signal should not pass multi-signal check"
        );
    }

    #[test]
    fn has_multi_signal_isbn_plus_title() {
        let candidate = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &[], None, 0.95),
            score: 0.95,
            provider_name: "open_library".to_string(),
            match_reasons: vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (95%)".to_string(),
            ],
        };
        assert!(
            has_multi_signal(&candidate),
            "ISBN + title should pass multi-signal check"
        );
    }

    #[test]
    fn merge_metadata_fills_missing_fields() {
        let mut target = ProviderMetadata {
            provider_name: "open_library".to_string(),
            title: Some("Dune".to_string()),
            authors: vec![],
            description: None,
            language: Some("en".to_string()),
            publisher: None,
            publication_date: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            subjects: Vec::new(),
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.9,
        };

        let source = ProviderMetadata {
            provider_name: "hardcover".to_string(),
            title: Some("Dune (different subtitle)".to_string()),
            authors: vec![],
            description: Some("A sci-fi classic".to_string()),
            language: Some("fr".to_string()), // Different — should NOT overwrite.
            publisher: Some("Chilton Books".to_string()),
            publication_date: Some("1965".to_string()),
            identifiers: vec![
                ProviderIdentifier {
                    identifier_type: IdentifierType::Isbn13,
                    value: "9780441172719".to_string(), // Same — should NOT duplicate.
                },
                ProviderIdentifier {
                    identifier_type: IdentifierType::Hardcover,
                    value: "hc-123".to_string(), // New — should be added.
                },
            ],
            subjects: vec!["Science Fiction".to_string()],
            series: Some(ProviderSeries {
                name: "Dune".to_string(),
                position: Some(1.0),
            }),
            page_count: Some(412),
            cover_url: Some("https://example.com/cover.jpg".to_string()),
            rating: Some(4.5),
            confidence: 0.9,
        };

        merge_metadata(&mut target, &source);

        // Filled from source.
        assert_eq!(target.description.as_deref(), Some("A sci-fi classic"));
        assert_eq!(target.publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(target.publication_date.as_deref(), Some("1965"));
        assert_eq!(target.series.as_ref().unwrap().name, "Dune");
        assert_eq!(target.page_count, Some(412));
        assert_eq!(
            target.cover_url.as_deref(),
            Some("https://example.com/cover.jpg")
        );
        assert_eq!(target.rating, Some(4.5));
        assert_eq!(target.subjects, vec!["Science Fiction"]);

        // NOT overwritten (target already had language).
        assert_eq!(target.language.as_deref(), Some("en"));

        // Identifiers: should have ISBN-13 (existing) + Hardcover (new).
        assert_eq!(target.identifiers.len(), 2);
        assert!(target
            .identifiers
            .iter()
            .any(|id| id.identifier_type == IdentifierType::Hardcover));
    }
}
