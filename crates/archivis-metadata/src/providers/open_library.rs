use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use archivis_core::models::IdentifierType;

use crate::client::MetadataHttpClient;
use crate::errors::ProviderError;
use crate::provider::MetadataProvider;
use crate::types::{
    MetadataQuery, ProviderAuthor, ProviderIdentifier, ProviderMetadata, ProviderSeries,
};

const PROVIDER_NAME: &str = "open_library";
const BASE_URL: &str = "https://openlibrary.org";
const COVERS_URL: &str = "https://covers.openlibrary.org";
const MAX_REQUESTS_PER_MINUTE: u32 = 100;

/// Open Library metadata provider.
///
/// Uses the Open Library REST API for ISBN lookups, title+author search,
/// and cover image retrieval. No authentication is required.
pub struct OpenLibraryProvider {
    client: Arc<MetadataHttpClient>,
    enabled: bool,
    /// Cache of author OLID -> name mappings to avoid repeated lookups.
    author_cache: RwLock<HashMap<String, String>>,
}

impl OpenLibraryProvider {
    /// Create a new Open Library provider.
    ///
    /// The provider registers itself with the shared HTTP client for rate
    /// limiting at 100 requests/minute.
    pub fn new(client: Arc<MetadataHttpClient>, enabled: bool) -> Self {
        Self {
            client,
            enabled,
            author_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Register this provider's rate limiter with the shared HTTP client.
    /// Must be called before making requests. Accepts a mutable reference
    /// to the client during initialization.
    pub fn register_rate_limiter(client: &mut MetadataHttpClient) {
        client.register_provider(PROVIDER_NAME, MAX_REQUESTS_PER_MINUTE);
    }

    /// Register a custom rate limit with the shared HTTP client.
    pub fn register_rate_limiter_with_limit(client: &mut MetadataHttpClient, max_rpm: u32) {
        client.register_provider(PROVIDER_NAME, max_rpm);
    }

    /// Look up an edition by ISBN, then fetch the parent work for
    /// description, subjects, and consolidated metadata.
    async fn fetch_isbn_edition(
        &self,
        isbn: &str,
    ) -> Result<Option<ProviderMetadata>, ProviderError> {
        let url = format!("{BASE_URL}/isbn/{isbn}.json");
        let response = self.client.get(PROVIDER_NAME, &url).await?;
        let status = response.status().as_u16();

        if status == 404 {
            debug!(isbn = isbn, "ISBN not found on Open Library");
            return Ok(None);
        }

        if !response.status().is_success() {
            return Err(ProviderError::ApiError {
                status,
                message: format!("ISBN lookup returned HTTP {status}"),
            });
        }

        let edition: OlEdition = response
            .json()
            .await
            .map_err(|e| ProviderError::ParseError(format!("failed to parse edition: {e}")))?;

        // Fetch the parent work for description/subjects if we have a works reference.
        let work = if let Some(work_key) = edition.works.as_ref().and_then(|w| w.first()) {
            self.fetch_work(&work_key.key).await?
        } else {
            None
        };

        // Resolve author names from OLIDs.
        let authors = self.resolve_authors(edition.authors.as_ref()).await;

        let metadata = Self::build_metadata_from_edition(&edition, work.as_ref(), &authors, isbn);
        Ok(Some(metadata))
    }

    /// Fetch a work by its key (e.g., `/works/OL123W`).
    async fn fetch_work(&self, work_key: &str) -> Result<Option<OlWork>, ProviderError> {
        let url = format!("{BASE_URL}{work_key}.json");
        let response = self.client.get(PROVIDER_NAME, &url).await?;

        if !response.status().is_success() {
            debug!(
                work_key = work_key,
                status = response.status().as_u16(),
                "failed to fetch work"
            );
            return Ok(None);
        }

        let work: OlWork = response
            .json()
            .await
            .map_err(|e| ProviderError::ParseError(format!("failed to parse work: {e}")))?;

        Ok(Some(work))
    }

    /// Resolve a list of author references to names, using the cache.
    async fn resolve_authors(&self, author_refs: Option<&Vec<OlAuthorRef>>) -> Vec<ProviderAuthor> {
        let Some(refs) = author_refs else {
            return Vec::new();
        };

        let mut authors = Vec::with_capacity(refs.len());

        for author_ref in refs {
            let key = author_ref.key();
            let Some(key) = key else {
                continue;
            };

            // Check cache first.
            {
                let cache = self.author_cache.read().await;
                if let Some(name) = cache.get(key) {
                    authors.push(ProviderAuthor {
                        name: name.clone(),
                        role: Some("author".to_string()),
                    });
                    continue;
                }
            }

            // Fetch from API and cache.
            match self.fetch_author(key).await {
                Ok(Some(name)) => {
                    {
                        let mut cache = self.author_cache.write().await;
                        cache.insert(key.to_string(), name.clone());
                    }
                    authors.push(ProviderAuthor {
                        name,
                        role: Some("author".to_string()),
                    });
                }
                Ok(None) => {
                    debug!(author_key = key, "author not found");
                }
                Err(e) => {
                    warn!(author_key = key, error = %e, "failed to fetch author");
                }
            }
        }

        authors
    }

    /// Fetch an author by key and return their name.
    async fn fetch_author(&self, author_key: &str) -> Result<Option<String>, ProviderError> {
        let url = format!("{BASE_URL}{author_key}.json");
        let response = self.client.get(PROVIDER_NAME, &url).await?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let author: OlAuthor = response
            .json()
            .await
            .map_err(|e| ProviderError::ParseError(format!("failed to parse author: {e}")))?;

        Ok(author.name.or(author.personal_name))
    }

    /// Build a `ProviderMetadata` from an edition, optional work, and
    /// resolved author names.
    fn build_metadata_from_edition(
        edition: &OlEdition,
        work: Option<&OlWork>,
        authors: &[ProviderAuthor],
        queried_isbn: &str,
    ) -> ProviderMetadata {
        let title = edition
            .title
            .clone()
            .or_else(|| work.and_then(|w| w.title.clone()));

        let description = work
            .and_then(|w| extract_text_value(w.description.as_ref()))
            .or_else(|| extract_text_value(edition.description.as_ref()));

        let publisher = edition.publishers.as_ref().and_then(|p| p.first().cloned());

        let publication_date = edition.publish_date.clone();

        let page_count = edition.number_of_pages;

        let subjects = work.and_then(|w| w.subjects.clone()).unwrap_or_default();

        let series = edition
            .series
            .as_ref()
            .and_then(|s| s.first().map(|name| parse_series_string(name)));

        let cover_url = edition
            .covers
            .as_ref()
            .and_then(|covers| covers.first().map(|id| cover_url_from_id(*id)));

        let language = edition
            .languages
            .as_ref()
            .and_then(|langs| langs.first())
            .and_then(|lang| ol_language_to_iso(&lang.key));

        let mut identifiers = Vec::new();

        // ISBN-13 — keep only the first entry; editions rarely have
        // multiple ISBN-13s, but cap defensively.
        if let Some(isbn) = edition.isbn_13.as_ref().and_then(|v| v.first()) {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: isbn.clone(),
            });
        }

        // ISBN-10 — same: keep only the first.
        if let Some(isbn) = edition.isbn_10.as_ref().and_then(|v| v.first()) {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Isbn10,
                value: isbn.clone(),
            });
        }

        // Open Library key as OLID
        if let Some(ref key) = edition.key {
            let olid = key.trim_start_matches("/books/").to_string();
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::OpenLibrary,
                value: olid,
            });
        }

        // Ensure the queried ISBN is in the identifiers list.
        let queried_present = identifiers.iter().any(|id| id.value == queried_isbn);
        if !queried_present {
            let id_type = if queried_isbn.len() == 13 {
                IdentifierType::Isbn13
            } else {
                IdentifierType::Isbn10
            };
            identifiers.push(ProviderIdentifier {
                identifier_type: id_type,
                value: queried_isbn.to_string(),
            });
        }

        ProviderMetadata {
            provider_name: PROVIDER_NAME.to_string(),
            title,
            authors: authors.to_vec(),
            description,
            language,
            publisher,
            publication_date,
            identifiers,
            subjects,
            series,
            page_count,
            cover_url,
            rating: None, // Open Library doesn't provide ratings in edition/work API.
            confidence: 0.95,
        }
    }

    /// Parse search results into `ProviderMetadata` entries.
    fn parse_search_results(
        results: &OlSearchResponse,
        query: &MetadataQuery,
    ) -> Vec<ProviderMetadata> {
        results
            .docs
            .iter()
            .map(|doc| {
                let title = doc.title.clone();

                let authors: Vec<ProviderAuthor> = doc
                    .author_name
                    .as_ref()
                    .map(|names| {
                        names
                            .iter()
                            .map(|n| ProviderAuthor {
                                name: n.clone(),
                                role: Some("author".to_string()),
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let cover_url = doc.cover_i.map(cover_url_from_id);

                let publisher = doc.publisher.as_ref().and_then(|p| p.first().cloned());

                let page_count = doc.number_of_pages_median;

                let subjects = doc.subject.clone().unwrap_or_default();

                // OL search results aggregate ISBNs across ALL editions of a
                // work (hardcover, paperback, audiobook, etc.). Limit to at most
                // one ISBN-13 and one ISBN-10 to avoid polluting the book record
                // with identifiers from unrelated editions.
                let mut identifiers = Vec::new();
                if let Some(ref isbns) = doc.isbn {
                    let mut has_isbn13 = false;
                    let mut has_isbn10 = false;
                    for isbn in isbns {
                        if isbn.len() == 13 && !has_isbn13 {
                            has_isbn13 = true;
                            identifiers.push(ProviderIdentifier {
                                identifier_type: IdentifierType::Isbn13,
                                value: isbn.clone(),
                            });
                        } else if isbn.len() != 13 && !has_isbn10 {
                            has_isbn10 = true;
                            identifiers.push(ProviderIdentifier {
                                identifier_type: IdentifierType::Isbn10,
                                value: isbn.clone(),
                            });
                        }
                        if has_isbn13 && has_isbn10 {
                            break;
                        }
                    }
                }

                if let Some(ref key) = doc.key {
                    let olid = key.trim_start_matches("/works/").to_string();
                    identifiers.push(ProviderIdentifier {
                        identifier_type: IdentifierType::OpenLibrary,
                        value: olid,
                    });
                }

                let publication_date = doc.first_publish_year.map(|y| y.to_string());

                // Compute confidence based on title match quality.
                let confidence = compute_search_confidence(
                    doc.title.as_deref(),
                    query.title.as_deref(),
                    query.isbn.as_deref(),
                    &identifiers,
                );

                ProviderMetadata {
                    provider_name: PROVIDER_NAME.to_string(),
                    title,
                    authors,
                    description: None, // Search results don't include descriptions.
                    language: None,    // Search results don't include language reliably.
                    publisher,
                    publication_date,
                    identifiers,
                    subjects,
                    series: None, // Search results don't include series.
                    page_count,
                    cover_url,
                    rating: None,
                    confidence,
                }
            })
            .collect()
    }
}

#[async_trait]
impl MetadataProvider for OpenLibraryProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn is_available(&self) -> bool {
        self.enabled
    }

    async fn lookup_isbn(&self, isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.enabled {
            return Err(ProviderError::NotConfigured(
                "Open Library provider is disabled".to_string(),
            ));
        }

        Ok(self
            .fetch_isbn_edition(isbn)
            .await?
            .map_or_else(Vec::new, |metadata| vec![metadata]))
    }

    async fn search(&self, query: &MetadataQuery) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.enabled {
            return Err(ProviderError::NotConfigured(
                "Open Library provider is disabled".to_string(),
            ));
        }

        // Build search URL based on available query fields.
        let url = build_search_url(query);

        let response = self.client.get(PROVIDER_NAME, &url).await?;
        let status = response.status().as_u16();

        if !response.status().is_success() {
            return Err(ProviderError::ApiError {
                status,
                message: format!("search returned HTTP {status}"),
            });
        }

        let search_response: OlSearchResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ParseError(format!("failed to parse search: {e}")))?;

        let mut results = Self::parse_search_results(&search_response, query);

        // Sort by confidence descending.
        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    async fn fetch_cover(&self, cover_url: &str) -> Result<Vec<u8>, ProviderError> {
        if !self.enabled {
            return Err(ProviderError::NotConfigured(
                "Open Library provider is disabled".to_string(),
            ));
        }

        // Cover images are fetched without rate limiting (they come from a CDN).
        let response = self.client.raw_client().get(cover_url).send().await?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(ProviderError::ApiError {
                status,
                message: format!("cover fetch returned HTTP {status}"),
            });
        }

        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }
}

// ── Open Library API response types ──────────────────────────────────

/// Open Library edition response (from `/isbn/{isbn}.json` or `/books/{olid}.json`).
#[derive(Debug, Deserialize)]
struct OlEdition {
    key: Option<String>,
    title: Option<String>,
    authors: Option<Vec<OlAuthorRef>>,
    publishers: Option<Vec<String>>,
    publish_date: Option<String>,
    isbn_13: Option<Vec<String>>,
    isbn_10: Option<Vec<String>>,
    number_of_pages: Option<i32>,
    covers: Option<Vec<i64>>,
    languages: Option<Vec<OlKeyRef>>,
    series: Option<Vec<String>>,
    works: Option<Vec<OlKeyRef>>,
    description: Option<serde_json::Value>,
}

/// Author reference in an edition — may be `{"key": "/authors/OL1234A"}`
/// or `{"author": {"key": "/authors/OL1234A"}}`.
#[derive(Debug, Deserialize)]
struct OlAuthorRef {
    key: Option<String>,
    author: Option<OlKeyRef>,
}

impl OlAuthorRef {
    /// Extract the author key, handling both direct key and nested author object.
    fn key(&self) -> Option<&str> {
        self.key
            .as_deref()
            .or_else(|| self.author.as_ref().map(|a| a.key.as_str()))
    }
}

/// Simple `{"key": "..."}` reference used throughout the OL API.
#[derive(Debug, Deserialize)]
struct OlKeyRef {
    key: String,
}

/// Open Library author response (from `/authors/{olid}.json`).
#[derive(Debug, Deserialize)]
struct OlAuthor {
    name: Option<String>,
    personal_name: Option<String>,
}

/// Open Library work response (from `/works/{olid}.json`).
#[derive(Debug, Deserialize)]
struct OlWork {
    title: Option<String>,
    description: Option<serde_json::Value>,
    subjects: Option<Vec<String>>,
}

/// Open Library search response (from `/search.json`).
#[derive(Debug, Deserialize)]
struct OlSearchResponse {
    docs: Vec<OlSearchDoc>,
}

/// A single document in the search results.
#[derive(Debug, Deserialize)]
struct OlSearchDoc {
    key: Option<String>,
    title: Option<String>,
    author_name: Option<Vec<String>>,
    first_publish_year: Option<i32>,
    isbn: Option<Vec<String>>,
    cover_i: Option<i64>,
    publisher: Option<Vec<String>>,
    number_of_pages_median: Option<i32>,
    subject: Option<Vec<String>>,
}

// ── Helper functions ─────────────────────────────────────────────────

/// Construct a cover image URL from a cover ID.
///
/// Uses the cover ID-based URL (not ISBN-based) to avoid the stricter
/// rate limits on the ISBN-based covers endpoint.
pub fn cover_url_from_id(cover_id: i64) -> String {
    format!("{COVERS_URL}/b/id/{cover_id}-L.jpg")
}

/// Parse an Open Library series string into a name and optional position.
///
/// OL series strings often embed the volume number in the name, e.g.
/// `"Harry Potter, #6"`, `"Discworld #12"`, or `"Dune, 3"`.
/// This splits the trailing number from the series name.
fn parse_series_string(raw: &str) -> ProviderSeries {
    let trimmed = raw.trim();

    // Try to split on the last comma or '#' that precedes a number.
    // Patterns: "Name, #6", "Name #6", "Name, 6", "Name 6"
    // We scan from the end to find a trailing number (integer or decimal).
    if let Some(pos) = find_trailing_position(trimmed) {
        let (name_part, num_str) = trimmed.split_at(pos);
        // Strip trailing separators: ", #", " #", ", ", " "
        let name = name_part.trim_end_matches(|c: char| c == ',' || c == '#' || c.is_whitespace());
        if !name.is_empty() {
            if let Ok(position) = num_str.trim().parse::<f32>() {
                return ProviderSeries {
                    name: name.to_string(),
                    position: Some(position),
                };
            }
        }
    }

    ProviderSeries {
        name: trimmed.to_string(),
        position: None,
    }
}

/// Find the byte offset where a trailing numeric position starts.
///
/// Looks for patterns like `, #6`, ` #12`, `, 3`, ` 3.5` at the end
/// of the string.  Returns the start of the numeric portion.
fn find_trailing_position(s: &str) -> Option<usize> {
    // Walk backwards to find the start of a trailing number (digits and optional dot).
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return None;
    }

    // Find the end of the trailing number (must end with a digit).
    let mut i = len;
    if !bytes[i - 1].is_ascii_digit() {
        return None;
    }

    // Walk back through digits and at most one decimal point.
    let mut saw_dot = false;
    while i > 0 {
        let b = bytes[i - 1];
        if b.is_ascii_digit() {
            i -= 1;
        } else if b == b'.' && !saw_dot {
            saw_dot = true;
            i -= 1;
        } else {
            break;
        }
    }

    let num_start = i;

    // There must be something before the number.
    if num_start == 0 {
        return None;
    }

    // Skip optional '#' before the number.
    if i > 0 && bytes[i - 1] == b'#' {
        i -= 1;
    }

    // Skip whitespace and/or comma separator.
    while i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b',') {
        i -= 1;
    }

    // Only accept if we actually consumed a separator (not just "Name6").
    if i == num_start {
        return None;
    }

    Some(num_start)
}

/// Map an Open Library language key to an ISO 639-1 code.
///
/// OL language keys look like `/languages/eng`. This maps common
/// three-letter codes to two-letter ISO 639-1 codes.
fn ol_language_to_iso(key: &str) -> Option<String> {
    let code = key.trim_start_matches("/languages/");
    match code {
        "eng" => Some("en".to_string()),
        "fre" | "fra" => Some("fr".to_string()),
        "ger" | "deu" => Some("de".to_string()),
        "spa" => Some("es".to_string()),
        "ita" => Some("it".to_string()),
        "por" => Some("pt".to_string()),
        "rus" => Some("ru".to_string()),
        "jpn" => Some("ja".to_string()),
        "chi" | "zho" => Some("zh".to_string()),
        "kor" => Some("ko".to_string()),
        "ara" => Some("ar".to_string()),
        "hin" => Some("hi".to_string()),
        "dut" | "nld" => Some("nl".to_string()),
        "pol" => Some("pl".to_string()),
        "swe" => Some("sv".to_string()),
        "nor" | "nob" | "nno" => Some("no".to_string()),
        "dan" => Some("da".to_string()),
        "fin" => Some("fi".to_string()),
        "tur" => Some("tr".to_string()),
        "cze" | "ces" => Some("cs".to_string()),
        "hun" => Some("hu".to_string()),
        "rum" | "ron" => Some("ro".to_string()),
        "gre" | "ell" => Some("el".to_string()),
        "heb" => Some("he".to_string()),
        "tha" => Some("th".to_string()),
        "vie" => Some("vi".to_string()),
        "ukr" => Some("uk".to_string()),
        "cat" => Some("ca".to_string()),
        "bul" => Some("bg".to_string()),
        "hrv" => Some("hr".to_string()),
        "srp" => Some("sr".to_string()),
        "slv" => Some("sl".to_string()),
        "lit" => Some("lt".to_string()),
        "lav" => Some("lv".to_string()),
        "est" => Some("et".to_string()),
        "ind" => Some("id".to_string()),
        "may" | "msa" => Some("ms".to_string()),
        "per" | "fas" => Some("fa".to_string()),
        "urd" => Some("ur".to_string()),
        _ => {
            // For unknown codes, return the raw three-letter code.
            if code.len() <= 3 && !code.is_empty() {
                Some(code.to_string())
            } else {
                None
            }
        }
    }
}

/// Extract text from an OL description field which may be either a
/// plain string or an object `{"type": "/type/text", "value": "..."}`.
fn extract_text_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Object(map)) => {
            map.get("value").and_then(|v| v.as_str()).map(String::from)
        }
        _ => None,
    }
}

/// Build a search URL from a `MetadataQuery`.
fn build_search_url(query: &MetadataQuery) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());

    // If only ISBN is set, use the isbn parameter.
    if query.isbn.is_some() && query.title.is_none() && query.author.is_none() {
        if let Some(ref isbn) = query.isbn {
            serializer.append_pair("isbn", isbn);
        }
    } else {
        if let Some(ref title) = query.title {
            serializer.append_pair("title", title);
        }
        if let Some(ref author) = query.author {
            serializer.append_pair("author", author);
        }
    }

    serializer.append_pair(
        "fields",
        "key,title,author_name,first_publish_year,isbn,cover_i,publisher,number_of_pages_median,subject",
    );
    serializer.append_pair("limit", "5");

    let params = serializer.finish();
    format!("{BASE_URL}/search.json?{params}")
}

/// Compute confidence for a search result based on how well it matches
/// the query.
fn compute_search_confidence(
    result_title: Option<&str>,
    query_title: Option<&str>,
    query_isbn: Option<&str>,
    result_identifiers: &[ProviderIdentifier],
) -> f32 {
    // If the result contains the queried ISBN, that's a strong signal.
    if let Some(isbn) = query_isbn {
        let isbn_match = result_identifiers.iter().any(|id| {
            matches!(
                id.identifier_type,
                IdentifierType::Isbn13 | IdentifierType::Isbn10
            ) && id.value == isbn
        });
        if isbn_match {
            return 0.8;
        }
    }

    // Compare titles for fuzzy matching.
    match (result_title, query_title) {
        (Some(result), Some(query)) => {
            let r = normalize_for_comparison(result);
            let q = normalize_for_comparison(query);

            if r == q {
                0.8
            } else if r.contains(&q) || q.contains(&r) {
                0.7
            } else {
                // Simple word overlap ratio.
                let r_words: Vec<&str> = r.split_whitespace().collect();
                let q_words: Vec<&str> = q.split_whitespace().collect();
                let common = r_words.iter().filter(|w| q_words.contains(w)).count();
                let total = r_words.len().max(q_words.len()).max(1);
                #[allow(clippy::cast_precision_loss)]
                let ratio = (common as f32) / (total as f32);
                0.5 + (ratio * 0.3)
            }
        }
        _ => 0.5,
    }
}

/// Normalize a string for comparison: lowercase, strip articles, remove
/// punctuation, collapse whitespace.
fn normalize_for_comparison(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut result = String::with_capacity(lower.len());

    for ch in lower.chars() {
        if ch.is_alphanumeric() || ch.is_whitespace() {
            result.push(ch);
        } else {
            result.push(' ');
        }
    }

    // Collapse whitespace.
    let collapsed: Vec<&str> = result.split_whitespace().collect();

    // Strip leading articles.
    let articles = ["the", "a", "an"];
    let start = usize::from(collapsed.first().is_some_and(|w| articles.contains(w)));

    collapsed[start..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cover URL construction ───────────────────────────────────────

    #[test]
    fn cover_url_from_cover_id() {
        let url = cover_url_from_id(12345);
        assert_eq!(url, "https://covers.openlibrary.org/b/id/12345-L.jpg");
    }

    #[test]
    fn cover_url_negative_id() {
        // Negative cover IDs should still produce a valid URL format.
        let url = cover_url_from_id(-1);
        assert_eq!(url, "https://covers.openlibrary.org/b/id/-1-L.jpg");
    }

    // ── Language mapping ─────────────────────────────────────────────

    #[test]
    fn language_key_to_iso_common() {
        assert_eq!(ol_language_to_iso("/languages/eng"), Some("en".to_string()));
        assert_eq!(ol_language_to_iso("/languages/fre"), Some("fr".to_string()));
        assert_eq!(ol_language_to_iso("/languages/ger"), Some("de".to_string()));
        assert_eq!(ol_language_to_iso("/languages/spa"), Some("es".to_string()));
        assert_eq!(ol_language_to_iso("/languages/jpn"), Some("ja".to_string()));
        assert_eq!(ol_language_to_iso("/languages/chi"), Some("zh".to_string()));
    }

    #[test]
    fn language_key_unknown_code() {
        // Unknown three-letter codes are returned as-is.
        assert_eq!(
            ol_language_to_iso("/languages/xyz"),
            Some("xyz".to_string())
        );
    }

    #[test]
    fn language_key_empty() {
        assert_eq!(ol_language_to_iso("/languages/"), None);
    }

    // ── Text extraction ──────────────────────────────────────────────

    #[test]
    fn extract_text_from_string() {
        let val = serde_json::Value::String("A great book.".to_string());
        assert_eq!(
            extract_text_value(Some(&val)),
            Some("A great book.".to_string())
        );
    }

    #[test]
    fn extract_text_from_object() {
        let val = serde_json::json!({
            "type": "/type/text",
            "value": "A great book."
        });
        assert_eq!(
            extract_text_value(Some(&val)),
            Some("A great book.".to_string())
        );
    }

    #[test]
    fn extract_text_from_none() {
        assert_eq!(extract_text_value(None), None);
    }

    // ── Search URL construction ──────────────────────────────────────

    #[test]
    fn search_url_title_and_author() {
        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };
        let url = build_search_url(&query);
        assert!(url.starts_with("https://openlibrary.org/search.json?"));
        assert!(url.contains("title=Dune"));
        assert!(url.contains("author=Frank+Herbert"));
        assert!(url.contains("limit=5"));
    }

    #[test]
    fn search_url_isbn_only() {
        let query = MetadataQuery {
            isbn: Some("9780441172719".to_string()),
            ..Default::default()
        };
        let url = build_search_url(&query);
        assert!(url.contains("isbn=9780441172719"));
        assert!(!url.contains("title="));
    }

    // ── Normalization ────────────────────────────────────────────────

    #[test]
    fn normalize_strips_articles() {
        assert_eq!(normalize_for_comparison("The Hobbit"), "hobbit");
        assert_eq!(
            normalize_for_comparison("A Song of Ice and Fire"),
            "song of ice and fire"
        );
        assert_eq!(
            normalize_for_comparison("An Ember in the Ashes"),
            "ember in the ashes"
        );
    }

    #[test]
    fn normalize_removes_punctuation() {
        assert_eq!(
            normalize_for_comparison("Dune: The Machine Crusade"),
            "dune the machine crusade"
        );
    }

    // ── Confidence computation ───────────────────────────────────────

    #[test]
    fn confidence_exact_title_match() {
        let confidence = compute_search_confidence(Some("Dune"), Some("Dune"), None, &[]);
        assert!((confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_isbn_match() {
        let identifiers = vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780441172719".to_string(),
        }];
        let confidence = compute_search_confidence(
            Some("Dune"),
            Some("Different Title"),
            Some("9780441172719"),
            &identifiers,
        );
        assert!((confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_partial_title_match() {
        let confidence = compute_search_confidence(Some("Dune Messiah"), Some("Dune"), None, &[]);
        // "dune messiah" contains "dune" -> 0.7
        assert!((confidence - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_no_title_match() {
        let confidence = compute_search_confidence(Some("Foundation"), Some("Dune"), None, &[]);
        // Should be in the 0.5-0.8 range, low end.
        assert!(confidence >= 0.5);
        assert!(confidence <= 0.8);
    }

    // ── Edition JSON parsing ─────────────────────────────────────────

    #[test]
    fn parse_edition_json() {
        let json = r#"{
            "key": "/books/OL7353617M",
            "title": "Dune",
            "authors": [
                {"key": "/authors/OL34221A"}
            ],
            "publishers": ["Chilton Books"],
            "publish_date": "1965",
            "isbn_13": ["9780441172719"],
            "isbn_10": ["0441172717"],
            "number_of_pages": 412,
            "covers": [8231856],
            "languages": [{"key": "/languages/eng"}],
            "works": [{"key": "/works/OL893415W"}]
        }"#;

        let edition: OlEdition = serde_json::from_str(json).unwrap();
        assert_eq!(edition.title.as_deref(), Some("Dune"));
        assert_eq!(edition.publishers.as_ref().unwrap()[0], "Chilton Books");
        assert_eq!(edition.isbn_13.as_ref().unwrap()[0], "9780441172719");
        assert_eq!(edition.number_of_pages, Some(412));
        assert_eq!(edition.covers.as_ref().unwrap()[0], 8_231_856);
        assert_eq!(edition.languages.as_ref().unwrap()[0].key, "/languages/eng");
    }

    #[test]
    fn parse_edition_minimal() {
        let json = r#"{
            "title": "Unknown Book"
        }"#;

        let edition: OlEdition = serde_json::from_str(json).unwrap();
        assert_eq!(edition.title.as_deref(), Some("Unknown Book"));
        assert!(edition.authors.is_none());
        assert!(edition.isbn_13.is_none());
        assert!(edition.covers.is_none());
    }

    // ── Work JSON parsing ────────────────────────────────────────────

    #[test]
    fn parse_work_json() {
        let json = r#"{
            "title": "Dune",
            "description": {
                "type": "/type/text",
                "value": "Set on the desert planet Arrakis, Dune is the story of the boy Paul Atreides."
            },
            "subjects": ["Science fiction", "Sand dunes", "Ecology"]
        }"#;

        let work: OlWork = serde_json::from_str(json).unwrap();
        assert_eq!(work.title.as_deref(), Some("Dune"));
        let desc = extract_text_value(work.description.as_ref()).unwrap();
        assert!(desc.contains("Paul Atreides"));
        assert_eq!(work.subjects.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn parse_work_description_as_string() {
        let json = r#"{
            "title": "Dune",
            "description": "A science fiction classic."
        }"#;

        let work: OlWork = serde_json::from_str(json).unwrap();
        let desc = extract_text_value(work.description.as_ref()).unwrap();
        assert_eq!(desc, "A science fiction classic.");
    }

    // ── Search response parsing ──────────────────────────────────────

    #[test]
    fn parse_search_response() {
        let json = r#"{
            "numFound": 2,
            "start": 0,
            "docs": [
                {
                    "key": "/works/OL893415W",
                    "title": "Dune",
                    "author_name": ["Frank Herbert"],
                    "first_publish_year": 1965,
                    "isbn": ["9780441172719", "0441172717"],
                    "cover_i": 8231856,
                    "publisher": ["Chilton Books", "Ace Books"],
                    "number_of_pages_median": 412,
                    "subject": ["Science Fiction", "Space opera"]
                },
                {
                    "key": "/works/OL15149W",
                    "title": "Dune Messiah",
                    "author_name": ["Frank Herbert"],
                    "first_publish_year": 1969,
                    "isbn": ["9780441172696"],
                    "cover_i": 8231857,
                    "publisher": ["Putnam"],
                    "number_of_pages_median": 256,
                    "subject": ["Science Fiction"]
                }
            ]
        }"#;

        let search_response: OlSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(search_response.docs.len(), 2);

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };

        let results = OpenLibraryProvider::parse_search_results(&search_response, &query);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].provider_name, "open_library");
        assert_eq!(results[0].title.as_deref(), Some("Dune"));
        assert_eq!(results[0].authors.len(), 1);
        assert_eq!(results[0].authors[0].name, "Frank Herbert");
        assert_eq!(
            results[0].cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/8231856-L.jpg")
        );
        assert_eq!(results[0].publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(results[0].page_count, Some(412));
        assert_eq!(results[0].subjects.len(), 2);
        assert!(results[0].confidence > 0.5);

        assert_eq!(results[1].title.as_deref(), Some("Dune Messiah"));
    }

    // ── Full metadata construction from edition ──────────────────────

    #[test]
    fn build_metadata_from_edition_complete() {
        let edition = OlEdition {
            key: Some("/books/OL7353617M".to_string()),
            title: Some("Dune".to_string()),
            authors: None,
            publishers: Some(vec!["Chilton Books".to_string()]),
            publish_date: Some("1965".to_string()),
            isbn_13: Some(vec!["9780441172719".to_string()]),
            isbn_10: Some(vec!["0441172717".to_string()]),
            number_of_pages: Some(412),
            covers: Some(vec![8_231_856]),
            languages: Some(vec![OlKeyRef {
                key: "/languages/eng".to_string(),
            }]),
            series: None,
            works: None,
            description: None,
        };

        let work = OlWork {
            title: Some("Dune".to_string()),
            description: Some(serde_json::Value::String(
                "A science fiction classic.".to_string(),
            )),
            subjects: Some(vec!["Science Fiction".to_string(), "Ecology".to_string()]),
        };

        let authors = vec![ProviderAuthor {
            name: "Frank Herbert".to_string(),
            role: Some("author".to_string()),
        }];

        let metadata = OpenLibraryProvider::build_metadata_from_edition(
            &edition,
            Some(&work),
            &authors,
            "9780441172719",
        );

        assert_eq!(metadata.provider_name, "open_library");
        assert_eq!(metadata.title.as_deref(), Some("Dune"));
        assert_eq!(metadata.authors.len(), 1);
        assert_eq!(metadata.authors[0].name, "Frank Herbert");
        assert_eq!(
            metadata.description.as_deref(),
            Some("A science fiction classic.")
        );
        assert_eq!(metadata.language.as_deref(), Some("en"));
        assert_eq!(metadata.publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(metadata.publication_date.as_deref(), Some("1965"));
        assert_eq!(metadata.page_count, Some(412));
        assert_eq!(
            metadata.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/8231856-L.jpg")
        );
        assert_eq!(metadata.subjects.len(), 2);
        assert!((metadata.confidence - 0.95).abs() < f32::EPSILON);

        // Check identifiers.
        let isbn13 = metadata
            .identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::Isbn13);
        assert!(isbn13.is_some());
        assert_eq!(isbn13.unwrap().value, "9780441172719");

        let isbn10 = metadata
            .identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::Isbn10);
        assert!(isbn10.is_some());
        assert_eq!(isbn10.unwrap().value, "0441172717");

        let olid = metadata
            .identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::OpenLibrary);
        assert!(olid.is_some());
        assert_eq!(olid.unwrap().value, "OL7353617M");
    }

    // ── ISBN capping ─────────────────────────────────────────────────

    #[test]
    fn search_results_cap_isbns_to_one_per_type() {
        // Simulate an OL search doc with many ISBNs aggregated across
        // editions (hardcover, paperback, audiobook, different printings).
        let json = r#"{
            "numFound": 1,
            "start": 0,
            "docs": [
                {
                    "key": "/works/OL20648239W",
                    "title": "Wicked Plants",
                    "author_name": ["Amy Stewart"],
                    "first_publish_year": 2009,
                    "isbn": [
                        "9781565126831",
                        "9780606264020",
                        "9781565129399",
                        "156512683X",
                        "0606264027",
                        "1565129393",
                        "9781616200640",
                        "9781616200190",
                        "1616200642",
                        "1616200197",
                        "9780374531137",
                        "0374531137",
                        "9781469285894",
                        "1469285894",
                        "9781622311118",
                        "1622311116"
                    ],
                    "cover_i": 6466800,
                    "publisher": ["Algonquin Books"],
                    "number_of_pages_median": 236,
                    "subject": ["Botany", "Poisonous plants"]
                }
            ]
        }"#;

        let search_response: OlSearchResponse = serde_json::from_str(json).unwrap();
        let query = MetadataQuery {
            title: Some("Wicked Plants".to_string()),
            ..Default::default()
        };

        let results = OpenLibraryProvider::parse_search_results(&search_response, &query);
        assert_eq!(results.len(), 1);

        let isbn_identifiers: Vec<_> = results[0]
            .identifiers
            .iter()
            .filter(|id| {
                matches!(
                    id.identifier_type,
                    IdentifierType::Isbn13 | IdentifierType::Isbn10
                )
            })
            .collect();

        // At most one ISBN-13 and one ISBN-10 (2 total max).
        assert!(
            isbn_identifiers.len() <= 2,
            "expected at most 2 ISBN identifiers, got {}",
            isbn_identifiers.len()
        );

        let isbn13_count = isbn_identifiers
            .iter()
            .filter(|id| id.identifier_type == IdentifierType::Isbn13)
            .count();
        let isbn10_count = isbn_identifiers
            .iter()
            .filter(|id| id.identifier_type == IdentifierType::Isbn10)
            .count();

        assert_eq!(isbn13_count, 1, "expected exactly one ISBN-13");
        assert_eq!(isbn10_count, 1, "expected exactly one ISBN-10");

        // Should be the *first* of each type encountered.
        let isbn13 = isbn_identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::Isbn13)
            .unwrap();
        assert_eq!(isbn13.value, "9781565126831");

        let isbn10 = isbn_identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::Isbn10)
            .unwrap();
        assert_eq!(isbn10.value, "156512683X");

        // OLID should still be present.
        let olid = results[0]
            .identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::OpenLibrary);
        assert!(olid.is_some());
    }

    #[test]
    fn edition_metadata_caps_isbns_to_first_of_each_type() {
        let edition = OlEdition {
            key: Some("/books/OL12345M".to_string()),
            title: Some("Test Book".to_string()),
            authors: None,
            publishers: None,
            publish_date: None,
            isbn_13: Some(vec![
                "9781111111111".to_string(),
                "9782222222222".to_string(),
                "9783333333333".to_string(),
            ]),
            isbn_10: Some(vec!["1111111111".to_string(), "2222222222".to_string()]),
            number_of_pages: None,
            covers: None,
            languages: None,
            series: None,
            works: None,
            description: None,
        };

        let metadata =
            OpenLibraryProvider::build_metadata_from_edition(&edition, None, &[], "9781111111111");

        let isbn13s: Vec<_> = metadata
            .identifiers
            .iter()
            .filter(|id| id.identifier_type == IdentifierType::Isbn13)
            .collect();
        let isbn10s: Vec<_> = metadata
            .identifiers
            .iter()
            .filter(|id| id.identifier_type == IdentifierType::Isbn10)
            .collect();

        assert_eq!(isbn13s.len(), 1, "expected exactly one ISBN-13");
        assert_eq!(isbn13s[0].value, "9781111111111");

        assert_eq!(isbn10s.len(), 1, "expected exactly one ISBN-10");
        assert_eq!(isbn10s[0].value, "1111111111");
    }

    // ── 404 handling ─────────────────────────────────────────────────

    #[test]
    fn parse_empty_search_response() {
        let json = r#"{
            "numFound": 0,
            "start": 0,
            "docs": []
        }"#;

        let search_response: OlSearchResponse = serde_json::from_str(json).unwrap();
        assert!(search_response.docs.is_empty());

        let query = MetadataQuery::default();
        let results = OpenLibraryProvider::parse_search_results(&search_response, &query);
        assert!(results.is_empty());
    }

    // ── Author ref parsing ───────────────────────────────────────────

    #[test]
    fn author_ref_direct_key() {
        let json = r#"{"key": "/authors/OL34221A"}"#;
        let author_ref: OlAuthorRef = serde_json::from_str(json).unwrap();
        assert_eq!(author_ref.key(), Some("/authors/OL34221A"));
    }

    #[test]
    fn author_ref_nested_author() {
        let json = r#"{"author": {"key": "/authors/OL34221A"}}"#;
        let author_ref: OlAuthorRef = serde_json::from_str(json).unwrap();
        assert_eq!(author_ref.key(), Some("/authors/OL34221A"));
    }

    // ── Series string parsing ─────────────────────────────────────────

    #[test]
    fn parse_series_with_comma_hash() {
        let s = parse_series_string("Harry Potter, #6");
        assert_eq!(s.name, "Harry Potter");
        assert!((s.position.unwrap() - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_with_hash_no_comma() {
        let s = parse_series_string("Discworld #12");
        assert_eq!(s.name, "Discworld");
        assert!((s.position.unwrap() - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_with_comma_no_hash() {
        let s = parse_series_string("Dune, 3");
        assert_eq!(s.name, "Dune");
        assert!((s.position.unwrap() - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_with_decimal_position() {
        let s = parse_series_string("Witcher, #1.5");
        assert_eq!(s.name, "Witcher");
        assert!((s.position.unwrap() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_no_position() {
        let s = parse_series_string("Discworld");
        assert_eq!(s.name, "Discworld");
        assert!(s.position.is_none());
    }

    #[test]
    fn parse_series_trims_whitespace() {
        let s = parse_series_string("  Harry Potter , #6  ");
        assert_eq!(s.name, "Harry Potter");
        assert!((s.position.unwrap() - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_only_number_stays_as_name() {
        // Degenerate case: raw string is just a number — treat as name, no position
        let s = parse_series_string("42");
        assert_eq!(s.name, "42");
        assert!(s.position.is_none());
    }

    // ── Integration test (live API — ignored by default) ─────────────

    #[tokio::test]
    #[ignore = "requires live network access to Open Library API"]
    async fn live_isbn_lookup_dune() {
        let mut client = MetadataHttpClient::new("0.1.0", None);
        OpenLibraryProvider::register_rate_limiter(&mut client);
        let provider = OpenLibraryProvider::new(Arc::new(client), true);

        let results = provider.lookup_isbn("9780441172719").await.unwrap();

        assert!(
            !results.is_empty(),
            "expected at least one result for Dune ISBN"
        );
        let metadata = &results[0];
        assert_eq!(metadata.provider_name, "open_library");
        // Title should be some variant of "Dune".
        let title = metadata.title.as_deref().unwrap_or("");
        assert!(
            title.to_lowercase().contains("dune"),
            "expected title containing 'dune', got: {title}"
        );
        // Should have at least one author.
        assert!(!metadata.authors.is_empty(), "expected at least one author");
        // Confidence should be high for ISBN match.
        assert!(
            metadata.confidence >= 0.9,
            "expected high confidence, got: {}",
            metadata.confidence
        );
    }

    #[tokio::test]
    #[ignore = "requires live network access to Open Library API"]
    async fn live_search_dune() {
        let mut client = MetadataHttpClient::new("0.1.0", None);
        OpenLibraryProvider::register_rate_limiter(&mut client);
        let provider = OpenLibraryProvider::new(Arc::new(client), true);

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };

        let results = provider.search(&query).await.unwrap();
        assert!(!results.is_empty(), "expected search results for Dune");
    }
}
