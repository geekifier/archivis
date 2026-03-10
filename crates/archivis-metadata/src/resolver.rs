//! Multi-provider metadata resolver with confidence scoring.
//!
//! ## Resolution pipeline (7 phases)
//!
//! 1. **Provider query** — ISBN / ASIN identifier lookups (concurrent), with
//!    fallback to title+author search when no ID results are found.
//! 2. **Candidate gathering** — raw candidates collected from all providers.
//! 3. **Scoring** — each candidate receives a composite confidence score
//!    (base provider confidence ± ISBN/ASIN match bonus, title/author
//!    similarity, and contradiction penalty).
//! 4. **Dedup & cross-provider merge** — same-provider duplicates are merged
//!    by strongest-wins; cross-provider ISBN groups are merged with a
//!    corroboration bonus; fuzzy cross-provider matches receive a bonus
//!    without merging.
//! 5. **Sort** — candidates ordered by score descending.
//! 6. **Tier assignment** — each candidate receives a [`CandidateMatchTier`]:
//!    - `StrongIdMatch`: hard ID signal + content corroboration + trusted
//!      identifier proof from the book. Only tier eligible for auto-apply.
//!    - `ProbableMatch`: ID signal or cross-provider corroboration, but
//!      missing content or trusted-ID proof.
//!    - `WeakMatch`: content-only, speculative.
//! 7. **Auto-apply decision** — returns a boolean + [`ResolverDecision`]
//!    reason code; gates: tier must be `StrongIdMatch`, score above
//!    threshold, at most one candidate above threshold, and minimum gap
//!    between top two.
//!
//! ## Trust hierarchy
//!
//! `User > Embedded > ContentScan > Provider > Filename`. `User` and
//! `Embedded` identifiers are always *trusted* for [`StrongIdMatch`]
//! tier promotion. A **single** `ContentScan` ISBN is also trusted (it
//! is almost certainly the book's own ISBN); multiple scan ISBNs are
//! excluded (bibliography noise guard).

use std::collections::HashSet;
use std::sync::Arc;

use archivis_core::isbn::{normalize_asin, normalize_isbn, to_isbn13};
use archivis_core::models::{IdentifierType, MetadataSource};
use archivis_core::settings::SettingsReader;
use tracing::{debug, warn};

use crate::provider::MetadataProvider;
use crate::provider_names;
use crate::registry::ProviderRegistry;
use crate::similarity;
use crate::types::{MetadataQuery, ProviderFeature, ProviderIdentifier, ProviderMetadata};

// ── Scoring constants ───────────────────────────────────────────────

/// Bonus for ISBN exact match between query and candidate.
const ISBN_MATCH_BONUS: f32 = 0.2;

/// Bonus for ASIN exact match between query and candidate.
const ASIN_MATCH_BONUS: f32 = 0.2;

/// Maximum bonus for title similarity.
const TITLE_SIMILARITY_MAX_BONUS: f32 = 0.10;

/// Maximum bonus for author similarity.
const AUTHOR_MATCH_MAX_BONUS: f32 = 0.05;

/// Maximum bonus for publisher similarity.
const PUBLISHER_MATCH_BONUS: f32 = 0.05;

/// Bonus when multiple providers return the same book.
const CROSS_PROVIDER_BONUS: f32 = 0.1;

/// Maximum bonus for candidate field completeness (data richness).
/// Awarded proportionally to the number of valuable metadata fields present.
const FIELD_COMPLETENESS_MAX_BONUS: f32 = 0.05;

/// Penalty when candidate title is very different from existing title.
const CONTRADICTION_PENALTY: f32 = 0.15;

/// Title similarity below which a contradiction penalty is applied.
const CONTRADICTION_THRESHOLD: f32 = 0.3;

/// Default auto-apply threshold.
pub const DEFAULT_AUTO_APPLY_THRESHOLD: f32 = 0.85;

/// Minimum gap between best and second-best candidate scores for auto-apply.
/// If the gap is smaller, results are considered ambiguous and need manual review.
const AUTO_APPLY_MIN_GAP: f32 = 0.15;

/// Score proximity threshold for considering two candidates "close".
/// If the top two candidates are within this range, auto-apply is suppressed.
const AUTO_APPLY_CLOSE_SCORE_RANGE: f32 = 0.1;

// ── Match enums (resolver-internal) ─────────────────────────────────

/// Strength of a same-book match between two candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BookMatch {
    No,
    Fuzzy,
    Strong,
}

/// Structured signal indicating why a candidate matched.
///
/// Resolver-internal; not stored in DB. The `match_reasons: Vec<String>` field
/// remains for human-readable display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MatchSignal {
    IsbnMatch,
    AsinMatch,
    TitleMatch,
    AuthorMatch,
    PublisherMatch,
    CrossProvider,
    Contradiction,
}

// ── Public types ────────────────────────────────────────────────────

/// Match confidence tier for a candidate.
///
/// Determines eligibility for automated actions. Only [`StrongIdMatch`] may
/// be auto-applied; all other tiers require manual review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateMatchTier {
    /// Hard identifier proof (ISBN/ASIN exact match) with content
    /// corroboration (title or author match) and a trusted identifier from
    /// the book. The only tier eligible for auto-apply.
    StrongIdMatch,
    /// Some identifier signal or cross-provider corroboration, but
    /// insufficient hard proof for auto-apply. Needs manual review.
    ProbableMatch,
    /// Fuzzy content-only match with no identifier proof. Speculative.
    WeakMatch,
}

impl std::fmt::Display for CandidateMatchTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StrongIdMatch => write!(f, "strong_id_match"),
            Self::ProbableMatch => write!(f, "probable_match"),
            Self::WeakMatch => write!(f, "weak_match"),
        }
    }
}

/// Structured reason code for the resolver's auto-apply decision.
///
/// Returned alongside the boolean auto-apply flag to provide a machine-readable
/// explanation of why auto-apply was allowed or blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverDecision {
    /// All checks passed — auto-apply is allowed.
    AutoApplyAllowed,
    /// Candidate has an ID signal (ISBN/ASIN match) but the book's trusted
    /// identifiers don't include a matching ID, so the tier stays below
    /// `StrongIdMatch`.
    BlockedNoTrustedId,
    /// Multiple candidates are above the threshold or the gap between the top
    /// two is too small for a confident decision.
    BlockedAmbiguous,
    /// Candidate has a title contradiction and no strong ID proof.
    BlockedContradiction,
    /// Best candidate's tier is below `StrongIdMatch` (catch-all for
    /// non-specific tier failures) or score is below threshold.
    BlockedLowTier,
    /// No candidates were returned by any provider.
    NoCandidates,
}

impl std::fmt::Display for ResolverDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AutoApplyAllowed => write!(f, "auto_apply_allowed"),
            Self::BlockedNoTrustedId => write!(f, "blocked_no_trusted_id"),
            Self::BlockedAmbiguous => write!(f, "blocked_ambiguous"),
            Self::BlockedContradiction => write!(f, "blocked_contradiction"),
            Self::BlockedLowTier => write!(f, "blocked_low_tier"),
            Self::NoCandidates => write!(f, "no_candidates"),
        }
    }
}

/// Result of resolving metadata across multiple providers.
#[derive(Debug, Clone)]
pub struct ResolverResult {
    /// All candidates, sorted by score descending.
    pub candidates: Vec<ScoredCandidate>,
    /// The best match (highest score), if any.
    pub best_match: Option<ScoredCandidate>,
    /// Whether the best match score meets the auto-apply threshold.
    pub auto_apply: bool,
    /// Machine-readable reason code for the auto-apply decision.
    pub decision: ResolverDecision,
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
    /// Structured match signals (resolver-internal, not stored in DB).
    signals: HashSet<MatchSignal>,
    /// Match confidence tier, assigned after scoring/dedup.
    pub tier: CandidateMatchTier,
    /// Cached count of populated metadata fields for sort tiebreaking.
    pub field_count: usize,
}

impl ScoredCandidate {
    /// Create a new `ScoredCandidate` with empty signals and `WeakMatch` tier.
    pub fn new(
        metadata: ProviderMetadata,
        score: f32,
        provider_name: String,
        match_reasons: Vec<String>,
    ) -> Self {
        Self {
            metadata,
            score,
            provider_name,
            match_reasons,
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        }
    }
}

/// What we already know about a book (from import/embedded metadata).
///
/// Used for cross-validation against provider results.
#[derive(Debug, Clone)]
pub struct ExistingBookMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub identifiers: Vec<ProviderIdentifier>,
    /// Identifiers from trusted sources (user, embedded) of trusted types
    /// (isbn13, isbn10, asin). Used as proof for auto-apply decisions.
    pub trusted_identifiers: Vec<ProviderIdentifier>,
    pub metadata_source: MetadataSource,
}

impl Default for ExistingBookMetadata {
    fn default() -> Self {
        Self {
            title: None,
            authors: Vec::new(),
            publisher: None,
            identifiers: Vec::new(),
            trusted_identifiers: Vec::new(),
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
    settings: Arc<dyn SettingsReader>,
}

impl MetadataResolver {
    /// Create a new resolver backed by live settings.
    pub fn new(registry: Arc<ProviderRegistry>, settings: Arc<dyn SettingsReader>) -> Self {
        Self { registry, settings }
    }

    /// Create a resolver with a fixed threshold for tests.
    #[cfg(test)]
    fn with_fixed_threshold(registry: Arc<ProviderRegistry>, threshold: f32) -> Self {
        use crate::test_util::StubSettings;
        let settings = Arc::new(StubSettings::new(vec![(
            "metadata.auto_apply_threshold",
            serde_json::json!(threshold),
        )]));
        Self { registry, settings }
    }

    /// Create a resolver with the default auto-apply threshold (0.85) for tests.
    #[cfg(test)]
    fn with_defaults(registry: Arc<ProviderRegistry>) -> Self {
        Self::with_fixed_threshold(registry, DEFAULT_AUTO_APPLY_THRESHOLD)
    }

    /// Read the current auto-apply threshold from settings.
    #[allow(clippy::cast_possible_truncation)] // threshold is always in 0.0..1.0
    fn auto_apply_threshold(&self) -> f32 {
        self.settings
            .get_setting("metadata.auto_apply_threshold")
            .and_then(|value| value.as_f64())
            .map_or(DEFAULT_AUTO_APPLY_THRESHOLD, |f| f as f32)
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

        // Phase 4: Deduplication and cross-provider bonuses.
        deduplicate_and_boost(&mut all_candidates);

        // Recompute cached `field_count` after merges may have changed metadata.
        for candidate in &mut all_candidates {
            candidate.field_count = FieldCounts::of(&candidate.metadata).present;
        }

        // Phase 5: Sort by score descending, then by field count (richer first).
        all_candidates.sort_by(cmp_candidates);

        // Phase 6: Assign match tiers and apply tier-based score adjustment.
        for candidate in &mut all_candidates {
            candidate.tier = assign_tier(candidate, existing);

            // Apply tier factor so the score reflects evidence quality.
            let tier_factor = match candidate.tier {
                CandidateMatchTier::StrongIdMatch => 1.0_f32,
                CandidateMatchTier::ProbableMatch => 0.85,
                CandidateMatchTier::WeakMatch => 0.65,
            };
            candidate.score = (candidate.score * tier_factor).clamp(0.0, 1.0);

            debug!(
                provider = %candidate.provider_name,
                score = candidate.score,
                tier = %candidate.tier,
                "candidate tier assigned"
            );
        }

        // Re-sort after tier adjustment (tier multiplier can change relative ordering).
        all_candidates.sort_by(cmp_candidates);

        // Phase 7: Determine auto-apply.
        let best_match = all_candidates.first().cloned();
        let (auto_apply, decision) = self.should_auto_apply(best_match.as_ref(), &all_candidates);

        ResolverResult {
            candidates: all_candidates,
            best_match,
            auto_apply,
            decision,
        }
    }

    /// Determine whether the best match should be auto-applied.
    ///
    /// Returns `(allowed, decision)` where `decision` is a machine-readable
    /// reason code.
    ///
    /// Conservative rules to avoid applying ambiguous results:
    /// 1. The best candidate must be tier `StrongIdMatch` (hard ID proof +
    ///    content corroboration + trusted identifier).
    /// 2. The best candidate must exceed the auto-apply threshold.
    /// 3. There must be at most ONE candidate above the threshold.
    /// 4. The top two candidates must NOT have scores within 0.1 of each other.
    /// 5. The second-best candidate must be at least 0.15 below the best.
    fn should_auto_apply(
        &self,
        best_match: Option<&ScoredCandidate>,
        candidates: &[ScoredCandidate],
    ) -> (bool, ResolverDecision) {
        let Some(best) = best_match else {
            return (false, ResolverDecision::NoCandidates);
        };

        let threshold = self.auto_apply_threshold();

        // Only StrongIdMatch tier candidates may auto-apply.
        if best.tier != CandidateMatchTier::StrongIdMatch {
            debug!(
                tier = %best.tier,
                "auto-apply suppressed: only strong_id_match tier may auto-apply"
            );
            // Distinguish *why* the tier is too low.
            let has_id_signal = best.signals.contains(&MatchSignal::IsbnMatch)
                || best.signals.contains(&MatchSignal::AsinMatch);
            let has_contradiction = best.signals.contains(&MatchSignal::Contradiction);

            let decision = if has_id_signal {
                ResolverDecision::BlockedNoTrustedId
            } else if has_contradiction {
                ResolverDecision::BlockedContradiction
            } else {
                ResolverDecision::BlockedLowTier
            };
            return (false, decision);
        }

        // Score must exceed threshold.
        if best.score < threshold {
            return (false, ResolverDecision::BlockedLowTier);
        }

        // Count how many candidates are above the auto-apply threshold.
        let above_threshold = candidates.iter().filter(|c| c.score >= threshold).count();

        if above_threshold > 1 {
            debug!(
                above_threshold,
                "auto-apply suppressed: multiple candidates above threshold"
            );
            return (false, ResolverDecision::BlockedAmbiguous);
        }

        // Check the gap between best and second-best.
        if let Some(second) = candidates.get(1) {
            let gap = best.score - second.score;

            if gap < AUTO_APPLY_CLOSE_SCORE_RANGE {
                debug!(
                    best_score = best.score,
                    second_score = second.score,
                    gap,
                    "auto-apply suppressed: top two candidates too close"
                );
                return (false, ResolverDecision::BlockedAmbiguous);
            }

            if gap < AUTO_APPLY_MIN_GAP {
                debug!(
                    best_score = best.score,
                    second_score = second.score,
                    gap,
                    "auto-apply suppressed: second-best too close (need gap >= {AUTO_APPLY_MIN_GAP})"
                );
                return (false, ResolverDecision::BlockedAmbiguous);
            }
        }

        (true, ResolverDecision::AutoApplyAllowed)
    }

    /// Gather candidates from all providers via identifier lookups and/or search.
    ///
    /// Runs ISBN and ASIN lookups concurrently when both exist, merges
    /// results, and lets the existing dedup phases handle duplicates.
    /// Falls back to title+author search only when identifier lookups yield nothing.
    async fn gather_candidates(
        &self,
        query: &MetadataQuery,
        providers: &[Arc<dyn MetadataProvider>],
    ) -> Vec<ScoredCandidate> {
        let mut candidates = Vec::new();

        // Fire ISBN and ASIN lookups concurrently when both are present;
        // each branch already fans out across providers via `join_all`.
        // Only send lookups to providers that advertise support.
        match (&query.isbn, &query.asin) {
            (Some(isbn), Some(asin)) => {
                let isbn_provs = filter_providers(
                    providers,
                    |p| p.capabilities().supports_isbn(),
                    "skipping ISBN lookup (not supported)",
                );
                let asin_provs = filter_providers(
                    providers,
                    |p| p.capabilities().supports_id_lookup(IdentifierType::Asin),
                    "skipping ASIN lookup (not supported)",
                );
                let normalized_asin = normalize_asin(asin);
                debug!(isbn = %isbn, "resolving metadata via ISBN lookup");
                debug!(asin = %normalized_asin, "resolving metadata via ASIN lookup");
                let (isbn_results, asin_results) = tokio::join!(
                    query_providers_by_id(&isbn_provs, isbn, IdentifierLookup::Isbn),
                    query_providers_by_id(&asin_provs, &normalized_asin, IdentifierLookup::Asin),
                );
                candidates.extend(isbn_results);
                candidates.extend(asin_results);
            }
            (Some(isbn), None) => {
                let isbn_provs = filter_providers(
                    providers,
                    |p| p.capabilities().supports_isbn(),
                    "skipping ISBN lookup (not supported)",
                );
                debug!(isbn = %isbn, "resolving metadata via ISBN lookup");
                candidates
                    .extend(query_providers_by_id(&isbn_provs, isbn, IdentifierLookup::Isbn).await);
            }
            (None, Some(asin)) => {
                let asin_provs = filter_providers(
                    providers,
                    |p| p.capabilities().supports_id_lookup(IdentifierType::Asin),
                    "skipping ASIN lookup (not supported)",
                );
                let normalized = normalize_asin(asin);
                debug!(asin = %normalized, "resolving metadata via ASIN lookup");
                candidates.extend(
                    query_providers_by_id(&asin_provs, &normalized, IdentifierLookup::Asin).await,
                );
            }
            (None, None) => {}
        }

        // Drop candidates with unsupported physical formats (audiobooks, etc.).
        filter_unsupported_candidates(&mut candidates);

        // If any identifier lookup produced results, return them
        // (dedup phases will handle overlaps).
        if !candidates.is_empty() {
            return candidates;
        }

        // Title+author search fallback — only providers that support `Search`.
        if query.title.is_some() {
            debug!("no identifier results, falling back to title+author search");
            let search_provs = filter_providers(
                providers,
                |p| p.capabilities().has_feature(ProviderFeature::Search),
                "skipping search (not supported)",
            );
            let mut search_candidates = query_providers_search(&search_provs, query).await;
            filter_unsupported_candidates(&mut search_candidates);
            search_candidates
        } else {
            debug!("no identifiers and no title in query, cannot search");
            Vec::new()
        }
    }
}

/// Filter providers by a capability predicate, logging skipped providers.
fn filter_providers(
    providers: &[Arc<dyn MetadataProvider>],
    predicate: impl Fn(&dyn MetadataProvider) -> bool,
    skip_reason: &str,
) -> Vec<Arc<dyn MetadataProvider>> {
    providers
        .iter()
        .filter(|p| {
            if predicate(p.as_ref()) {
                true
            } else {
                debug!(provider = %p.name(), "{skip_reason}");
                false
            }
        })
        .cloned()
        .collect()
}

/// Kind of identifier lookup to dispatch.
#[derive(Debug, Clone, Copy)]
enum IdentifierLookup {
    Isbn,
    Asin,
}

/// Query all providers by identifier concurrently, returning scored candidates.
async fn query_providers_by_id(
    providers: &[Arc<dyn MetadataProvider>],
    id: &str,
    lookup: IdentifierLookup,
) -> Vec<ScoredCandidate> {
    let futs: Vec<_> = providers
        .iter()
        .map(|p| {
            let provider = Arc::clone(p);
            let id = id.to_string();
            async move {
                let name = provider.name().to_string();
                let result = match lookup {
                    IdentifierLookup::Isbn => provider.lookup_isbn(&id).await,
                    IdentifierLookup::Asin => provider.lookup_asin(&id).await,
                };
                let label = match lookup {
                    IdentifierLookup::Isbn => "ISBN",
                    IdentifierLookup::Asin => "ASIN",
                };
                match result {
                    Ok(results) => {
                        debug!(provider = %name, count = results.len(), "{label} lookup results");
                        results
                            .into_iter()
                            .map(|m| (name.clone(), m))
                            .collect::<Vec<_>>()
                    }
                    Err(e) => {
                        warn!(provider = %name, error = %e, "{label} lookup failed");
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
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
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
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
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
        decision: ResolverDecision::NoCandidates,
    }
}

// ── Scoring helpers ─────────────────────────────────────────────────

/// Populated-field statistics for a candidate's metadata.
///
/// Single source of truth for which fields count toward "data richness",
/// so the scoring bonus and sort tiebreaker stay in sync even as
/// `ProviderMetadata` evolves.
struct FieldCounts {
    present: usize,
    total: usize,
}

impl FieldCounts {
    fn of(metadata: &ProviderMetadata) -> Self {
        let fields: &[bool] = &[
            metadata.subtitle.is_some(),
            !metadata.authors.is_empty(),
            metadata.description.is_some(),
            metadata.publisher.is_some(),
            metadata.publication_year.is_some(),
            metadata.cover_url.is_some(),
            !metadata.subjects.is_empty(),
            metadata.series.is_some(),
            metadata.page_count.is_some(),
            metadata.language.is_some(),
        ];
        let present = fields.iter().filter(|&&f| f).count();
        Self {
            present,
            total: fields.len(),
        }
    }

    /// Completeness as a 0.0–1.0 ratio.
    #[allow(clippy::cast_precision_loss)]
    fn ratio(&self) -> f32 {
        self.present as f32 / self.total as f32
    }
}

/// Compare candidates by score descending, then by field count (richer first).
fn cmp_candidates(a: &ScoredCandidate, b: &ScoredCandidate) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.field_count.cmp(&a.field_count))
}

/// Score an individual candidate based on the query and existing metadata.
fn score_candidate(
    candidate: &mut ScoredCandidate,
    query: &MetadataQuery,
    existing: Option<&ExistingBookMetadata>,
) {
    let mut score = candidate.metadata.confidence;
    let mut reasons = Vec::new();
    let mut signals = HashSet::new();

    // ── ISBN exact match bonus ──
    if let Some(query_isbn) = &query.isbn {
        if candidate_has_isbn(&candidate.metadata, query_isbn) {
            score += ISBN_MATCH_BONUS;
            reasons.push("ISBN exact match".to_string());
            signals.insert(MatchSignal::IsbnMatch);
        }
    }

    // ── ASIN exact match bonus ──
    if let Some(query_asin) = &query.asin {
        if candidate_has_asin(&candidate.metadata, query_asin) {
            score += ASIN_MATCH_BONUS;
            reasons.push("ASIN exact match".to_string());
            signals.insert(MatchSignal::AsinMatch);
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
                signals.insert(MatchSignal::TitleMatch);
            } else {
                // Contradiction: candidate title very different from existing.
                score -= CONTRADICTION_PENALTY;
                reasons.push(format!(
                    "Title contradiction ({:.0}% similarity)",
                    title_sim * 100.0
                ));
                signals.insert(MatchSignal::Contradiction);
            }
        }

        // ── Author match bonus ──
        if !existing.authors.is_empty() {
            let candidate_authors: Vec<String> = candidate
                .metadata
                .authors
                .iter()
                .filter(|a| matches!(a.role.as_deref(), None | Some("author")))
                .map(|a| a.name.clone())
                .collect();

            if !candidate_authors.is_empty() {
                let author_sim =
                    similarity::author_similarity(&existing.authors, &candidate_authors);
                let bonus = author_sim * AUTHOR_MATCH_MAX_BONUS;
                score += bonus;
                if author_sim > 0.5 {
                    reasons.push(format!("Author match ({:.0}%)", author_sim * 100.0));
                    signals.insert(MatchSignal::AuthorMatch);
                }
            }
        }

        // ── Publisher match bonus ──
        if let (Some(existing_pub), Some(candidate_pub)) =
            (&existing.publisher, &candidate.metadata.publisher)
        {
            let pub_sim = similarity::similarity(existing_pub, candidate_pub);
            let bonus = pub_sim * PUBLISHER_MATCH_BONUS;
            score += bonus;
            if pub_sim > 0.5 {
                reasons.push(format!("Publisher match ({:.0}%)", pub_sim * 100.0));
                signals.insert(MatchSignal::PublisherMatch);
            }
        }
    }

    // ── Field completeness bonus ──
    // Reward candidates that have more metadata fields populated.
    let counts = FieldCounts::of(&candidate.metadata);
    candidate.field_count = counts.present;
    score += counts.ratio() * FIELD_COMPLETENESS_MAX_BONUS;

    // Clamp to 0.0-1.0.
    score = score.clamp(0.0, 1.0);

    candidate.score = score;
    candidate.match_reasons = reasons;
    candidate.signals = signals;
}

/// Check whether a candidate's identifiers contain the given ISBN.
fn candidate_has_isbn(metadata: &ProviderMetadata, isbn: &str) -> bool {
    let normalized_query = normalize_isbn(isbn);
    metadata.identifiers.iter().any(|id| {
        matches!(
            id.identifier_type,
            IdentifierType::Isbn13 | IdentifierType::Isbn10
        ) && normalize_isbn(&id.value) == normalized_query
    })
}

/// Check whether a candidate's identifiers contain the given ASIN.
fn candidate_has_asin(metadata: &ProviderMetadata, asin: &str) -> bool {
    let normalized_query = normalize_asin(asin);
    metadata.identifiers.iter().any(|id| {
        id.identifier_type == IdentifierType::Asin && normalize_asin(&id.value) == normalized_query
    })
}

/// Deduplicate candidates and apply cross-provider corroboration bonuses.
///
/// Three-phase approach:
/// 1. Same-provider strong dedup: full merge, no bonus.
/// 2. Cross-provider strong merge: per-ISBN groups, at-most-one-merge-per-candidate.
/// 3. Cross-provider fuzzy corroboration: no merge, score bonus only.
#[allow(clippy::too_many_lines)]
fn deduplicate_and_boost(candidates: &mut Vec<ScoredCandidate>) {
    if candidates.len() < 2 {
        return;
    }

    // ── Phase 1: Same-provider strong dedup ──
    // Full merge via existing merge_metadata(). Loser removed. No score bonus.
    {
        let len = candidates.len();
        let mut consumed: Vec<bool> = vec![false; len];

        for i in 0..len {
            if consumed[i] {
                continue;
            }
            for j in (i + 1)..len {
                if consumed[j] {
                    continue;
                }
                if candidates[i].provider_name != candidates[j].provider_name {
                    continue;
                }
                if book_match_strength(&candidates[i], &candidates[j]) != BookMatch::Strong {
                    continue;
                }

                debug!(
                    provider = %candidates[i].provider_name,
                    "same-provider duplicate found"
                );

                let (winner_idx, loser_idx) = if candidates[i].score >= candidates[j].score {
                    (i, j)
                } else {
                    (j, i)
                };

                let loser_metadata = candidates[loser_idx].metadata.clone();
                merge_metadata(&mut candidates[winner_idx].metadata, &loser_metadata);
                candidates[winner_idx]
                    .match_reasons
                    .push("Same-provider duplicate merged".to_string());
                consumed[loser_idx] = true;
            }
        }

        let mut idx = 0;
        candidates.retain(|_| {
            let keep = !consumed[idx];
            idx += 1;
            keep
        });
    }

    if candidates.len() < 2 {
        return;
    }

    // ── Phase 2: Cross-provider strong merge (per-ISBN groups) ──
    {
        // Collect all unique normalized ISBNs across all candidates.
        let mut all_isbns: Vec<String> = Vec::new();
        for c in candidates.iter() {
            for id in &c.metadata.identifiers {
                if matches!(
                    id.identifier_type,
                    IdentifierType::Isbn13 | IdentifierType::Isbn10
                ) {
                    let norm = normalize_isbn(&id.value);
                    if !all_isbns.contains(&norm) {
                        all_isbns.push(norm);
                    }
                }
            }
        }
        all_isbns.sort();

        // Track which candidates have been claimed (already participated in merge).
        let mut claimed: Vec<bool> = vec![false; candidates.len()];
        let mut consumed: Vec<bool> = vec![false; candidates.len()];

        for isbn in &all_isbns {
            // Find all unclaimed candidates from different providers that have this ISBN.
            let mut group: Vec<usize> = Vec::new();
            let mut providers_seen = HashSet::new();
            for (idx, c) in candidates.iter().enumerate() {
                if claimed[idx] {
                    continue;
                }
                let has_this_isbn = c.metadata.identifiers.iter().any(|id| {
                    matches!(
                        id.identifier_type,
                        IdentifierType::Isbn13 | IdentifierType::Isbn10
                    ) && normalize_isbn(&id.value) == *isbn
                });
                if has_this_isbn && !providers_seen.contains(&c.provider_name) {
                    providers_seen.insert(c.provider_name.clone());
                    group.push(idx);
                }
            }

            if group.len() < 2 {
                continue;
            }

            // Pick highest-scored as winner. Tie-break: cover_provider_rank (if both
            // have covers) → provider_name → original index.
            group.sort_by(|&a, &b| {
                let score_cmp = candidates[b]
                    .score
                    .partial_cmp(&candidates[a].score)
                    .unwrap_or(std::cmp::Ordering::Equal);
                if score_cmp != std::cmp::Ordering::Equal {
                    return score_cmp;
                }
                let a_has_cover = candidates[a].metadata.cover_url.is_some();
                let b_has_cover = candidates[b].metadata.cover_url.is_some();
                if a_has_cover && b_has_cover {
                    let rank_cmp = cover_provider_rank(&candidates[b].provider_name)
                        .cmp(&cover_provider_rank(&candidates[a].provider_name));
                    if rank_cmp != std::cmp::Ordering::Equal {
                        return rank_cmp;
                    }
                }
                let name_cmp = candidates[a]
                    .provider_name
                    .cmp(&candidates[b].provider_name);
                if name_cmp != std::cmp::Ordering::Equal {
                    return name_cmp;
                }
                a.cmp(&b)
            });

            let winner_idx = group[0];
            let loser_indices: Vec<usize> = group[1..].to_vec();

            debug!(
                winner = %candidates[winner_idx].provider_name,
                losers = ?loser_indices.iter().map(|&i| &candidates[i].provider_name).collect::<Vec<_>>(),
                isbn = %isbn,
                "cross-provider ISBN merge"
            );

            // Merge losers into winner in deterministic order (already sorted).
            for &loser_idx in &loser_indices {
                let loser_name = candidates[loser_idx].provider_name.clone();
                let loser_metadata = candidates[loser_idx].metadata.clone();
                let mut reasons = std::mem::take(&mut candidates[winner_idx].match_reasons);
                merge_metadata_cross_provider(
                    &mut candidates[winner_idx].metadata,
                    &loser_metadata,
                    &mut reasons,
                );
                reasons.push(format!("Merged with {loser_name} (shared ISBN)"));
                candidates[winner_idx].match_reasons = reasons;
                consumed[loser_idx] = true;
                claimed[loser_idx] = true;
            }

            // Apply bonus once to winner.
            candidates[winner_idx].score =
                (candidates[winner_idx].score + CROSS_PROVIDER_BONUS).min(1.0);
            candidates[winner_idx]
                .signals
                .insert(MatchSignal::CrossProvider);
            claimed[winner_idx] = true;
        }

        // Remove consumed losers.
        let mut idx = 0;
        candidates.retain(|_| {
            let keep = !consumed[idx];
            idx += 1;
            keep
        });
    }

    if candidates.len() < 2 {
        return;
    }

    // ── Phase 3: Cross-provider fuzzy corroboration ──
    // No merge, no removal. Bonus to corroborated candidates that don't already
    // have CrossProvider signal (prevents double-bonus from Phase 2).
    {
        let len = candidates.len();
        let mut corroborated: Vec<bool> = vec![false; len];

        // Scan all pairs for cross-provider fuzzy matches only.
        // Strong matches were already handled in Phase 2; any leftover strong
        // pairs (from claiming) are not corroborated here to keep phase
        // semantics clean.
        for i in 0..len {
            for j in (i + 1)..len {
                if candidates[i].provider_name == candidates[j].provider_name {
                    continue;
                }
                if book_match_strength(&candidates[i], &candidates[j]) == BookMatch::Fuzzy {
                    corroborated[i] = true;
                    corroborated[j] = true;
                }
            }
        }

        // Apply bonus only to newly corroborated (no CrossProvider signal yet).
        for i in 0..len {
            if corroborated[i] && !candidates[i].signals.contains(&MatchSignal::CrossProvider) {
                candidates[i].score = (candidates[i].score + CROSS_PROVIDER_BONUS).min(1.0);
                candidates[i].signals.insert(MatchSignal::CrossProvider);
                candidates[i]
                    .match_reasons
                    .push("Cross-provider corroboration".to_string());
            }
        }
    }
}

/// Determine the match strength between two candidates.
///
/// - `Strong`: ISBN match (cross-provider: ISBN only; same-provider: ISBN or same-type native ID).
/// - `Fuzzy`: title similarity >0.85 AND (both authorless OR author similarity >0.7).
/// - `No`: no match.
fn book_match_strength(a: &ScoredCandidate, b: &ScoredCandidate) -> BookMatch {
    let same_provider = a.provider_name == b.provider_name;

    // Check ISBN match.
    let has_isbn_match = a.metadata.identifiers.iter().any(|a_id| {
        if !matches!(
            a_id.identifier_type,
            IdentifierType::Isbn13 | IdentifierType::Isbn10
        ) {
            return false;
        }
        let a_norm = normalize_isbn(&a_id.value);
        b.metadata.identifiers.iter().any(|b_id| {
            matches!(
                b_id.identifier_type,
                IdentifierType::Isbn13 | IdentifierType::Isbn10
            ) && normalize_isbn(&b_id.value) == a_norm
        })
    });

    if has_isbn_match {
        return BookMatch::Strong;
    }

    // Same-provider: also check same-type provider-native ID match (OLID==OLID, HC-ID==HC-ID).
    if same_provider {
        let has_native_match = a.metadata.identifiers.iter().any(|a_id| {
            if matches!(
                a_id.identifier_type,
                IdentifierType::Isbn13 | IdentifierType::Isbn10
            ) {
                return false;
            }
            b.metadata.identifiers.iter().any(|b_id| {
                b_id.identifier_type == a_id.identifier_type && b_id.value == a_id.value
            })
        });
        if has_native_match {
            return BookMatch::Strong;
        }
    }

    // Fuzzy: title similarity >0.85 AND (both authorless OR author similarity >0.7).
    if let (Some(a_title), Some(b_title)) = (&a.metadata.title, &b.metadata.title) {
        let title_sim = similarity::similarity(a_title, b_title);
        if title_sim > 0.85 {
            let a_authors: Vec<String> =
                a.metadata.authors.iter().map(|a| a.name.clone()).collect();
            let b_authors: Vec<String> =
                b.metadata.authors.iter().map(|a| a.name.clone()).collect();
            if a_authors.is_empty() && b_authors.is_empty() {
                return BookMatch::Fuzzy;
            }
            let author_sim = similarity::author_similarity(&a_authors, &b_authors);
            if author_sim > 0.7 {
                return BookMatch::Fuzzy;
            }
        }
    }

    BookMatch::No
}

/// Fill empty/missing fields in `target` from `source`.
///
/// Covers the 9 text/scalar fields shared by both same-provider and
/// cross-provider merge paths. Does NOT touch cover, identifiers, or
/// subjects — those have merge-strategy-specific handling.
fn fill_metadata_gaps(target: &mut ProviderMetadata, source: &ProviderMetadata) {
    if target.subtitle.is_none() {
        target.subtitle.clone_from(&source.subtitle);
    }
    if target.description.is_none() {
        target.description.clone_from(&source.description);
    }
    if target.language.is_none() {
        target.language.clone_from(&source.language);
    }
    if target.publisher.is_none() {
        target.publisher.clone_from(&source.publisher);
    }
    if target.publication_year.is_none() {
        target.publication_year = source.publication_year;
    }
    if target.series.is_none() {
        target.series.clone_from(&source.series);
    }
    if target.page_count.is_none() {
        target.page_count = source.page_count;
    }
    if target.rating.is_none() {
        target.rating = source.rating;
    }
    if target.subjects.is_empty() {
        target.subjects.clone_from(&source.subjects);
    }
    if target.physical_format.is_none() {
        target.physical_format.clone_from(&source.physical_format);
    }
}

/// Merge unique data from `source` into `target`.
///
/// Only fills in fields that are `None` or empty in the target.
fn merge_metadata(target: &mut ProviderMetadata, source: &ProviderMetadata) {
    fill_metadata_gaps(target, source);

    if target.cover_url.is_none() {
        target.cover_url.clone_from(&source.cover_url);
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
/// Requires at least one content signal (title or author) plus one other
/// signal. This preserves the design doc intent: "ISBN alone without title
/// cross-validation should NOT auto-apply." `IsbnMatch` + `CrossProvider`
/// alone is insufficient — at least one of `TitleMatch`/`AuthorMatch` must
/// be present.
///
/// Note: production auto-apply now uses [`CandidateMatchTier`] instead of
/// this function directly. Kept for test assertions.
#[cfg(test)]
fn has_multi_signal(candidate: &ScoredCandidate) -> bool {
    let has_content = candidate.signals.contains(&MatchSignal::TitleMatch)
        || candidate.signals.contains(&MatchSignal::AuthorMatch);
    has_content && candidate.signals.len() >= 2
}

/// Check whether the candidate has an exact trusted identifier match against
/// the book's trusted identifiers.
///
/// Requires at least one trusted existing ID (isbn13, isbn10, asin from
/// user/embedded sources) to match a candidate ID. ISBN-10 ↔ ISBN-13
/// cross-matching is supported via conversion.
fn has_trusted_id_proof(
    existing: Option<&ExistingBookMetadata>,
    candidate: &ScoredCandidate,
) -> bool {
    let Some(existing) = existing else {
        return false;
    };

    for trusted_id in &existing.trusted_identifiers {
        let trusted_type = trusted_id.identifier_type;

        for cand_id in &candidate.metadata.identifiers {
            // Same-type exact match (normalized).
            if cand_id.identifier_type == trusted_type {
                let match_found = match trusted_type {
                    IdentifierType::Isbn13 | IdentifierType::Isbn10 => {
                        normalize_isbn(&trusted_id.value) == normalize_isbn(&cand_id.value)
                    }
                    IdentifierType::Asin => {
                        normalize_asin(&trusted_id.value) == normalize_asin(&cand_id.value)
                    }
                    _ => false,
                };
                if match_found {
                    return true;
                }
            }

            // Cross-type ISBN matching: ISBN-10 ↔ ISBN-13.
            if matches!(
                (trusted_type, cand_id.identifier_type),
                (IdentifierType::Isbn10, IdentifierType::Isbn13)
                    | (IdentifierType::Isbn13, IdentifierType::Isbn10)
            ) {
                let trusted_as_13 = to_isbn13(&trusted_id.value, trusted_type);
                let cand_as_13 = to_isbn13(&cand_id.value, cand_id.identifier_type);
                if let (Some(t), Some(c)) = (trusted_as_13, cand_as_13) {
                    if t == c {
                        return true;
                    }
                }
            }
        }
    }

    false
}

// ── Tier assignment ─────────────────────────────────────────────────

/// Assign a [`CandidateMatchTier`] based on the candidate's signals and
/// whether trusted identifier proof exists between the book and candidate.
///
/// Criteria:
/// - **`StrongIdMatch`**: hard ID signal (ISBN/ASIN) + content signal
///   (title/author) + trusted identifier proof.
/// - **`ProbableMatch`**: hard ID signal without content corroboration,
///   OR content signals with cross-provider corroboration.
/// - **`WeakMatch`**: everything else (content-only, no ID proof).
fn assign_tier(
    candidate: &ScoredCandidate,
    existing: Option<&ExistingBookMetadata>,
) -> CandidateMatchTier {
    let has_id = candidate.signals.contains(&MatchSignal::IsbnMatch)
        || candidate.signals.contains(&MatchSignal::AsinMatch);
    let has_content = candidate.signals.contains(&MatchSignal::TitleMatch)
        || candidate.signals.contains(&MatchSignal::AuthorMatch);
    let has_proof = has_trusted_id_proof(existing, candidate);

    if has_id && has_content && has_proof {
        CandidateMatchTier::StrongIdMatch
    } else if has_id || (has_content && candidate.signals.contains(&MatchSignal::CrossProvider)) {
        CandidateMatchTier::ProbableMatch
    } else {
        CandidateMatchTier::WeakMatch
    }
}

// ── Cross-provider merge helpers ────────────────────────────────────

/// Provider priority for cover selection during cross-provider merge.
const COVER_PROVIDER_PRIORITY: &[&str] = &[provider_names::OPEN_LIBRARY, provider_names::HARDCOVER];

/// Return a rank for cover provider priority (higher = better).
/// Unknown providers get 0.
fn cover_provider_rank(provider: &str) -> usize {
    COVER_PROVIDER_PRIORITY
        .iter()
        .position(|&p| p == provider)
        .map_or(0, |pos| pos + 1)
}

/// Whether an identifier type is portable across providers (i.e. not provider-native).
fn is_portable_identifier(id_type: IdentifierType) -> bool {
    matches!(id_type, IdentifierType::Isbn13 | IdentifierType::Isbn10)
}

/// Merge metadata across providers. Unlike `merge_metadata()` (same-provider),
/// this restricts identifiers to portable types only (ISBNs) and uses
/// provider-priority cover selection.
fn merge_metadata_cross_provider(
    target: &mut ProviderMetadata,
    source: &ProviderMetadata,
    match_reasons: &mut Vec<String>,
) {
    fill_metadata_gaps(target, source);

    // ── Cover: provider-priority selection ──
    let target_has_cover = target.cover_url.is_some();
    let source_has_cover = source.cover_url.is_some();
    match (target_has_cover, source_has_cover) {
        (false, true) => {
            target.cover_url.clone_from(&source.cover_url);
            match_reasons.push(format!("Cover from {}", source.provider_name));
        }
        (true, true) => {
            let target_rank = cover_provider_rank(&target.provider_name);
            let source_rank = cover_provider_rank(&source.provider_name);
            if source_rank > target_rank {
                target.cover_url.clone_from(&source.cover_url);
                match_reasons.push(format!("Cover from {}", source.provider_name));
            }
        }
        _ => {} // target has cover and source doesn't, or neither has cover
    }

    // ── Identifiers: portable only, with ISBN normalization ──
    for source_id in &source.identifiers {
        if !is_portable_identifier(source_id.identifier_type) {
            continue;
        }
        let source_norm = normalize_isbn(&source_id.value);
        let already_has = target.identifiers.iter().any(|t_id| {
            is_portable_identifier(t_id.identifier_type)
                && normalize_isbn(&t_id.value) == source_norm
        });
        if !already_has {
            target.identifiers.push(source_id.clone());
        }
    }

    // Remove non-portable identifiers from the *source* provider only.
    // The winner keeps its own native IDs; the loser's native IDs are dropped.
    let source_provider = &source.provider_name;
    target.identifiers.retain(|id| {
        is_portable_identifier(id.identifier_type) || {
            // Non-portable ID: keep only if it doesn't belong to the source provider.
            // We identify source-origin IDs by matching the provider-specific identifier
            // type (e.g., Hardcover IDs from the hardcover provider).
            !identifier_type_belongs_to_provider(id.identifier_type, source_provider)
        }
    });
}

/// Check whether a non-portable identifier type is native to a given provider.
fn identifier_type_belongs_to_provider(id_type: IdentifierType, provider: &str) -> bool {
    match id_type {
        IdentifierType::OpenLibrary => provider == provider_names::OPEN_LIBRARY,
        IdentifierType::Hardcover => provider == provider_names::HARDCOVER,
        IdentifierType::GoogleBooks => provider == provider_names::GOOGLE_BOOKS,
        IdentifierType::Lccn => provider == provider_names::LOC,
        // Portable types don't belong to any single provider.
        IdentifierType::Isbn13 | IdentifierType::Isbn10 | IdentifierType::Asin => false,
    }
}

/// Check whether a `physical_format` value indicates an unsupported media type
/// (audiobook, sound recording, video recording, etc.).
pub(crate) fn is_unsupported_format(format: &str) -> bool {
    let lower = format.to_lowercase();
    lower.contains("audio")
        || lower.contains("sound recording")
        || lower.contains("videorecording")
        || lower.contains("mp3")
        || lower.contains("listened")
        || lower.contains("watched")
}

/// Drop candidates whose `physical_format` matches an unsupported media type.
fn filter_unsupported_candidates(candidates: &mut Vec<ScoredCandidate>) {
    let pre_filter = candidates.len();
    candidates.retain(|c| match c.metadata.physical_format.as_deref() {
        Some(fmt) if is_unsupported_format(fmt) => {
            debug!(
                provider = %c.provider_name,
                physical_format = fmt,
                title = ?c.metadata.title,
                "dropping candidate with unsupported format"
            );
            false
        }
        _ => true,
    });
    if candidates.len() < pre_filter {
        debug!(
            dropped = pre_filter - candidates.len(),
            remaining = candidates.len(),
            "filtered unsupported-format candidates"
        );
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::errors::ProviderError;
    use crate::provider::MetadataProvider;
    use crate::test_util::StubSettings;
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
            subtitle: None,
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
            publication_year: None,
            identifiers,
            subjects: Vec::new(),
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence,
        }
    }

    #[test]
    fn auto_apply_threshold_reads_canonical_setting_key() {
        let registry = Arc::new(ProviderRegistry::new());
        let resolver = MetadataResolver::new(
            registry,
            Arc::new(StubSettings::new(vec![(
                "metadata.auto_apply_threshold",
                serde_json::json!(0.92),
            )])),
        );

        assert!((resolver.auto_apply_threshold() - 0.92).abs() < f32::EPSILON);
    }

    use crate::types::{ProviderCapabilities, ProviderFeature, ProviderQuality};

    /// Default "supports everything" capabilities for test stubs.
    static STUB_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Community,
        default_rate_limit_rpm: 100,
        supported_id_lookups: &[
            IdentifierType::Isbn13,
            IdentifierType::Isbn10,
            IdentifierType::Asin,
            IdentifierType::Lccn,
            IdentifierType::Hardcover,
        ],
        features: &[ProviderFeature::Search, ProviderFeature::Covers],
    };

    /// A configurable stub provider for testing the resolver.
    struct StubProvider {
        name: String,
        isbn_results: Vec<ProviderMetadata>,
        asin_results: Vec<ProviderMetadata>,
        search_results: Vec<ProviderMetadata>,
        capabilities: &'static ProviderCapabilities,
    }

    impl StubProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                isbn_results: Vec::new(),
                asin_results: Vec::new(),
                search_results: Vec::new(),
                capabilities: &STUB_CAPABILITIES,
            }
        }

        fn with_isbn_results(mut self, results: Vec<ProviderMetadata>) -> Self {
            self.isbn_results = results;
            self
        }

        fn with_asin_results(mut self, results: Vec<ProviderMetadata>) -> Self {
            self.asin_results = results;
            self
        }

        fn with_search_results(mut self, results: Vec<ProviderMetadata>) -> Self {
            self.search_results = results;
            self
        }

        fn with_capabilities(mut self, caps: &'static ProviderCapabilities) -> Self {
            self.capabilities = caps;
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

        fn capabilities(&self) -> &'static ProviderCapabilities {
            self.capabilities
        }

        async fn lookup_isbn(&self, _isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(self.isbn_results.clone())
        }

        async fn lookup_asin(&self, _asin: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(self.asin_results.clone())
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
            best.match_reasons.iter().any(|r| r.contains("Merged with")),
            "expected cross-provider merge reason, got: {:?}",
            best.match_reasons
        );
        assert!(
            best.signals.contains(&MatchSignal::CrossProvider),
            "expected CrossProvider signal"
        );
        // No loser-native identifiers after cross-provider merge.
        assert!(
            !best
                .metadata
                .identifiers
                .iter()
                .any(|id| identifier_type_belongs_to_provider(id.identifier_type, "hardcover")),
            "cross-provider merge should not keep loser native IDs, got: {:?}",
            best.metadata.identifiers
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
            trusted_identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
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
        // Use truly different books (different titles AND different authors)
        // so that dedup does not merge them.
        let ol = StubProvider::new("open_library").with_search_results(vec![
            make_metadata("open_library", "Dune", &["Frank Herbert"], None, 0.5),
            make_metadata("open_library", "Foundation", &["Isaac Asimov"], None, 0.8),
        ]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            title: Some("Foundation".to_string()),
            ..Default::default()
        };

        let result = resolver.resolve(&query, None).await;
        assert!(
            result.candidates.len() >= 2,
            "expected at least 2 candidates, got {}",
            result.candidates.len()
        );

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
    fn book_match_strength_strong_by_isbn() {
        let a = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], Some("9780441172719"), 0.9),
            score: 0.9,
            provider_name: "open_library".to_string(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        let b = ScoredCandidate {
            metadata: make_metadata("hc", "Dune", &["Frank Herbert"], Some("9780441172719"), 0.9),
            score: 0.9,
            provider_name: "hardcover".to_string(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        assert_eq!(book_match_strength(&a, &b), BookMatch::Strong);
    }

    #[test]
    fn book_match_strength_fuzzy_by_title_author() {
        let a = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.9),
            score: 0.9,
            provider_name: "open_library".to_string(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        let b = ScoredCandidate {
            metadata: make_metadata("hc", "Dune", &["Herbert, Frank"], None, 0.9),
            score: 0.9,
            provider_name: "hardcover".to_string(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        assert_eq!(book_match_strength(&a, &b), BookMatch::Fuzzy);
    }

    #[test]
    fn book_match_strength_no_different_titles() {
        let a = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.9),
            score: 0.9,
            provider_name: "open_library".to_string(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        let b = ScoredCandidate {
            metadata: make_metadata("hc", "Foundation", &["Isaac Asimov"], None, 0.9),
            score: 0.9,
            provider_name: "hardcover".to_string(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        assert_eq!(book_match_strength(&a, &b), BookMatch::No);
    }

    #[test]
    fn has_multi_signal_requires_two_signals() {
        let candidate = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &[], None, 0.95),
            score: 0.95,
            provider_name: "open_library".to_string(),
            match_reasons: vec!["ISBN exact match".to_string()],
            signals: HashSet::from([MatchSignal::IsbnMatch]),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
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
            signals: [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
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
            subtitle: None,
            authors: vec![],
            description: None,
            language: Some("en".to_string()),
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            subjects: Vec::new(),
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.9,
        };

        let source = ProviderMetadata {
            provider_name: "hardcover".to_string(),
            title: Some("Dune (different subtitle)".to_string()),
            subtitle: None,
            authors: vec![],
            description: Some("A sci-fi classic".to_string()),
            language: Some("fr".to_string()), // Different — should NOT overwrite.
            publisher: Some("Chilton Books".to_string()),
            publication_year: Some(1965),
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
            physical_format: None,
            confidence: 0.9,
        };

        merge_metadata(&mut target, &source);

        // Filled from source.
        assert_eq!(target.description.as_deref(), Some("A sci-fi classic"));
        assert_eq!(target.publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(target.publication_year, Some(1965));
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

    // ── Same-provider deduplication tests ──

    #[test]
    fn same_provider_duplicates_are_merged() {
        // Two candidates from the same provider with matching ISBN.
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.7,
                ),
                score: 0.7,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(
            candidates.len(),
            1,
            "same-provider duplicates should be merged into one candidate"
        );
    }

    #[test]
    fn same_provider_merge_keeps_higher_score_without_bonus() {
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.7,
                ),
                score: 0.7,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 1);
        let winner = &candidates[0];

        // Score should remain 0.9 — no cross-provider bonus applied.
        assert!(
            (winner.score - 0.9).abs() < f32::EPSILON,
            "same-provider merge should NOT apply corroboration bonus, got {}",
            winner.score
        );

        // Should have the same-provider merge reason, not cross-provider.
        assert!(
            winner
                .match_reasons
                .iter()
                .any(|r| r.contains("Same-provider duplicate merged")),
            "expected same-provider merge reason, got: {:?}",
            winner.match_reasons
        );
        assert!(
            !winner
                .match_reasons
                .iter()
                .any(|r| r.contains("Cross-provider")),
            "same-provider merge should NOT have cross-provider reason"
        );
    }

    #[test]
    fn cross_provider_strong_merge_gets_bonus() {
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "hardcover",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.85,
                ),
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 1);
        let winner = &candidates[0];

        // Score should be 0.9 + 0.1 = 1.0 (clamped).
        assert!(
            (winner.score - 1.0).abs() < f32::EPSILON,
            "cross-provider merge should apply bonus, got {}",
            winner.score
        );
        assert!(
            winner
                .match_reasons
                .iter()
                .any(|r| r.contains("Merged with")),
            "expected merge reason, got: {:?}",
            winner.match_reasons
        );
        assert!(
            winner.signals.contains(&MatchSignal::CrossProvider),
            "expected CrossProvider signal"
        );
        // No loser-native identifiers after cross-provider merge.
        assert!(
            !winner
                .metadata
                .identifiers
                .iter()
                .any(|id| identifier_type_belongs_to_provider(id.identifier_type, "hardcover")),
            "cross-provider merge should not keep loser native IDs"
        );
    }

    #[test]
    fn same_provider_merge_preserves_data_from_loser() {
        let mut high_score = make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.9,
        );
        high_score.description = None;
        high_score.publisher = None;

        let mut low_score = make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.7,
        );
        low_score.description = Some("A sci-fi classic".to_string());
        low_score.publisher = Some("Chilton Books".to_string());

        let mut candidates = vec![
            ScoredCandidate {
                metadata: high_score,
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: low_score,
                score: 0.7,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 1);
        let winner = &candidates[0];

        // Data from the lower-scored candidate should be merged in.
        assert_eq!(
            winner.metadata.description.as_deref(),
            Some("A sci-fi classic"),
            "description should be merged from loser"
        );
        assert_eq!(
            winner.metadata.publisher.as_deref(),
            Some("Chilton Books"),
            "publisher should be merged from loser"
        );
    }

    #[tokio::test]
    async fn same_provider_duplicates_deduped_in_full_resolve() {
        // One provider returns two results for the same book (e.g., ISBN
        // lookup and search both match). They should be deduplicated.
        let ol = StubProvider::new("open_library").with_isbn_results(vec![
            make_metadata(
                "open_library",
                "Dune",
                &["Frank Herbert"],
                Some("9780441172719"),
                0.95,
            ),
            make_metadata(
                "open_library",
                "Dune",
                &["Frank Herbert"],
                Some("9780441172719"),
                0.80,
            ),
        ]);

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

        assert_eq!(
            result.candidates.len(),
            1,
            "same-provider duplicates should be merged, got {} candidates",
            result.candidates.len()
        );

        // Should NOT have cross-provider reason.
        let best = result.best_match.as_ref().unwrap();
        assert!(
            !best
                .match_reasons
                .iter()
                .any(|r| r.contains("Cross-provider")),
            "same-provider merge should not claim cross-provider match"
        );
    }

    // ── Conservative auto-apply tests ──

    /// Helper to build a `ScoredCandidate` with signals for auto-apply tests.
    #[allow(clippy::too_many_arguments)]
    fn make_scored(
        provider: &str,
        title: &str,
        authors: &[&str],
        isbn13: Option<&str>,
        confidence: f32,
        score: f32,
        reasons: Vec<String>,
        signals: HashSet<MatchSignal>,
        tier: CandidateMatchTier,
    ) -> ScoredCandidate {
        ScoredCandidate {
            metadata: make_metadata(provider, title, authors, isbn13, confidence),
            score,
            provider_name: provider.to_string(),
            match_reasons: reasons,
            signals,
            tier,
            field_count: 0,
        }
    }

    #[test]
    fn no_auto_apply_when_two_candidates_above_threshold() {
        // Directly test should_auto_apply with two candidates above threshold.
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);
        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let second = make_scored(
            "open_library",
            "Foundation",
            &["Isaac Asimov"],
            Some("9780553293357"),
            0.90,
            0.88,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (85%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let candidates = vec![best.clone(), second];

        // Both are above 0.85 threshold -> should NOT auto-apply.
        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            !auto,
            "auto-apply should be false when multiple candidates exceed threshold"
        );
        assert_eq!(decision, ResolverDecision::BlockedAmbiguous);
    }

    #[tokio::test]
    async fn no_auto_apply_when_top_two_scores_close() {
        // Two candidates with scores within 0.1 of each other.
        // Use search results with moderately high confidence to get close scores.
        let ol = StubProvider::new("open_library").with_search_results(vec![
            make_metadata(
                "open_library",
                "Dune",
                &["Frank Herbert"],
                Some("9780441172719"),
                0.80,
            ),
            make_metadata(
                "open_library",
                "Dune Messiah",
                &["Frank Herbert"],
                Some("9780593098233"),
                0.78,
            ),
        ]);

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

        if result.candidates.len() >= 2 {
            let gap = result.candidates[0].score - result.candidates[1].score;
            if gap < 0.1 {
                assert!(
                    !result.auto_apply,
                    "auto-apply should be false when top two candidates are within 0.1 (gap={gap})"
                );
            }
        }
    }

    #[tokio::test]
    async fn auto_apply_with_single_dominant_candidate() {
        // One high-confidence candidate, one much lower — should auto-apply.
        let ol = StubProvider::new("open_library").with_isbn_results(vec![
            make_metadata(
                "open_library",
                "Dune",
                &["Frank Herbert"],
                Some("9780441172719"),
                0.95,
            ),
            make_metadata(
                "open_library",
                "Foundation",
                &["Isaac Asimov"],
                Some("9780553293357"),
                0.3,
            ),
        ]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            trusted_identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        // The best match should have ISBN + title signals and be well above threshold.
        let best = result.best_match.as_ref().unwrap();
        assert!(
            best.score >= 0.85,
            "expected best score >= 0.85, got {}",
            best.score
        );

        // If the gap to second-best is >= 0.15, auto-apply should be true.
        if result.candidates.len() >= 2 {
            let gap = result.candidates[0].score - result.candidates[1].score;
            if gap >= 0.15 {
                assert!(
                    result.auto_apply,
                    "auto-apply should be true with dominant candidate (gap={gap})"
                );
            }
        } else {
            // Only one candidate — should auto-apply if above threshold with multi-signal.
            assert!(
                result.auto_apply,
                "single dominant candidate should auto-apply"
            );
        }
    }

    #[test]
    fn should_auto_apply_unit_single_candidate_above_threshold() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);
        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let candidates = vec![best.clone()];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            auto,
            "single candidate above threshold with multi-signal and ID proof should auto-apply"
        );
        assert_eq!(decision, ResolverDecision::AutoApplyAllowed);
    }

    #[test]
    fn should_auto_apply_unit_two_above_threshold() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);
        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let second = make_scored(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780593098233"),
            0.90,
            0.90,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (90%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let candidates = vec![best.clone(), second];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            !auto,
            "should NOT auto-apply when two candidates are above threshold"
        );
        assert_eq!(decision, ResolverDecision::BlockedAmbiguous);
    }

    #[test]
    fn should_auto_apply_unit_close_scores() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);
        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.90,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let second = make_scored(
            "hardcover",
            "Foundation",
            &["Isaac Asimov"],
            None,
            0.5,
            0.82,
            vec![],
            HashSet::new(),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![best.clone(), second];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            !auto,
            "should NOT auto-apply when top two scores are within 0.1"
        );
        assert_eq!(decision, ResolverDecision::BlockedAmbiguous);
    }

    #[test]
    fn should_auto_apply_unit_gap_between_010_and_015() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);
        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.90,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let second = make_scored(
            "hardcover",
            "Foundation",
            &["Isaac Asimov"],
            None,
            0.5,
            0.78,
            vec![],
            HashSet::new(),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![best.clone(), second];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            !auto,
            "should NOT auto-apply when gap is between 0.10 and 0.15"
        );
        assert_eq!(decision, ResolverDecision::BlockedAmbiguous);
    }

    #[test]
    fn should_auto_apply_unit_large_gap() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);
        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let second = make_scored(
            "hardcover",
            "Foundation",
            &["Isaac Asimov"],
            None,
            0.5,
            0.50,
            vec![],
            HashSet::new(),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![best.clone(), second];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            auto,
            "should auto-apply when gap is well above 0.15 and only one above threshold"
        );
        assert_eq!(decision, ResolverDecision::AutoApplyAllowed);
    }

    #[test]
    fn should_auto_apply_unit_no_best_match() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let (auto, decision) = resolver.should_auto_apply(None, &[]);
        assert!(!auto, "should NOT auto-apply without a best match");
        assert_eq!(decision, ResolverDecision::NoCandidates);
    }

    #[test]
    fn should_auto_apply_unit_below_threshold() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            None,
            0.5,
            0.70,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let candidates = vec![best.clone()];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            !auto,
            "should NOT auto-apply when best score is below threshold"
        );
        assert_eq!(decision, ResolverDecision::BlockedLowTier);
    }

    #[test]
    fn should_auto_apply_unit_blocked_no_trusted_id() {
        // Candidate has an ID signal (ISBN match) but tier is ProbableMatch
        // because the book's trusted_identifiers don't include the ISBN.
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.92,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            // Tier stays ProbableMatch because trusted_identifiers didn't
            // contain the ISBN (e.g. ASIN-matched candidate with no matching
            // trusted ISBN).
            CandidateMatchTier::ProbableMatch,
        );
        let candidates = vec![best.clone()];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(!auto, "should NOT auto-apply without trusted ID proof");
        assert_eq!(
            decision,
            ResolverDecision::BlockedNoTrustedId,
            "ISBN signal present but tier too low → BlockedNoTrustedId"
        );
    }

    #[test]
    fn should_auto_apply_unit_blocked_contradiction() {
        // Candidate has a title contradiction signal and no ID signal,
        // so it cannot be StrongIdMatch.
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let best = make_scored(
            "open_library",
            "XYZ",
            &["Somebody Else"],
            None,
            0.7,
            0.63,
            vec!["Title contradiction (15% similarity)".to_string()],
            HashSet::from([MatchSignal::Contradiction]),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![best.clone()];

        let (auto, decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(
            !auto,
            "should NOT auto-apply when title contradiction exists without ID proof"
        );
        assert_eq!(
            decision,
            ResolverDecision::BlockedContradiction,
            "contradiction reason present without ID signal → BlockedContradiction"
        );
    }

    // ── New regression tests ──

    #[test]
    fn cross_provider_isbn_merge_restricts_identifiers() {
        // OL has ISBN + OLID, HC has ISBN + HC-ID.
        // After merge: winner (OL) keeps ISBN + OLID, loser (HC) native ID dropped.
        let mut ol_meta = make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.9,
        );
        ol_meta.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::OpenLibrary,
            value: "OL123456M".to_string(),
        });

        let mut hc_meta = make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.85,
        );
        hc_meta.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Hardcover,
            value: "hc-789".to_string(),
        });

        let mut candidates = vec![
            ScoredCandidate {
                metadata: ol_meta,
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: hc_meta,
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 1);
        let winner = &candidates[0];

        // Winner's native OLID should be kept.
        assert!(
            winner
                .metadata
                .identifiers
                .iter()
                .any(|id| id.identifier_type == IdentifierType::OpenLibrary),
            "winner's native OLID should be preserved, got: {:?}",
            winner.metadata.identifiers
        );
        // Loser's native HC-ID should be dropped.
        assert!(
            !winner
                .metadata
                .identifiers
                .iter()
                .any(|id| id.identifier_type == IdentifierType::Hardcover),
            "loser's native HC-ID should be removed, got: {:?}",
            winner.metadata.identifiers
        );
        // Should have ISBN + OLID = 2 identifiers.
        assert_eq!(
            winner.metadata.identifiers.len(),
            2,
            "expected ISBN + OLID, got: {:?}",
            winner.metadata.identifiers
        );
    }

    #[test]
    fn cross_provider_isbn_merge_prefers_higher_ranked_cover() {
        // OL has rank 1, HC has rank 2. HC cover should win when HC is loser
        // but has higher rank.
        let mut ol_meta = make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.9,
        );
        ol_meta.cover_url = Some("https://ol.example.com/cover.jpg".to_string());

        let mut hc_meta = make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.85,
        );
        hc_meta.cover_url = Some("https://hc.example.com/cover.jpg".to_string());

        let mut candidates = vec![
            ScoredCandidate {
                metadata: ol_meta,
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: hc_meta,
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 1);
        let winner = &candidates[0];

        // hardcover has higher rank (2) than open_library (1), so HC cover wins.
        assert_eq!(
            winner.metadata.cover_url.as_deref(),
            Some("https://hc.example.com/cover.jpg"),
            "higher-ranked provider cover should win"
        );
        assert!(
            winner
                .match_reasons
                .iter()
                .any(|r| r.contains("Cover from hardcover")),
            "expected cover provenance reason, got: {:?}",
            winner.match_reasons
        );
    }

    #[test]
    fn cross_provider_fuzzy_match_no_deep_merge() {
        // Different ISBNs, same title+author. Should NOT merge — both kept, both boosted.
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "hardcover",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9781234567890"),
                    0.85,
                ),
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(
            candidates.len(),
            2,
            "fuzzy cross-provider match should keep both candidates"
        );

        // Both should have CrossProvider signal and corroboration boost.
        for c in &candidates {
            assert!(
                c.signals.contains(&MatchSignal::CrossProvider),
                "expected CrossProvider signal on {}, got: {:?}",
                c.provider_name,
                c.signals
            );
            assert!(
                c.match_reasons
                    .iter()
                    .any(|r| r.contains("Cross-provider corroboration")),
                "expected corroboration reason on {}, got: {:?}",
                c.provider_name,
                c.match_reasons
            );
        }
    }

    #[test]
    fn cross_provider_isbn_merge_fills_text_gaps() {
        let mut ol_meta = make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.9,
        );
        ol_meta.description = None;
        ol_meta.publisher = None;

        let mut hc_meta = make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.85,
        );
        hc_meta.description = Some("A sci-fi masterpiece".to_string());
        hc_meta.publisher = Some("Chilton Books".to_string());

        let mut candidates = vec![
            ScoredCandidate {
                metadata: ol_meta,
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: hc_meta,
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 1);
        let winner = &candidates[0];

        assert_eq!(
            winner.metadata.description.as_deref(),
            Some("A sci-fi masterpiece"),
            "description should be filled from loser"
        );
        assert_eq!(
            winner.metadata.publisher.as_deref(),
            Some("Chilton Books"),
            "publisher should be filled from loser"
        );
    }

    // ── Determinism tests ──

    #[test]
    fn three_provider_fuzzy_corroboration_deterministic() {
        // Three providers, same title+author, different ISBNs. All kept, each boosted once.
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "hardcover",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9781234567890"),
                    0.85,
                ),
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "google_books",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9789876543210"),
                    0.80,
                ),
                score: 0.80,
                provider_name: "google_books".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 3, "all three should be kept");
        for c in &candidates {
            assert!(
                c.signals.contains(&MatchSignal::CrossProvider),
                "expected CrossProvider signal on {}",
                c.provider_name
            );
        }
    }

    #[test]
    fn transitive_fuzzy_corroboration() {
        // A~B fuzzy (0.91), B~C fuzzy (0.86), A!~C (0.71 < 0.85).
        // All three corroborated via pair-derived flags: A flagged by (A,B),
        // C flagged by (B,C), B flagged by both. Transitive outcome without
        // explicit component build.

        // Verify the similarity invariants that this test depends on.
        let ab = similarity::similarity("The Left Hand of Darkness", "The Left Hand of Destiny");
        let bc = similarity::similarity("The Left Hand of Destiny", "The Right Hand of Destiny");
        let ac = similarity::similarity("The Left Hand of Darkness", "The Right Hand of Destiny");
        assert!(ab > 0.85, "A~B should be >0.85, got {ab}");
        assert!(bc > 0.85, "B~C should be >0.85, got {bc}");
        assert!(ac <= 0.85, "A~C should be <=0.85, got {ac}");

        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "The Left Hand of Darkness",
                    &["Ursula K. Le Guin"],
                    Some("9780441478125"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                // B: similar title to both A and C (bridge).
                metadata: make_metadata(
                    "hardcover",
                    "The Left Hand of Destiny",
                    &["Ursula K. Le Guin"],
                    Some("9781234567890"),
                    0.85,
                ),
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                // C: similar to B but NOT directly similar to A (title sim 0.71).
                metadata: make_metadata(
                    "google_books",
                    "The Right Hand of Destiny",
                    &["Ursula K. Le Guin"],
                    Some("9789876543210"),
                    0.80,
                ),
                score: 0.80,
                provider_name: "google_books".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 3, "all three should be kept");
        for c in &candidates {
            assert!(
                c.signals.contains(&MatchSignal::CrossProvider),
                "{} should be corroborated",
                c.provider_name
            );
        }
    }

    #[test]
    fn mixed_strong_and_fuzzy_no_bleed() {
        // A+B ISBN merge (strong), C fuzzy-matches only B.
        // B consumed → C has no remaining fuzzy partner → C kept without boost.
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "hardcover",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.85,
                ),
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                // C: different ISBN, different title — no fuzzy match with A.
                metadata: make_metadata(
                    "google_books",
                    "Foundation",
                    &["Isaac Asimov"],
                    Some("9780553293357"),
                    0.80,
                ),
                score: 0.80,
                provider_name: "google_books".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        // A+B merged, C stays.
        assert_eq!(candidates.len(), 2);

        // C should NOT have CrossProvider (no fuzzy partner after B consumed).
        let c = candidates
            .iter()
            .find(|c| c.provider_name == "google_books");
        assert!(c.is_some(), "C should remain in candidates");
        assert!(
            !c.unwrap().signals.contains(&MatchSignal::CrossProvider),
            "C should not be corroborated — B was consumed"
        );
    }

    #[test]
    fn same_provider_fuzzy_no_merge() {
        // Same provider, same title, different ISBNs. Both kept, no bonus.
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.9,
                ),
                score: 0.9,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9781234567890"),
                    0.85,
                ),
                score: 0.85,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(candidates.len(), 2, "same-provider fuzzy should keep both");
        for c in &candidates {
            assert!(
                !c.signals.contains(&MatchSignal::CrossProvider),
                "same-provider fuzzy should not get CrossProvider signal"
            );
        }
    }

    #[test]
    fn has_multi_signal_requires_content_signal() {
        // IsbnMatch + CrossProvider alone → false.
        let candidate_isbn_cp = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &[], None, 0.95),
            score: 0.95,
            provider_name: "open_library".to_string(),
            match_reasons: vec![],
            signals: [MatchSignal::IsbnMatch, MatchSignal::CrossProvider]
                .into_iter()
                .collect(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        assert!(
            !has_multi_signal(&candidate_isbn_cp),
            "IsbnMatch + CrossProvider without content signal should fail"
        );

        // IsbnMatch + TitleMatch → true.
        let candidate_isbn_title = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &[], None, 0.95),
            score: 0.95,
            provider_name: "open_library".to_string(),
            match_reasons: vec![],
            signals: [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        assert!(
            has_multi_signal(&candidate_isbn_title),
            "IsbnMatch + TitleMatch should pass"
        );
    }

    #[test]
    fn phase2_winner_corroborates_without_double_bonus() {
        // A+B ISBN merge (Phase 2, A wins +0.1). C fuzzy-matches A in Phase 3.
        // C gets +0.1. A already has CrossProvider → no second bonus.
        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.8,
                ),
                score: 0.8,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "hardcover",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"),
                    0.75,
                ),
                score: 0.75,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                // C: same title/author, different ISBN → fuzzy match with A.
                metadata: make_metadata(
                    "google_books",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9789876543210"),
                    0.70,
                ),
                score: 0.70,
                provider_name: "google_books".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        // A+B merged → 2 candidates remain.
        assert_eq!(candidates.len(), 2);

        let a = candidates
            .iter()
            .find(|c| c.provider_name == "open_library")
            .unwrap();
        let c = candidates
            .iter()
            .find(|c| c.provider_name == "google_books")
            .unwrap();

        // A: 0.8 + 0.1 (Phase 2) = 0.9. No double bonus.
        assert!(
            (a.score - 0.9).abs() < f32::EPSILON,
            "A should have score 0.9 (no double bonus), got {}",
            a.score
        );

        // C: 0.70 + 0.1 (Phase 3 corroboration) = 0.80.
        assert!(
            (c.score - 0.80).abs() < f32::EPSILON,
            "C should have score 0.80 (one bonus), got {}",
            c.score
        );
        assert!(c.signals.contains(&MatchSignal::CrossProvider));
    }

    #[test]
    fn isbn_only_query_no_auto_apply_with_cross_provider() {
        // Book with ISBN but no existing title/author metadata.
        // Single merged candidate has IsbnMatch + CrossProvider but no TitleMatch/AuthorMatch.
        // has_multi_signal() should return false → auto_apply is false.
        let candidate = ScoredCandidate {
            metadata: make_metadata(
                "open_library",
                "Dune",
                &["Frank Herbert"],
                Some("9780441172719"),
                0.95,
            ),
            score: 0.95,
            provider_name: "open_library".to_string(),
            match_reasons: vec![],
            signals: [MatchSignal::IsbnMatch, MatchSignal::CrossProvider]
                .into_iter()
                .collect(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        assert!(
            !has_multi_signal(&candidate),
            "ISBN + CrossProvider without content signal should not auto-apply"
        );
    }

    #[test]
    fn strong_bridge_no_transitive_merge_winner_not_bridge() {
        // A (score 0.95, ISBN X) ↔ B (score 0.85, ISBN X+Y) ↔ C (score 0.80, ISBN Y).
        // A !↔ C. ISBN X group processed first: A wins, B merged (claimed).
        // ISBN Y group: B claimed → C has no partner → stays separate.
        // Result: 2 candidates (merged A, standalone C).
        let mut b_meta = make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"), // ISBN X
            0.85,
        );
        b_meta.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9781234567890".to_string(), // ISBN Y
        });

        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"), // ISBN X only
                    0.95,
                ),
                score: 0.95,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: b_meta,
                score: 0.85,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "google_books",
                    "Dune Deluxe",
                    &["Frank Herbert"],
                    Some("9781234567890"), // ISBN Y only
                    0.80,
                ),
                score: 0.80,
                provider_name: "google_books".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(
            candidates.len(),
            2,
            "expected 2 candidates: merged A and standalone C"
        );

        let a = candidates
            .iter()
            .find(|c| c.provider_name == "open_library");
        let c = candidates
            .iter()
            .find(|c| c.provider_name == "google_books");

        assert!(a.is_some(), "A (open_library) should remain as winner");
        assert!(c.is_some(), "C (google_books) should remain standalone");
    }

    #[test]
    fn strong_bridge_no_transitive_merge_bridge_is_winner() {
        // B (score 0.95, ISBN X+Y) ↔ A (score 0.85, ISBN X) and B ↔ C (score 0.80, ISBN Y).
        // ISBN X processed first (lexicographic): B wins, A merged. B claimed.
        // ISBN Y: B claimed → C separate.
        // Result: 2 candidates (merged B, standalone C). B does NOT absorb both sides.
        let mut b_meta = make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"), // ISBN X
            0.95,
        );
        b_meta.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9781234567890".to_string(), // ISBN Y
        });

        let mut candidates = vec![
            ScoredCandidate {
                metadata: make_metadata(
                    "open_library",
                    "Dune",
                    &["Frank Herbert"],
                    Some("9780441172719"), // ISBN X only
                    0.85,
                ),
                score: 0.85,
                provider_name: "open_library".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: b_meta,
                score: 0.95,
                provider_name: "hardcover".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
            ScoredCandidate {
                metadata: make_metadata(
                    "google_books",
                    "Dune Deluxe",
                    &["Frank Herbert"],
                    Some("9781234567890"), // ISBN Y only
                    0.80,
                ),
                score: 0.80,
                provider_name: "google_books".to_string(),
                match_reasons: Vec::new(),
                signals: HashSet::new(),
                tier: CandidateMatchTier::WeakMatch,
                field_count: 0,
            },
        ];

        deduplicate_and_boost(&mut candidates);

        assert_eq!(
            candidates.len(),
            2,
            "expected 2 candidates: merged B and standalone C"
        );

        let b = candidates.iter().find(|c| c.provider_name == "hardcover");
        let c = candidates
            .iter()
            .find(|c| c.provider_name == "google_books");

        assert!(b.is_some(), "B (hardcover) should remain as winner");
        assert!(c.is_some(), "C (google_books) should remain standalone");

        // B should have CrossProvider from merging A.
        assert!(
            b.unwrap().signals.contains(&MatchSignal::CrossProvider),
            "B should have CrossProvider signal"
        );
    }

    // ── Regression tests: resolution pipeline hardening ──────────────

    /// Regression: fuzzy title + author match alone must NOT trigger auto-apply.
    ///
    /// The `has_multi_signal` guard currently accepts {`TitleMatch`, `AuthorMatch`}
    /// as sufficient. These are both content-derived fuzzy signals — without a
    /// hard identifier proof (`IsbnMatch` or `CrossProvider`), auto-apply is unsafe.
    ///
    /// Invariant: "No auto-apply without exact trusted ID proof."
    #[test]
    fn fuzzy_title_author_match_without_id_proof_must_not_auto_apply() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        // Candidate scored via search: high title + author match, NO ISBN.
        // Signals will be {TitleMatch, AuthorMatch} — both fuzzy, no hard proof.
        let candidate = make_scored(
            "open_library",
            "Red Storm Rising: A Naval Thriller",
            &["Tom Clancy"],
            None, // No ISBN identifier
            0.7,
            0.93, // Score above threshold from fuzzy bonuses
            vec![
                "Title fuzzy match (89%)".to_string(),
                "Author match (100%)".to_string(),
            ],
            [MatchSignal::TitleMatch, MatchSignal::AuthorMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![candidate.clone()];

        // Precondition: score IS above threshold.
        assert!(
            candidate.score >= 0.85,
            "precondition: score should be above threshold, got {}",
            candidate.score
        );

        // INVARIANT: Without IsbnMatch or CrossProvider, auto-apply must be
        // suppressed even when score is high and title+author match well.
        let (auto, _decision) = resolver.should_auto_apply(Some(&candidate), &candidates);
        assert!(
            !auto,
            "fuzzy title+author match without ISBN/cross-provider proof \
             should NOT auto-apply (score: {}, signals: {:?})",
            candidate.score, candidate.signals
        );
    }

    /// Regression: full resolve with a single search-only candidate should
    /// NOT auto-apply when there is no ISBN in the query or the result.
    ///
    /// Reproduces the regression path: book has title + author, provider
    /// search returns a plausible match with high confidence, title and
    /// author similarity are high, but no ISBN ever enters the picture.
    #[tokio::test]
    async fn search_only_high_confidence_match_must_not_auto_apply() {
        // Provider returns a result via search (not ISBN lookup) with
        // high confidence and matching title+author.
        let provider = StubProvider::new("open_library").with_search_results(vec![make_metadata(
            "open_library",
            "Red Storm Rising",
            &["Tom Clancy"],
            None, // No ISBN in result
            0.75, // High provider confidence
        )]);

        let registry = make_registry(vec![Arc::new(provider)]);
        let resolver = MetadataResolver::with_defaults(registry);

        // Query has no ISBN — typical for ASIN-only or title-only books.
        let query = MetadataQuery {
            title: Some("Red Storm Rising".to_string()),
            author: Some("Tom Clancy".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Red Storm Rising".to_string()),
            authors: vec!["Tom Clancy".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        let best = result.best_match.as_ref().expect("should have a candidate");
        // Score: 0.75 base + title + author bonuses, then WeakMatch tier factor (0.65).
        assert!(
            best.score >= 0.50,
            "precondition: score should be reasonable, got {}",
            best.score
        );

        // INVARIANT: search-only result without ISBN proof must NOT auto-apply.
        assert!(
            !result.auto_apply,
            "search-only match without ISBN proof should NOT auto-apply \
             (score: {}, signals: {:?}, reasons: {:?})",
            best.score, best.signals, best.match_reasons
        );
    }

    /// Regression: when query has ASIN but no ISBN, the resolver must still
    /// find candidates via the ASIN lookup path.
    #[tokio::test]
    async fn asin_only_query_should_find_candidates_via_asin() {
        // Build a candidate that an ASIN-aware provider would return,
        // including the ASIN in its identifiers.
        let mut correct_book = make_metadata(
            "hardcover",
            "Mortal Arts",
            &["Anna Lee Huber"],
            Some("9780425253465"),
            0.95,
        );
        correct_book.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Asin,
            value: "B00BDQ399Y".to_string(),
        });

        let provider = StubProvider::new("hardcover")
            .with_asin_results(vec![correct_book])
            .with_search_results(vec![]); // Title search returns nothing

        let registry = make_registry(vec![Arc::new(provider)]);
        let resolver = MetadataResolver::with_defaults(registry);

        // Query has ASIN but no ISBN — common for Kindle-sourced ebooks.
        let query = MetadataQuery {
            asin: Some("B00BDQ399Y".to_string()),
            title: Some("Mortal Arts".to_string()),
            author: Some("Anna Lee Huber".to_string()),
            isbn: None,
        };

        let result = resolver.resolve(&query, None).await;

        assert!(
            !result.candidates.is_empty(),
            "resolver should find candidates via ASIN lookup"
        );

        let best = result.best_match.as_ref().unwrap();
        assert!(
            best.signals.contains(&MatchSignal::AsinMatch),
            "best candidate should have ASIN match signal"
        );
    }

    /// ASIN exact match should influence scoring analogous to ISBN match.
    #[tokio::test]
    async fn asin_exact_match_boosts_score() {
        let mut candidate = make_metadata(
            "hardcover",
            "The Martian",
            &["Andy Weir"],
            Some("9780553418026"),
            0.7,
        );
        candidate.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Asin,
            value: "B00EMXBDMA".to_string(),
        });

        let provider = StubProvider::new("hardcover").with_isbn_results(vec![candidate]);

        let registry = make_registry(vec![Arc::new(provider)]);
        let resolver = MetadataResolver::with_defaults(registry);

        // Query has both ISBN and ASIN.
        let query = MetadataQuery {
            isbn: Some("9780553418026".to_string()),
            asin: Some("B00EMXBDMA".to_string()),
            title: Some("The Martian".to_string()),
            ..Default::default()
        };

        let existing = ExistingBookMetadata {
            title: Some("The Martian".to_string()),
            authors: vec!["Andy Weir".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;
        let best = result.best_match.as_ref().expect("should have a candidate");

        // Base 0.7 + ISBN 0.2 + ASIN 0.2 + title/author bonuses → clamped to 1.0,
        // then tier factor applied (no trusted IDs → not StrongIdMatch).
        assert!(
            best.score > 0.60,
            "ASIN match should boost score: {}",
            best.score
        );
        assert!(
            best.signals.contains(&MatchSignal::AsinMatch),
            "should have ASIN match signal"
        );
        assert!(
            best.signals.contains(&MatchSignal::IsbnMatch),
            "should also have ISBN match signal"
        );
    }

    /// ASIN-only query falls back to title+author search when no ASIN
    /// lookup results are available.
    #[tokio::test]
    async fn asin_query_falls_back_to_search_when_no_asin_results() {
        let search_result = make_metadata(
            "hardcover",
            "Mortal Arts",
            &["Anna Lee Huber"],
            Some("9780425253465"),
            0.75,
        );

        // No ASIN results, but search finds the book by title.
        let provider = StubProvider::new("hardcover")
            .with_asin_results(vec![])
            .with_search_results(vec![search_result]);

        let registry = make_registry(vec![Arc::new(provider)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            asin: Some("B00BDQ399Y".to_string()),
            title: Some("Mortal Arts".to_string()),
            author: Some("Anna Lee Huber".to_string()),
            isbn: None,
        };

        let result = resolver.resolve(&query, None).await;

        assert!(
            !result.candidates.is_empty(),
            "should fall back to title+author search"
        );
    }

    /// When both ISBN and ASIN are present, both paths fire and results are
    /// merged. The higher-confidence ISBN result should be the best match.
    #[tokio::test]
    async fn both_isbn_and_asin_paths_fire_and_merge() {
        let isbn_result = make_metadata(
            "hardcover",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        );

        let mut asin_result =
            make_metadata("hardcover", "Dune (Kindle)", &["Frank Herbert"], None, 0.85);
        asin_result.identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Asin,
            value: "B00BDQ399Y".to_string(),
        });

        let provider = StubProvider::new("hardcover")
            .with_isbn_results(vec![isbn_result])
            .with_asin_results(vec![asin_result]);

        let registry = make_registry(vec![Arc::new(provider)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            asin: Some("B00BDQ399Y".to_string()),
            ..Default::default()
        };

        let result = resolver.resolve(&query, None).await;

        // Both paths should have produced candidates (may be deduped).
        assert!(
            !result.candidates.is_empty(),
            "should have candidates from merged ISBN+ASIN paths"
        );

        // Best match should be the higher-confidence ISBN result.
        let best = result
            .best_match
            .as_ref()
            .expect("should have a best match");
        assert_eq!(
            best.metadata.title.as_deref(),
            Some("Dune"),
            "ISBN result (higher confidence) should be best match"
        );
    }

    // ── Tier assignment tests ─────────────────────────────────────────

    #[test]
    fn tier_strong_id_match_isbn_title_trusted() {
        let candidate = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::WeakMatch, // will be overridden by assign_tier
        );
        let existing = ExistingBookMetadata {
            trusted_identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            ..Default::default()
        };

        let tier = assign_tier(&candidate, Some(&existing));
        assert_eq!(
            tier,
            CandidateMatchTier::StrongIdMatch,
            "ISBN + title + trusted proof = StrongIdMatch"
        );
    }

    #[test]
    fn tier_probable_match_isbn_only_no_content() {
        let candidate = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec!["ISBN exact match".to_string()],
            HashSet::from([MatchSignal::IsbnMatch]),
            CandidateMatchTier::WeakMatch,
        );
        let existing = ExistingBookMetadata {
            trusted_identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            ..Default::default()
        };

        let tier = assign_tier(&candidate, Some(&existing));
        assert_eq!(
            tier,
            CandidateMatchTier::ProbableMatch,
            "ISBN without content signal = ProbableMatch"
        );
    }

    #[test]
    fn tier_probable_match_content_plus_cross_provider() {
        let candidate = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            None,
            0.90,
            0.90,
            vec![
                "Title fuzzy match (100%)".to_string(),
                "Cross-provider corroboration".to_string(),
            ],
            [MatchSignal::TitleMatch, MatchSignal::CrossProvider]
                .into_iter()
                .collect(),
            CandidateMatchTier::WeakMatch,
        );

        let tier = assign_tier(&candidate, None);
        assert_eq!(
            tier,
            CandidateMatchTier::ProbableMatch,
            "content + cross-provider without ID = ProbableMatch"
        );
    }

    #[test]
    fn tier_weak_match_fuzzy_only() {
        let candidate = make_scored(
            "open_library",
            "Red Storm Rising",
            &["Tom Clancy"],
            None,
            0.75,
            0.93,
            vec![
                "Title fuzzy match (89%)".to_string(),
                "Author match (100%)".to_string(),
            ],
            [MatchSignal::TitleMatch, MatchSignal::AuthorMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::WeakMatch,
        );

        let tier = assign_tier(&candidate, None);
        assert_eq!(
            tier,
            CandidateMatchTier::WeakMatch,
            "fuzzy title + author without any ID = WeakMatch"
        );
    }

    #[test]
    fn tier_strong_id_requires_trusted_proof() {
        // ISBN + title signals but NO trusted proof → not StrongIdMatch.
        let candidate = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::WeakMatch,
        );
        // No trusted identifiers on the book.
        let existing = ExistingBookMetadata::default();

        let tier = assign_tier(&candidate, Some(&existing));
        assert_eq!(
            tier,
            CandidateMatchTier::ProbableMatch,
            "ISBN + title but no trusted proof = ProbableMatch"
        );
    }

    #[test]
    fn fuzzy_only_single_candidate_cannot_auto_apply_via_tier() {
        // Acceptance criterion: fuzzy-only single candidate cannot auto-apply.
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let candidate = make_scored(
            "open_library",
            "Red Storm Rising",
            &["Tom Clancy"],
            None,
            0.75,
            0.93,
            vec![
                "Title fuzzy match (89%)".to_string(),
                "Author match (100%)".to_string(),
            ],
            [MatchSignal::TitleMatch, MatchSignal::AuthorMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![candidate.clone()];

        let (auto, _decision) = resolver.should_auto_apply(Some(&candidate), &candidates);
        assert!(
            !auto,
            "WeakMatch tier cannot auto-apply regardless of score"
        );
    }

    #[test]
    fn strong_id_match_can_auto_apply_when_unambiguous() {
        // Acceptance criterion: strong exact-ID candidate can auto-apply if unambiguous.
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let best = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec![
                "ISBN exact match".to_string(),
                "Title fuzzy match (100%)".to_string(),
            ],
            [MatchSignal::IsbnMatch, MatchSignal::TitleMatch]
                .into_iter()
                .collect(),
            CandidateMatchTier::StrongIdMatch,
        );
        let second = make_scored(
            "hardcover",
            "Foundation",
            &["Isaac Asimov"],
            None,
            0.5,
            0.50,
            vec![],
            HashSet::new(),
            CandidateMatchTier::WeakMatch,
        );
        let candidates = vec![best.clone(), second];

        let (auto, _decision) = resolver.should_auto_apply(Some(&best), &candidates);
        assert!(auto, "StrongIdMatch with large gap should auto-apply");
    }

    #[test]
    fn probable_match_cannot_auto_apply() {
        let registry = make_registry(vec![]);
        let resolver = MetadataResolver::with_defaults(registry);

        let candidate = make_scored(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
            0.95,
            vec!["ISBN exact match".to_string()],
            HashSet::from([MatchSignal::IsbnMatch]),
            CandidateMatchTier::ProbableMatch,
        );
        let candidates = vec![candidate.clone()];

        let (auto, _decision) = resolver.should_auto_apply(Some(&candidate), &candidates);
        assert!(!auto, "ProbableMatch tier cannot auto-apply");
    }

    #[tokio::test]
    async fn full_resolve_assigns_tiers_correctly() {
        // ISBN lookup with matching title+author → StrongIdMatch.
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
            trusted_identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;
        let best = result.best_match.as_ref().unwrap();

        assert_eq!(
            best.tier,
            CandidateMatchTier::StrongIdMatch,
            "ISBN lookup with matching title/author + trusted proof = StrongIdMatch"
        );
        assert!(result.auto_apply, "StrongIdMatch should auto-apply");
    }

    #[tokio::test]
    async fn full_resolve_search_only_gets_weak_tier() {
        // Search-only result → WeakMatch.
        let ol = StubProvider::new("open_library").with_search_results(vec![make_metadata(
            "open_library",
            "Red Storm Rising",
            &["Tom Clancy"],
            None,
            0.75,
        )]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            title: Some("Red Storm Rising".to_string()),
            author: Some("Tom Clancy".to_string()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Red Storm Rising".to_string()),
            authors: vec!["Tom Clancy".to_string()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;
        let best = result.best_match.as_ref().unwrap();

        assert_eq!(
            best.tier,
            CandidateMatchTier::WeakMatch,
            "search-only result without ID = WeakMatch"
        );
        assert!(!result.auto_apply, "WeakMatch should not auto-apply");
    }

    // ── Content-scan evidence safety tests ──────────────────────────

    /// ISBN match + title match but NO trusted identifiers (scan-only
    /// scenario).  Must NOT reach `StrongIdMatch` or auto-apply.
    #[tokio::test]
    async fn scan_only_isbn_gets_probable_not_strong_id() {
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
        // Book has matching ISBN in `identifiers` (from scan) but NOT in
        // `trusted_identifiers` — simulates a content-scan-only book.
        let existing = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            trusted_identifiers: vec![], // no trusted proof
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;
        assert!(result.best_match.is_some());
        let best = result.best_match.as_ref().unwrap();

        // Has ISBN + title + author signals, but missing trusted ID proof →
        // cannot be StrongIdMatch.
        assert_eq!(
            best.tier,
            CandidateMatchTier::ProbableMatch,
            "scan-only ISBN must not reach StrongIdMatch tier; got {}",
            best.tier
        );
        assert!(
            !result.auto_apply,
            "scan-only evidence must never auto-apply"
        );
    }

    /// Noisy multi-ISBN scenario: book content yielded several ISBNs
    /// (bibliography/references).  One happens to match a provider result.
    /// Even with high score, must NOT auto-apply without trusted proof.
    #[tokio::test]
    async fn noisy_multi_isbn_scan_no_auto_apply() {
        // Provider returns a match for one of the noisy ISBNs.
        let ol = StubProvider::new("open_library").with_isbn_results(vec![make_metadata(
            "open_library",
            "Neuromancer",
            &["William Gibson"],
            Some("9780441569595"),
            0.95,
        )]);

        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441569595".to_string()),
            ..Default::default()
        };

        // Book's actual title is different from the provider result (the
        // matching ISBN was a reference, not the book's own ISBN).
        // No trusted identifiers — scan-only.
        let existing = ExistingBookMetadata {
            title: Some("Burning Chrome".to_string()),
            authors: vec!["William Gibson".to_string()],
            identifiers: vec![
                ProviderIdentifier {
                    identifier_type: IdentifierType::Isbn13,
                    value: "9780441569595".to_string(), // Neuromancer (reference)
                },
                ProviderIdentifier {
                    identifier_type: IdentifierType::Isbn13,
                    value: "9780060539825".to_string(), // another reference
                },
                ProviderIdentifier {
                    identifier_type: IdentifierType::Isbn13,
                    value: "9780441015085".to_string(), // yet another reference
                },
            ],
            trusted_identifiers: vec![],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;
        assert!(result.best_match.is_some());
        let best = result.best_match.as_ref().unwrap();

        // Title contradiction + no trusted proof → low tier and no auto-apply.
        assert_ne!(
            best.tier,
            CandidateMatchTier::StrongIdMatch,
            "noisy scan ISBNs must not produce StrongIdMatch"
        );
        assert!(
            !result.auto_apply,
            "noisy multi-ISBN scan evidence must not auto-apply"
        );
    }

    /// Trusted embedded ISBN allows auto-apply; scan-only does not.
    /// Proves that the trust boundary between Embedded and `ContentScan`
    /// is the gating factor for auto-apply eligibility.
    #[tokio::test]
    async fn trusted_vs_scan_isbn_auto_apply_boundary() {
        let meta = make_metadata(
            "open_library",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        );

        // ── With trusted (embedded) ISBN → StrongIdMatch, auto-apply ──
        let ol = StubProvider::new("open_library").with_isbn_results(vec![meta.clone()]);
        let registry = make_registry(vec![Arc::new(ol)]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };
        let existing_trusted = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            trusted_identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            ..Default::default()
        };

        let result_trusted = resolver.resolve(&query, Some(&existing_trusted)).await;
        let best_trusted = result_trusted.best_match.as_ref().unwrap();
        assert_eq!(best_trusted.tier, CandidateMatchTier::StrongIdMatch);
        assert!(result_trusted.auto_apply, "trusted ISBN should auto-apply");

        // ── Same ISBN as scan-only → ProbableMatch, NO auto-apply ──
        let ol2 = StubProvider::new("open_library").with_isbn_results(vec![meta]);
        let registry2 = make_registry(vec![Arc::new(ol2)]);
        let resolver2 = MetadataResolver::with_defaults(registry2);

        let existing_scan = ExistingBookMetadata {
            title: Some("Dune".to_string()),
            authors: vec!["Frank Herbert".to_string()],
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            trusted_identifiers: vec![], // scan-only: no trusted proof
            ..Default::default()
        };

        let result_scan = resolver2.resolve(&query, Some(&existing_scan)).await;
        let best_scan = result_scan.best_match.as_ref().unwrap();
        assert_eq!(
            best_scan.tier,
            CandidateMatchTier::ProbableMatch,
            "scan-only ISBN must not reach StrongIdMatch"
        );
        assert!(
            !result_scan.auto_apply,
            "scan-only ISBN must not auto-apply"
        );
    }

    #[test]
    fn score_candidate_filters_non_author_roles() {
        let mut meta = make_metadata(
            "ol",
            "The Lady of the Lake",
            &[],
            Some("9780316273770"),
            0.95,
        );
        meta.authors = vec![
            ProviderAuthor {
                name: "Andrzej Sapkowski".into(),
                role: Some("author".into()),
            },
            ProviderAuthor {
                name: "David A. French".into(),
                role: Some("translator".into()),
            },
        ];

        let query = MetadataQuery {
            isbn: Some("9780316273770".into()),
            ..Default::default()
        };

        let existing = ExistingBookMetadata {
            title: Some("The Lady of the Lake".into()),
            authors: vec!["Andrzej Sapkowski".into()],
            ..Default::default()
        };

        let mut candidate = ScoredCandidate {
            metadata: meta,
            score: 0.95,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        score_candidate(&mut candidate, &query, Some(&existing));

        assert!(
            candidate
                .match_reasons
                .iter()
                .any(|r| r.contains("Author match (100%)")),
            "author score should be 100%% when translator is excluded; reasons: {:?}",
            candidate.match_reasons
        );
    }

    #[test]
    fn score_candidate_publisher_match_boosts_score() {
        // Use low base confidence so the publisher bonus doesn't get clamped
        let meta = make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.50);

        let query = MetadataQuery {
            title: Some("Dune".into()),
            ..Default::default()
        };

        let existing = ExistingBookMetadata {
            title: Some("Dune".into()),
            authors: vec!["Frank Herbert".into()],
            publisher: Some("Ace Books".into()),
            ..Default::default()
        };

        let mut with_pub = ScoredCandidate {
            metadata: {
                let mut m = meta.clone();
                m.publisher = Some("Ace Books".into());
                m
            },
            score: 0.50,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        let mut without_pub = ScoredCandidate {
            metadata: {
                let mut m = meta;
                m.publisher = None;
                m
            },
            score: 0.50,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        score_candidate(&mut with_pub, &query, Some(&existing));
        score_candidate(&mut without_pub, &query, Some(&existing));

        assert!(
            with_pub.score > without_pub.score,
            "publisher match should boost score: {:.4} vs {:.4}",
            with_pub.score,
            without_pub.score,
        );
        assert!(
            with_pub
                .match_reasons
                .iter()
                .any(|r| r.contains("Publisher match")),
            "expected Publisher match reason, got: {:?}",
            with_pub.match_reasons
        );
        assert!(
            with_pub.signals.contains(&MatchSignal::PublisherMatch),
            "expected PublisherMatch signal"
        );
    }

    // ── Unsupported format filtering tests ──

    #[test]
    fn is_unsupported_format_detects_audio_formats() {
        let audio_formats = [
            "Audio CD",
            "audio cd",
            "Audio Cassette",
            "MP3 CD",
            "Audiobook",
            "Digital Audio",
            "[Sound recording]",
            "Videorecording",
            "preloaded digital audio player",
            "10 Audio CDs",
            "Listened",
            "Watched",
        ];
        for fmt in audio_formats {
            assert!(is_unsupported_format(fmt), "expected unsupported: {fmt:?}");
        }
    }

    #[test]
    fn is_unsupported_format_allows_ebook_formats() {
        let allowed_formats = [
            "Paperback",
            "Hardcover",
            "E-book",
            "CD-ROM",
            "Unknown Binding",
            "Read",
        ];
        for fmt in allowed_formats {
            assert!(!is_unsupported_format(fmt), "expected supported: {fmt:?}");
        }
    }

    #[tokio::test]
    async fn gather_candidates_drops_audiobook_keeps_paperback() {
        let registry = Arc::new(ProviderRegistry::new());
        let resolver = MetadataResolver::new(registry, Arc::new(StubSettings::new(vec![])));

        let mut audiobook = make_metadata(
            "ol",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        );
        audiobook.physical_format = Some("Audio CD".to_string());

        let mut paperback = make_metadata(
            "ol",
            "Dune",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.90,
        );
        paperback.physical_format = Some("Paperback".to_string());

        let provider = Arc::new(
            StubProvider::new("open_library").with_isbn_results(vec![audiobook, paperback]),
        );

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };

        let candidates = resolver
            .gather_candidates(&query, &[provider as Arc<dyn MetadataProvider>])
            .await;

        assert_eq!(candidates.len(), 1, "audiobook candidate should be dropped");
        assert_eq!(
            candidates[0].metadata.physical_format.as_deref(),
            Some("Paperback"),
            "only paperback candidate should remain"
        );
    }

    #[tokio::test]
    async fn audiobook_identifier_hits_trigger_search_fallback() {
        let registry = Arc::new(ProviderRegistry::new());
        let resolver = MetadataResolver::new(registry, Arc::new(StubSettings::new(vec![])));

        let mut audiobook = make_metadata(
            "stub",
            "Dune (Audio)",
            &["Frank Herbert"],
            Some("9780441172719"),
            0.95,
        );
        audiobook.physical_format = Some("Audio CD".to_string());

        let search_result = make_metadata("stub", "Dune", &["Frank Herbert"], None, 0.75);

        let provider = Arc::new(
            StubProvider::new("stub")
                .with_isbn_results(vec![audiobook])
                .with_search_results(vec![search_result]),
        );

        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            title: Some("Dune".to_string()),
            ..Default::default()
        };

        let candidates = resolver
            .gather_candidates(&query, &[provider as Arc<dyn MetadataProvider>])
            .await;

        assert_eq!(
            candidates.len(),
            1,
            "audiobook filtered, search fallback should produce one result"
        );
        assert_eq!(
            candidates[0].metadata.title.as_deref(),
            Some("Dune"),
            "search fallback candidate should be returned"
        );
    }

    #[tokio::test]
    async fn search_fallback_filters_unsupported_format() {
        let registry = Arc::new(ProviderRegistry::new());
        let resolver = MetadataResolver::new(registry, Arc::new(StubSettings::new(vec![])));

        let mut listened = make_metadata("stub", "Dune (Listened)", &["Frank Herbert"], None, 0.75);
        listened.physical_format = Some("Listened".to_string());

        let provider = Arc::new(StubProvider::new("stub").with_search_results(vec![listened]));

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            ..Default::default()
        };

        let candidates = resolver
            .gather_candidates(&query, &[provider as Arc<dyn MetadataProvider>])
            .await;

        assert!(
            candidates.is_empty(),
            "search fallback should filter unsupported formats"
        );
    }

    // ── Capability-aware filtering ──────────────────────────────────

    static ISBN_ONLY_CAPS: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Authoritative,
        default_rate_limit_rpm: 20,
        supported_id_lookups: &[IdentifierType::Isbn13, IdentifierType::Isbn10],
        features: &[ProviderFeature::Search],
    };

    static ASIN_CAPS: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Curated,
        default_rate_limit_rpm: 50,
        supported_id_lookups: &[
            IdentifierType::Isbn13,
            IdentifierType::Isbn10,
            IdentifierType::Asin,
        ],
        features: &[ProviderFeature::Search, ProviderFeature::Covers],
    };

    static NO_SEARCH_CAPS: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Community,
        default_rate_limit_rpm: 100,
        supported_id_lookups: &[IdentifierType::Isbn13],
        features: &[ProviderFeature::Covers],
    };

    #[tokio::test]
    async fn asin_lookup_skips_non_asin_provider() {
        // `isbn_only` does NOT support ASIN; `asin_provider` does.
        let isbn_only = Arc::new(
            StubProvider::new("isbn_only")
                .with_capabilities(&ISBN_ONLY_CAPS)
                .with_asin_results(vec![make_metadata(
                    "isbn_only",
                    "Should Not Appear",
                    &["Author"],
                    None,
                    0.9,
                )]),
        ) as Arc<dyn MetadataProvider>;

        let asin_provider = Arc::new(
            StubProvider::new("asin_provider")
                .with_capabilities(&ASIN_CAPS)
                .with_asin_results(vec![make_metadata(
                    "asin_provider",
                    "Dune",
                    &["Frank Herbert"],
                    None,
                    0.95,
                )]),
        ) as Arc<dyn MetadataProvider>;

        let registry = make_registry(vec![isbn_only, asin_provider]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            asin: Some("B00GFHJKQ4".to_string()),
            ..Default::default()
        };

        let result = resolver.resolve(&query, None).await;

        // Only `asin_provider` should have been queried.
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].metadata.title.as_deref(), Some("Dune"));
        assert_eq!(result.candidates[0].provider_name, "asin_provider");
    }

    #[tokio::test]
    async fn search_skips_non_search_provider() {
        // `no_search` has only Covers, no Search feature.
        let no_search = Arc::new(
            StubProvider::new("no_search")
                .with_capabilities(&NO_SEARCH_CAPS)
                .with_search_results(vec![make_metadata(
                    "no_search",
                    "Should Not Appear",
                    &["Author"],
                    None,
                    0.9,
                )]),
        ) as Arc<dyn MetadataProvider>;

        let with_search = Arc::new(
            StubProvider::new("with_search")
                .with_capabilities(&ASIN_CAPS)
                .with_search_results(vec![make_metadata(
                    "with_search",
                    "Dune",
                    &["Frank Herbert"],
                    None,
                    0.8,
                )]),
        ) as Arc<dyn MetadataProvider>;

        let registry = make_registry(vec![no_search, with_search]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            ..Default::default()
        };

        let result = resolver.resolve(&query, None).await;

        // Only `with_search` should have been queried.
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].provider_name, "with_search");
    }

    // ── Field completeness / data richness tests ────────────────────

    #[test]
    fn field_completeness_score_minimal() {
        // Title-only metadata (authors empty, everything else None/empty).
        let mut meta = make_metadata("ol", "Dune", &[], None, 0.5);
        meta.authors.clear();
        let fc = FieldCounts::of(&meta);
        assert!(
            fc.ratio() < 0.15,
            "expected low completeness for minimal metadata, got {}/{}",
            fc.present,
            fc.total,
        );
    }

    #[test]
    fn field_completeness_score_full() {
        let meta = ProviderMetadata {
            provider_name: "test".into(),
            title: Some("Dune".into()),
            subtitle: Some("Sub".into()),
            authors: vec![ProviderAuthor {
                name: "Frank Herbert".into(),
                role: Some("author".into()),
            }],
            description: Some("A novel".into()),
            language: Some("en".into()),
            publisher: Some("Ace".into()),
            publication_year: Some(1965),
            identifiers: Vec::new(),
            subjects: vec!["sci-fi".into()],
            series: Some(ProviderSeries {
                name: "Dune".into(),
                position: Some(1.0),
            }),
            page_count: Some(412),
            cover_url: Some("https://example.com/cover.jpg".into()),
            rating: None,
            physical_format: None,
            confidence: 0.9,
        };
        let fc = FieldCounts::of(&meta);
        assert_eq!(
            fc.present, fc.total,
            "expected all fields present for fully populated metadata, got {}/{}",
            fc.present, fc.total,
        );
    }

    #[test]
    fn richness_bonus_differentiates_equal_candidates() {
        let query = MetadataQuery {
            title: Some("Ready for Anything".into()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Ready for Anything".into()),
            authors: vec!["David Allen".into()],
            ..Default::default()
        };

        // Sparse candidate: title + author only.
        let mut sparse = ScoredCandidate {
            metadata: make_metadata("loc", "Ready for Anything", &["David Allen"], None, 0.5),
            score: 0.5,
            provider_name: "loc".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        // Rich candidate: same base, but with subtitle + publisher + cover.
        let mut rich = ScoredCandidate {
            metadata: {
                let mut m = make_metadata("loc", "Ready for Anything", &["David Allen"], None, 0.5);
                m.subtitle = Some("52 Productivity Principles".into());
                m.publisher = Some("Viking".into());
                m.cover_url = Some("https://example.com/cover.jpg".into());
                m.publication_year = Some(2003);
                m
            },
            score: 0.5,
            provider_name: "loc".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        score_candidate(&mut sparse, &query, Some(&existing));
        score_candidate(&mut rich, &query, Some(&existing));

        assert!(
            rich.score > sparse.score,
            "richer candidate should score higher: rich={:.4} vs sparse={:.4}",
            rich.score,
            sparse.score,
        );
    }

    #[test]
    fn richness_cannot_override_isbn_match() {
        let query = MetadataQuery {
            title: Some("Dune".into()),
            isbn: Some("9780441172719".into()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".into()),
            authors: vec!["Frank Herbert".into()],
            ..Default::default()
        };

        // Sparse candidate WITH ISBN match.
        let mut sparse_isbn = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], Some("9780441172719"), 0.5),
            score: 0.5,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        // Rich candidate WITHOUT ISBN match.
        let mut rich_no_isbn = ScoredCandidate {
            metadata: {
                let mut m = make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.5);
                m.subtitle = Some("Sub".into());
                m.publisher = Some("Ace".into());
                m.cover_url = Some("https://example.com/cover.jpg".into());
                m.description = Some("A novel about desert planet".into());
                m.language = Some("en".into());
                m.publication_year = Some(1965);
                m.page_count = Some(412);
                m.subjects = vec!["sci-fi".into()];
                m.series = Some(ProviderSeries {
                    name: "Dune".into(),
                    position: Some(1.0),
                });
                m
            },
            score: 0.5,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };

        score_candidate(&mut sparse_isbn, &query, Some(&existing));
        score_candidate(&mut rich_no_isbn, &query, Some(&existing));

        assert!(
            sparse_isbn.score > rich_no_isbn.score,
            "ISBN match must outweigh richness: isbn={:.4} vs rich={:.4}",
            sparse_isbn.score,
            rich_no_isbn.score,
        );
    }

    #[test]
    fn richness_cannot_override_cross_provider_bonus() {
        // Two candidates: one with cross-provider bonus, one richer but without it.
        let query = MetadataQuery {
            title: Some("Dune".into()),
            ..Default::default()
        };
        let existing = ExistingBookMetadata {
            title: Some("Dune".into()),
            authors: vec!["Frank Herbert".into()],
            ..Default::default()
        };

        // Candidate with cross-provider bonus applied.
        let mut cross_prov = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.5),
            score: 0.5,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        score_candidate(&mut cross_prov, &query, Some(&existing));
        // Simulate cross-provider bonus.
        cross_prov.score = (cross_prov.score + CROSS_PROVIDER_BONUS).min(1.0);

        // Rich candidate without cross-provider bonus.
        let mut rich = ScoredCandidate {
            metadata: {
                let mut m = make_metadata("hc", "Dune", &["Frank Herbert"], None, 0.5);
                m.subtitle = Some("Sub".into());
                m.publisher = Some("Ace".into());
                m.cover_url = Some("https://example.com/cover.jpg".into());
                m.description = Some("A novel".into());
                m.language = Some("en".into());
                m.publication_year = Some(1965);
                m.page_count = Some(412);
                m.subjects = vec!["sci-fi".into()];
                m.series = Some(ProviderSeries {
                    name: "Dune".into(),
                    position: Some(1.0),
                });
                m
            },
            score: 0.5,
            provider_name: "hardcover".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::WeakMatch,
            field_count: 0,
        };
        score_candidate(&mut rich, &query, Some(&existing));

        assert!(
            cross_prov.score > rich.score,
            "cross-provider bonus must outweigh richness: cross={:.4} vs rich={:.4}",
            cross_prov.score,
            rich.score,
        );
    }

    #[test]
    fn sort_tiebreaker_prefers_richer_candidate() {
        let sparse = ScoredCandidate {
            metadata: make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.5),
            score: 0.85,
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::ProbableMatch,
            field_count: 1, // authors only
        };

        let rich = ScoredCandidate {
            metadata: {
                let mut m = make_metadata("ol", "Dune", &["Frank Herbert"], None, 0.5);
                m.subtitle = Some("Sub".into());
                m.publisher = Some("Ace".into());
                m.cover_url = Some("https://example.com/cover.jpg".into());
                m
            },
            score: 0.85, // Identical score.
            provider_name: "open_library".into(),
            match_reasons: Vec::new(),
            signals: HashSet::new(),
            tier: CandidateMatchTier::ProbableMatch,
            field_count: 4, // authors + subtitle + publisher + cover_url
        };

        // Sparse first, rich second — sort should flip them.
        let mut candidates = [sparse, rich];
        candidates.sort_by(cmp_candidates);

        assert!(
            candidates[0].metadata.subtitle.is_some(),
            "richer candidate should sort first on tie"
        );
    }

    #[tokio::test]
    async fn full_resolve_ranks_richer_candidate_higher() {
        // Replicate the "Ready for Anything" scenario: two candidates from the
        // same provider with different field completeness should rank the richer
        // one higher.
        let sparse_meta = make_metadata(
            "stub",
            "Ready for Anything",
            &["David Allen"],
            Some("9780743535304"),
            0.5,
        );

        let rich_meta = {
            let mut m = make_metadata(
                "stub",
                "Ready for Anything",
                &["David Allen"],
                Some("9780670032501"),
                0.5,
            );
            m.subtitle = Some("52 Productivity Principles for Work and Life".into());
            m.publisher = Some("Viking".into());
            m.publication_year = Some(2003);
            m.page_count = Some(164);
            m
        };

        let provider =
            Arc::new(StubProvider::new("stub").with_search_results(vec![sparse_meta, rich_meta]))
                as Arc<dyn MetadataProvider>;

        let registry = make_registry(vec![provider]);
        let resolver = MetadataResolver::with_defaults(registry);

        let query = MetadataQuery {
            title: Some("Ready for Anything".into()),
            ..Default::default()
        };

        let existing = ExistingBookMetadata {
            title: Some(
                "Ready for Anything: 52 Productivity Principles for Getting Things Done".into(),
            ),
            authors: vec!["David Allen".into()],
            ..Default::default()
        };

        let result = resolver.resolve(&query, Some(&existing)).await;

        assert!(
            result.candidates.len() >= 2,
            "expected at least 2 candidates, got {}",
            result.candidates.len()
        );

        // The richer candidate should rank first.
        assert!(
            result.candidates[0].metadata.subtitle.is_some(),
            "richer candidate (with subtitle) should rank first, but first candidate subtitle: {:?}",
            result.candidates[0].metadata.subtitle,
        );
        assert!(
            result.candidates[0].score >= result.candidates[1].score,
            "first candidate score ({:.4}) should be >= second ({:.4})",
            result.candidates[0].score,
            result.candidates[1].score,
        );
    }
}
