use std::sync::Arc;

use async_trait::async_trait;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use tracing::debug;

use archivis_core::models::IdentifierType;
use archivis_core::settings::SettingsReader;

use crate::client::MetadataHttpClient;
use crate::errors::ProviderError;
use crate::provider::MetadataProvider;
use crate::provider_names;
use crate::similarity::title_for_search;
use crate::types::{
    parse_year_from_str, MetadataQuery, ProviderAuthor, ProviderIdentifier, ProviderMetadata,
    ProviderSeries,
};

const PROVIDER_NAME: &str = provider_names::HARDCOVER;
const GRAPHQL_URL: &str = "https://api.hardcover.app/v1/graphql";
const MAX_REQUESTS_PER_MINUTE: u32 = 50;

/// GraphQL query for looking up an edition by ISBN-13.
const LOOKUP_ISBN13_QUERY: &str = r"
query LookupISBN($isbn: String!) {
  editions(where: { isbn_13: { _eq: $isbn } }, limit: 1) {
    id
    isbn_13
    isbn_10
    asin
    title
    pages
    release_date
    edition_format
    reading_format { format }
    language { language }
    publisher { name }
    image { url }
    cached_image
    cached_contributors
    book {
      id
      title
      description
      rating
      image { url }
      book_series {
        position
        featured
        series { name }
      }
      contributions {
        contribution
        author { name }
      }
      cached_tags
    }
  }
}
";

/// GraphQL query for looking up an edition by ISBN-10.
const LOOKUP_ISBN10_QUERY: &str = r"
query LookupISBN($isbn: String!) {
  editions(where: { isbn_10: { _eq: $isbn } }, limit: 1) {
    id
    isbn_13
    isbn_10
    asin
    title
    pages
    release_date
    edition_format
    reading_format { format }
    language { language }
    publisher { name }
    image { url }
    cached_image
    cached_contributors
    book {
      id
      title
      description
      rating
      image { url }
      book_series {
        position
        featured
        series { name }
      }
      contributions {
        contribution
        author { name }
      }
      cached_tags
    }
  }
}
";

/// GraphQL query for looking up an edition by ASIN.
const LOOKUP_ASIN_QUERY: &str = r"
query LookupASIN($asin: String!) {
  editions(where: { asin: { _eq: $asin } }, limit: 1) {
    id
    isbn_13
    isbn_10
    asin
    title
    pages
    release_date
    edition_format
    reading_format { format }
    language { language }
    publisher { name }
    image { url }
    cached_image
    cached_contributors
    book {
      id
      title
      description
      rating
      image { url }
      book_series {
        position
        featured
        series { name }
      }
      contributions {
        contribution
        author { name }
      }
      cached_tags
    }
  }
}
";

/// GraphQL query for searching books by text query.
///
/// Note: the `query_type` value contains a literal `"books"` that must
/// be sent as a GraphQL string argument. We use a Rust raw string here
/// so the inner quotes are preserved without escaping.
const SEARCH_QUERY: &str = "
query SearchBooks($query: String!) {
  search(query: $query, query_type: \"books\", per_page: 5) {
    ids
    results
  }
}
";

/// GraphQL query for fetching full book details by IDs.
const GET_BOOKS_QUERY: &str = r"
query GetBooks($ids: [Int!]!) {
  books(where: { id: { _in: $ids } }) {
    id
    title
    description
    rating
    contributions {
      contribution
      author { name }
    }
    book_series {
      position
      featured
      series { name }
    }
    image { url }
    default_physical_edition {
      isbn_13
      isbn_10
      pages
      release_date
      edition_format
      reading_format { format }
      publisher { name }
      language { language }
      image { url }
      cached_image
    }
    default_ebook_edition {
      isbn_13
      isbn_10
      pages
      release_date
      edition_format
      reading_format { format }
      publisher { name }
      language { language }
      image { url }
      cached_image
    }
    cached_tags
  }
}
";

/// Hardcover metadata provider.
///
/// Uses the Hardcover GraphQL API for ISBN lookups, title+author search,
/// and cover image retrieval. Requires a Bearer token for authentication.
///
/// Reads `metadata.enabled`, `metadata.hardcover.enabled`, and
/// `metadata.hardcover.api_token` from settings at call time so that
/// runtime changes via the admin UI take effect immediately.
pub struct HardcoverProvider {
    client: Arc<MetadataHttpClient>,
    settings: Arc<dyn SettingsReader>,
}

impl HardcoverProvider {
    /// Create a new Hardcover provider backed by live settings.
    pub fn new(client: Arc<MetadataHttpClient>, settings: Arc<dyn SettingsReader>) -> Self {
        Self { client, settings }
    }

    /// Read the current API token from settings.
    fn api_token(&self) -> Option<String> {
        self.settings
            .get_setting("metadata.hardcover.api_token")
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
    }

    /// Register this provider's rate limiter with the shared HTTP client.
    /// Must be called before making requests.
    pub fn register_rate_limiter(client: &mut MetadataHttpClient) {
        client.register_provider(PROVIDER_NAME, MAX_REQUESTS_PER_MINUTE);
    }

    /// Register a custom rate limit with the shared HTTP client.
    pub fn register_rate_limiter_with_limit(client: &mut MetadataHttpClient, max_rpm: u32) {
        client.register_provider(PROVIDER_NAME, max_rpm);
    }

    /// Execute a GraphQL query against the Hardcover API.
    async fn graphql_request<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T, ProviderError> {
        let token = self.api_token().ok_or_else(|| {
            ProviderError::NotConfigured("Hardcover API token not configured".to_string())
        })?;

        let body = GraphqlRequest {
            query: query.to_string(),
            variables,
        };

        let response = self
            .client
            .post_json_with_headers(
                PROVIDER_NAME,
                GRAPHQL_URL,
                &body,
                &[("Authorization", &format!("Bearer {token}"))],
            )
            .await?;

        let status = response.status().as_u16();

        if status == 401 {
            return Err(ProviderError::NotConfigured(
                "Invalid or expired Hardcover API token".to_string(),
            ));
        }

        if !response.status().is_success() {
            return Err(ProviderError::ApiError {
                status,
                message: format!("Hardcover API returned HTTP {status}"),
            });
        }

        let response_body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::ParseError(format!("failed to parse response: {e}")))?;

        // Check for GraphQL errors in the response body.
        if let Some(errors) = response_body.get("errors") {
            if let Some(arr) = errors.as_array() {
                if !arr.is_empty() {
                    let messages: Vec<String> = arr
                        .iter()
                        .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                        .map(String::from)
                        .collect();
                    let combined = messages.join("; ");
                    return Err(ProviderError::ApiError {
                        status: 200,
                        message: format!("GraphQL errors: {combined}"),
                    });
                }
            }
        }

        let data = response_body.get("data").ok_or_else(|| {
            ProviderError::ParseError("response missing 'data' field".to_string())
        })?;

        serde_json::from_value(data.clone())
            .map_err(|e| ProviderError::ParseError(format!("failed to parse data: {e}")))
    }

    /// Look up an edition by ISBN, trying ISBN-13 first, then ISBN-10.
    async fn fetch_isbn_edition(
        &self,
        isbn: &str,
    ) -> Result<Option<ProviderMetadata>, ProviderError> {
        // Try ISBN-13 first.
        let variables = serde_json::json!({ "isbn": isbn });
        let response: EditionsResponse =
            self.graphql_request(LOOKUP_ISBN13_QUERY, variables).await?;

        if let Some(edition) = response.editions.into_iter().next() {
            return Ok(Some(Self::build_metadata_from_edition(&edition, isbn)));
        }

        // Fall back to ISBN-10.
        debug!(isbn = isbn, "ISBN-13 not found, trying ISBN-10");
        let variables = serde_json::json!({ "isbn": isbn });
        let response: EditionsResponse =
            self.graphql_request(LOOKUP_ISBN10_QUERY, variables).await?;

        Ok(response
            .editions
            .into_iter()
            .next()
            .map(|edition| Self::build_metadata_from_edition(&edition, isbn)))
    }

    /// Look up an edition by ASIN.
    async fn fetch_asin_edition(
        &self,
        asin: &str,
    ) -> Result<Option<ProviderMetadata>, ProviderError> {
        // Normalize: strip whitespace, uppercase — ASINs are case-insensitive
        // but the GraphQL _eq filter is exact-match.
        let normalized: String = asin
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let variables = serde_json::json!({ "asin": normalized });
        let response: EditionsResponse = self.graphql_request(LOOKUP_ASIN_QUERY, variables).await?;

        Ok(response
            .editions
            .into_iter()
            .next()
            .map(|edition| Self::build_metadata_from_asin_edition(&edition, asin)))
    }

    /// Extract common fields from a Hardcover edition, returning a
    /// `ProviderMetadata` with an empty `identifiers` vec. Callers fill in
    /// identifiers via `build_edition_identifiers` or
    /// `build_asin_edition_identifiers`.
    fn extract_edition_fields(edition: &HcEdition) -> ProviderMetadata {
        let book = edition.book.as_ref();

        let title = edition
            .title
            .clone()
            .or_else(|| book.and_then(|b| b.title.clone()));

        let authors =
            extract_authors_from_contributions(book.and_then(|b| b.contributions.as_ref()));

        let description = book.and_then(|b| b.description.clone());
        let series = extract_series(book.and_then(|b| b.book_series.as_ref()));
        let publisher = edition.publisher.as_ref().map(|p| p.name.clone());
        let page_count = edition.pages;
        let publication_year = edition
            .release_date
            .as_deref()
            .and_then(parse_year_from_str);

        #[allow(clippy::cast_possible_truncation)]
        let rating = book.and_then(|b| b.rating).map(|r| r as f32);

        let subjects = book
            .and_then(|b| parse_cached_tags(b.cached_tags.as_ref()))
            .unwrap_or_default();

        // Prefer high-res `image.url` over `cached_image` thumbnail.
        let cover_url = edition
            .image
            .as_ref()
            .and_then(|i| i.url.clone())
            .or_else(|| parse_cached_image(edition.cached_image.as_ref()))
            .or_else(|| book.and_then(|b| b.image.as_ref().and_then(|i| i.url.clone())))
            .or_else(|| book.and_then(|b| parse_cached_image(b.cached_image.as_ref())));

        let language = edition
            .language
            .as_ref()
            .and_then(|l| normalize_language(&l.language));

        let physical_format = edition
            .reading_format
            .as_ref()
            .and_then(|rf| rf.format.clone())
            .or_else(|| edition.edition_format.clone());

        ProviderMetadata {
            provider_name: PROVIDER_NAME.to_string(),
            title,
            subtitle: None, // Hardcover API doesn't provide subtitles.
            authors,
            description,
            language,
            publisher,
            publication_year,
            identifiers: Vec::new(),
            subjects,
            series,
            page_count,
            cover_url,
            rating,
            physical_format,
            confidence: 0.95,
        }
    }

    /// Build `ProviderMetadata` from a Hardcover edition looked up by ASIN.
    fn build_metadata_from_asin_edition(
        edition: &HcEdition,
        queried_asin: &str,
    ) -> ProviderMetadata {
        let book_id = edition.book.as_ref().and_then(|b| b.id);
        let mut meta = Self::extract_edition_fields(edition);
        meta.identifiers = build_asin_edition_identifiers(edition, book_id, queried_asin);
        meta
    }

    /// Build `ProviderMetadata` from a Hardcover edition looked up by ISBN.
    fn build_metadata_from_edition(edition: &HcEdition, queried_isbn: &str) -> ProviderMetadata {
        let book_id = edition.book.as_ref().and_then(|b| b.id);
        let mut meta = Self::extract_edition_fields(edition);
        meta.identifiers = build_edition_identifiers(edition, book_id, queried_isbn);
        meta
    }

    /// Build `ProviderMetadata` from a Hardcover book (work) response
    /// returned by the search follow-up query.
    fn build_metadata_from_book(book: &HcBook, query: &MetadataQuery) -> ProviderMetadata {
        let title = book.title.clone();

        let authors = extract_authors_from_contributions(book.contributions.as_ref());

        let description = book.description.clone();

        let series = extract_series(book.book_series.as_ref());

        #[allow(clippy::cast_possible_truncation)]
        let rating = book.rating.map(|r| r as f32);

        let subjects = parse_cached_tags(book.cached_tags.as_ref()).unwrap_or_default();

        // Prefer ebook edition (Archivis imports digital files), fall back to physical.
        let edition = book
            .default_ebook_edition
            .as_ref()
            .or(book.default_physical_edition.as_ref());

        let publisher = edition.and_then(|e| e.publisher.as_ref().map(|p| p.name.clone()));
        let page_count = edition.and_then(|e| e.pages);
        let publication_year = edition
            .and_then(|e| e.release_date.as_deref())
            .and_then(parse_year_from_str);
        let language = edition
            .and_then(|e| e.language.as_ref())
            .and_then(|l| normalize_language(&l.language));

        // Cover URL: prefer high-res `image.url` over `cached_image` thumbnail.
        let cover_url = edition
            .and_then(|e| {
                e.image
                    .as_ref()
                    .and_then(|i| i.url.clone())
                    .or_else(|| parse_cached_image(e.cached_image.as_ref()))
            })
            .or_else(|| {
                let fallback = book
                    .default_physical_edition
                    .as_ref()
                    .or(book.default_ebook_edition.as_ref());
                fallback.and_then(|e| {
                    e.image
                        .as_ref()
                        .and_then(|i| i.url.clone())
                        .or_else(|| parse_cached_image(e.cached_image.as_ref()))
                })
            })
            .or_else(|| book.image.as_ref().and_then(|i| i.url.clone()));

        // `physical_format`: use `edition_format` (e.g. "paperback") which is meaningful.
        // `reading_format` ("Read"/"Listened") is a modality label, not a physical format.
        let physical_format = edition.and_then(|e| e.edition_format.clone());

        // Collect ISBNs from both editions to avoid losing portable identifiers.
        let mut identifiers = Vec::new();
        for ed in [
            book.default_ebook_edition.as_ref(),
            book.default_physical_edition.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(ref isbn13) = ed.isbn_13 {
                if !isbn13.is_empty()
                    && !identifiers
                        .iter()
                        .any(|i: &ProviderIdentifier| i.value == *isbn13)
                {
                    identifiers.push(ProviderIdentifier {
                        identifier_type: IdentifierType::Isbn13,
                        value: isbn13.clone(),
                    });
                }
            }
            if let Some(ref isbn10) = ed.isbn_10 {
                if !isbn10.is_empty()
                    && !identifiers
                        .iter()
                        .any(|i: &ProviderIdentifier| i.value == *isbn10)
                {
                    identifiers.push(ProviderIdentifier {
                        identifier_type: IdentifierType::Isbn10,
                        value: isbn10.clone(),
                    });
                }
            }
        }

        if let Some(book_id) = book.id {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Hardcover,
                value: book_id.to_string(),
            });
        }

        // Compute confidence based on title match quality.
        let confidence =
            compute_search_confidence(book.title.as_deref(), query.title.as_deref(), query);

        ProviderMetadata {
            provider_name: PROVIDER_NAME.to_string(),
            title,
            subtitle: None, // Hardcover API doesn't provide subtitles.
            authors,
            description,
            language,
            publisher,
            publication_year,
            identifiers,
            subjects,
            series,
            page_count,
            cover_url,
            rating,
            physical_format,
            confidence,
        }
    }
}

#[async_trait]
impl MetadataProvider for HardcoverProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn is_available(&self) -> bool {
        let global = self
            .settings
            .get_setting("metadata.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let provider = self
            .settings
            .get_setting("metadata.hardcover.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        global && provider && self.api_token().is_some()
    }

    async fn lookup_isbn(&self, isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.is_available() {
            return Err(ProviderError::NotConfigured(
                "Hardcover provider is not available (disabled or missing API token)".to_string(),
            ));
        }

        Ok(self
            .fetch_isbn_edition(isbn)
            .await?
            .map_or_else(Vec::new, |metadata| vec![metadata]))
    }

    async fn lookup_asin(&self, asin: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.is_available() {
            return Err(ProviderError::NotConfigured(
                "Hardcover provider is not available (disabled or missing API token)".to_string(),
            ));
        }

        Ok(self
            .fetch_asin_edition(asin)
            .await?
            .map_or_else(Vec::new, |metadata| vec![metadata]))
    }

    async fn search(&self, query: &MetadataQuery) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.is_available() {
            return Err(ProviderError::NotConfigured(
                "Hardcover provider is not available (disabled or missing API token)".to_string(),
            ));
        }

        // Build search string from title and author.
        let search_string = build_search_string(query);
        if search_string.is_empty() {
            return Ok(Vec::new());
        }

        // Step 1: Search for IDs.
        let variables = serde_json::json!({ "query": search_string });
        let search_response: SearchResponse = self.graphql_request(SEARCH_QUERY, variables).await?;

        let Some(search) = search_response.search else {
            return Ok(Vec::new());
        };

        let ids = search.ids.unwrap_or_default();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Step 2: Fetch full book details.
        let variables = serde_json::json!({ "ids": ids });
        let books_response: BooksResponse =
            self.graphql_request(GET_BOOKS_QUERY, variables).await?;

        let books = books_response.books.unwrap_or_default();

        let mut results: Vec<ProviderMetadata> = books
            .iter()
            .map(|book| Self::build_metadata_from_book(book, query))
            .collect();

        // Sort by confidence descending.
        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    async fn fetch_cover(&self, cover_url: &str) -> Result<Vec<u8>, ProviderError> {
        // Cover images are served from a CDN, no special auth needed.
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

// ── GraphQL request/response types ──────────────────────────────────

/// Deserialize an `Option<Vec<i64>>` that tolerates string-encoded integers.
///
/// The Hardcover API declares `ids: [Int]` in its GraphQL schema but the
/// actual JSON payload sometimes returns the values as strings
/// (e.g. `["788634"]` instead of `[788634]`). This visitor accepts both
/// representations as well as mixed arrays.
fn deserialize_ids<'de, D>(deserializer: D) -> Result<Option<Vec<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct IdsVisitor;

    impl<'de> Visitor<'de> for IdsVisitor {
        type Value = Option<Vec<i64>>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null or an array of integers/string-encoded integers")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut ids = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(val) = seq.next_element::<serde_json::Value>()? {
                let id = match &val {
                    serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| {
                        de::Error::custom(format!("number is not a valid i64: {n}"))
                    })?,
                    serde_json::Value::String(s) => s.parse::<i64>().map_err(|_| {
                        de::Error::custom(format!("string is not a valid i64: \"{s}\""))
                    })?,
                    other => {
                        return Err(de::Error::custom(format!(
                            "expected integer or string, got: {other}"
                        )));
                    }
                };
                ids.push(id);
            }
            Ok(Some(ids))
        }
    }

    deserializer.deserialize_any(IdsVisitor)
}

/// Deserialize an `Option<i64>` that tolerates a string-encoded integer.
fn deserialize_optional_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptI64Visitor;

    impl Visitor<'_> for OptI64Visitor {
        type Value = Option<i64>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null, an integer, or a string-encoded integer")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            i64::try_from(v)
                .map(Some)
                .map_err(|_| de::Error::custom(format!("u64 out of i64 range: {v}")))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            v.parse::<i64>()
                .map(Some)
                .map_err(|_| de::Error::custom(format!("string is not a valid i64: \"{v}\"")))
        }
    }

    deserializer.deserialize_any(OptI64Visitor)
}

/// GraphQL request body.
#[derive(Serialize)]
struct GraphqlRequest {
    query: String,
    variables: serde_json::Value,
}

/// Response wrapper for edition lookups.
#[derive(Debug, Deserialize)]
struct EditionsResponse {
    #[serde(default)]
    editions: Vec<HcEdition>,
}

/// Hardcover edition from GraphQL response.
#[derive(Debug, Deserialize)]
struct HcEdition {
    #[allow(dead_code)]
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    id: Option<i64>,
    isbn_13: Option<String>,
    isbn_10: Option<String>,
    asin: Option<String>,
    title: Option<String>,
    pages: Option<i32>,
    release_date: Option<String>,
    edition_format: Option<String>,
    reading_format: Option<HcReadingFormat>,
    language: Option<HcLanguage>,
    publisher: Option<HcPublisher>,
    image: Option<HcImage>,
    cached_image: Option<serde_json::Value>,
    #[allow(dead_code)]
    cached_contributors: Option<serde_json::Value>,
    book: Option<HcBookInEdition>,
}

/// High-resolution image from Hardcover's `image` relationship.
#[derive(Debug, Deserialize)]
struct HcImage {
    url: Option<String>,
}

/// Structured reading format from Hardcover (e.g. `"Audiobook"`, `"Physical"`).
#[derive(Debug, Deserialize)]
struct HcReadingFormat {
    format: Option<String>,
}

/// Book (work) data embedded in an edition response.
#[derive(Debug, Deserialize)]
struct HcBookInEdition {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    id: Option<i64>,
    title: Option<String>,
    description: Option<String>,
    rating: Option<f64>,
    book_series: Option<Vec<HcBookSeries>>,
    contributions: Option<Vec<HcContribution>>,
    cached_tags: Option<serde_json::Value>,
    image: Option<HcImage>,
    #[allow(dead_code)]
    cached_image: Option<serde_json::Value>,
}

/// Response wrapper for search queries.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    search: Option<HcSearch>,
}

/// Hardcover search result.
#[derive(Debug, Deserialize)]
struct HcSearch {
    #[serde(default, deserialize_with = "deserialize_ids")]
    ids: Option<Vec<i64>>,
    #[allow(dead_code)]
    results: Option<serde_json::Value>,
}

/// Response wrapper for books query (search follow-up).
#[derive(Debug, Deserialize)]
struct BooksResponse {
    books: Option<Vec<HcBook>>,
}

/// Hardcover book (work) from the books query.
#[derive(Debug, Deserialize)]
struct HcBook {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    id: Option<i64>,
    title: Option<String>,
    description: Option<String>,
    rating: Option<f64>,
    contributions: Option<Vec<HcContribution>>,
    book_series: Option<Vec<HcBookSeries>>,
    default_physical_edition: Option<HcDefaultEdition>,
    default_ebook_edition: Option<HcDefaultEdition>,
    image: Option<HcImage>,
    cached_tags: Option<serde_json::Value>,
}

/// Default edition in a book response.
#[derive(Debug, Deserialize)]
struct HcDefaultEdition {
    isbn_13: Option<String>,
    isbn_10: Option<String>,
    pages: Option<i32>,
    release_date: Option<String>,
    edition_format: Option<String>,
    #[allow(dead_code)]
    reading_format: Option<HcReadingFormat>,
    publisher: Option<HcPublisher>,
    language: Option<HcLanguage>,
    image: Option<HcImage>,
    cached_image: Option<serde_json::Value>,
}

/// Book-series junction.
#[derive(Debug, Deserialize)]
struct HcBookSeries {
    position: Option<f64>,
    featured: Option<bool>,
    series: Option<HcSeries>,
}

/// Series name wrapper.
#[derive(Debug, Deserialize)]
struct HcSeries {
    name: String,
}

/// Author contribution.
#[derive(Debug, Deserialize)]
struct HcContribution {
    contribution: Option<String>,
    author: Option<HcAuthor>,
}

/// Author name wrapper.
#[derive(Debug, Deserialize)]
struct HcAuthor {
    name: String,
}

/// Language name wrapper.
///
/// The Hardcover `languages` table uses `language` (not `name`) for the
/// human-readable language string.
#[derive(Debug, Deserialize)]
struct HcLanguage {
    language: String,
}

/// Publisher name wrapper.
#[derive(Debug, Deserialize)]
struct HcPublisher {
    name: String,
}

// ── Helper functions ─────────────────────────────────────────────────

/// Extract authors from a contributions array.
fn extract_authors_from_contributions(
    contributions: Option<&Vec<HcContribution>>,
) -> Vec<ProviderAuthor> {
    contributions
        .map(|contribs| {
            contribs
                .iter()
                .filter_map(|c| {
                    c.author.as_ref().map(|a| ProviderAuthor {
                        name: a.name.clone(),
                        role: Some(c.contribution.as_deref().unwrap_or("Author").to_lowercase()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Extract series information from a `book_series` array,
/// preferring the featured entry.
fn extract_series(book_series: Option<&Vec<HcBookSeries>>) -> Option<ProviderSeries> {
    book_series.and_then(|series_list| {
        let featured = series_list.iter().find(|s| s.featured.unwrap_or(false));
        let entry = featured.or_else(|| series_list.first());
        entry.and_then(|s| {
            s.series.as_ref().map(|ser| {
                #[allow(clippy::cast_possible_truncation)]
                ProviderSeries {
                    name: ser.name.clone(),
                    position: s.position.map(|p| p as f32),
                }
            })
        })
    })
}

/// Collect the common identifiers from an edition (`isbn_13`, `isbn_10`,
/// `asin`, hardcover book ID). Callers append their queried-identifier
/// guarantee.
fn collect_edition_identifiers(
    edition: &HcEdition,
    book_id: Option<i64>,
) -> Vec<ProviderIdentifier> {
    let mut identifiers = Vec::new();

    if let Some(ref isbn13) = edition.isbn_13 {
        if !isbn13.is_empty() {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: isbn13.clone(),
            });
        }
    }

    if let Some(ref isbn10) = edition.isbn_10 {
        if !isbn10.is_empty() {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Isbn10,
                value: isbn10.clone(),
            });
        }
    }

    if let Some(ref asin) = edition.asin {
        if !asin.is_empty() {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Asin,
                value: asin.clone(),
            });
        }
    }

    if let Some(id) = book_id {
        identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Hardcover,
            value: id.to_string(),
        });
    }

    identifiers
}

/// Build identifiers from an edition looked up by ISBN.
/// Ensures the queried ISBN is present in the identifiers list.
fn build_edition_identifiers(
    edition: &HcEdition,
    book_id: Option<i64>,
    queried_isbn: &str,
) -> Vec<ProviderIdentifier> {
    let mut identifiers = collect_edition_identifiers(edition, book_id);

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

    identifiers
}

/// Build identifiers from an edition looked up by ASIN.
/// Ensures the queried ASIN is present in the identifiers list.
fn build_asin_edition_identifiers(
    edition: &HcEdition,
    book_id: Option<i64>,
    queried_asin: &str,
) -> Vec<ProviderIdentifier> {
    let mut identifiers = collect_edition_identifiers(edition, book_id);

    let queried_present = identifiers.iter().any(|id| {
        id.identifier_type == IdentifierType::Asin && id.value.eq_ignore_ascii_case(queried_asin)
    });
    if !queried_present {
        identifiers.push(ProviderIdentifier {
            identifier_type: IdentifierType::Asin,
            value: queried_asin.to_string(),
        });
    }

    identifiers
}

/// Parse `cached_tags` from a JSON value.
///
/// The `cached_tags` field is a JSON value that may be:
/// - A JSON array of objects with a `"tag"` field
/// - A JSON array of strings
/// - A JSON string that needs secondary parsing
/// - null/missing
fn parse_cached_tags(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let value = value?;

    // If it's a string, try to parse it as JSON first.
    if let serde_json::Value::String(s) = value {
        return serde_json::from_str::<serde_json::Value>(s)
            .ok()
            .and_then(|v| parse_tags_from_array(&v));
    }

    parse_tags_from_array(value)
}

/// Extract tag names from a JSON array value.
fn parse_tags_from_array(value: &serde_json::Value) -> Option<Vec<String>> {
    let arr = value.as_array()?;
    let tags: Vec<String> = arr
        .iter()
        .filter_map(|item| {
            // Try as object with "tag" field.
            if let Some(tag) = item.get("tag").and_then(|t| t.as_str()) {
                return Some(tag.to_string());
            }
            // Try as object with "name" field.
            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                return Some(name.to_string());
            }
            // Try as plain string.
            item.as_str().map(String::from)
        })
        .collect();

    if tags.is_empty() {
        None
    } else {
        Some(tags)
    }
}

/// Parse `cached_image` from a JSON value to extract a cover URL.
///
/// The `cached_image` field is a JSON value that may be:
/// - A JSON string that is itself a URL
/// - A JSON string that needs secondary parsing into an object
/// - A JSON object with a `"url"` or `"image"` field
/// - null/missing
fn parse_cached_image(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;

    match value {
        serde_json::Value::String(s) => {
            // If the string looks like a URL, use it directly.
            if s.starts_with("http://") || s.starts_with("https://") {
                return Some(s.clone());
            }
            // Try parsing as JSON object.
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                return extract_image_url(&parsed);
            }
            None
        }
        serde_json::Value::Object(_) => extract_image_url(value),
        _ => None,
    }
}

/// Extract an image URL from a JSON object.
fn extract_image_url(value: &serde_json::Value) -> Option<String> {
    // Try "url" key first, then "image".
    if let Some(url) = value.get("url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }
    if let Some(url) = value.get("image").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }
    None
}

/// Normalize a language name to an ISO 639-1 code.
fn normalize_language(name: &str) -> Option<String> {
    match name.to_lowercase().as_str() {
        "english" => Some("en".to_string()),
        "french" | "français" => Some("fr".to_string()),
        "german" | "deutsch" => Some("de".to_string()),
        "spanish" | "español" => Some("es".to_string()),
        "italian" | "italiano" => Some("it".to_string()),
        "portuguese" | "português" => Some("pt".to_string()),
        "russian" | "русский" => Some("ru".to_string()),
        "japanese" | "日本語" => Some("ja".to_string()),
        "chinese" | "中文" => Some("zh".to_string()),
        "korean" | "한국어" => Some("ko".to_string()),
        "arabic" | "العربية" => Some("ar".to_string()),
        "hindi" | "हिन्दी" => Some("hi".to_string()),
        "dutch" | "nederlands" => Some("nl".to_string()),
        "polish" | "polski" => Some("pl".to_string()),
        "swedish" | "svenska" => Some("sv".to_string()),
        "norwegian" | "norsk" => Some("no".to_string()),
        "danish" | "dansk" => Some("da".to_string()),
        "finnish" | "suomi" => Some("fi".to_string()),
        "turkish" | "türkçe" => Some("tr".to_string()),
        "czech" | "čeština" => Some("cs".to_string()),
        "hungarian" | "magyar" => Some("hu".to_string()),
        "romanian" | "română" => Some("ro".to_string()),
        "greek" | "ελληνικά" => Some("el".to_string()),
        "hebrew" | "עברית" => Some("he".to_string()),
        "thai" | "ไทย" => Some("th".to_string()),
        "vietnamese" | "tiếng việt" => Some("vi".to_string()),
        "ukrainian" | "українська" => Some("uk".to_string()),
        "catalan" | "català" => Some("ca".to_string()),
        "bulgarian" | "български" => Some("bg".to_string()),
        "croatian" | "hrvatski" => Some("hr".to_string()),
        "serbian" | "српски" => Some("sr".to_string()),
        "slovenian" | "slovenščina" => Some("sl".to_string()),
        "lithuanian" | "lietuvių" => Some("lt".to_string()),
        "latvian" | "latviešu" => Some("lv".to_string()),
        "estonian" | "eesti" => Some("et".to_string()),
        "indonesian" | "bahasa indonesia" => Some("id".to_string()),
        "malay" | "bahasa melayu" => Some("ms".to_string()),
        "persian" | "فارسی" => Some("fa".to_string()),
        "urdu" | "اردو" => Some("ur".to_string()),
        _ => {
            // If the name is already a 2-letter code, return it.
            let trimmed = name.trim().to_lowercase();
            if trimmed.len() == 2 && trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
                Some(trimmed)
            } else {
                None
            }
        }
    }
}

/// Build a search string from a `MetadataQuery`.
fn build_search_string(query: &MetadataQuery) -> String {
    let mut parts = Vec::new();
    if let Some(ref title) = query.title {
        parts.push(title_for_search(title).to_string());
    }
    if let Some(ref author) = query.author {
        parts.push(author.clone());
    }
    parts.join(" ")
}

/// Compute confidence for a search result based on how well it matches
/// the query.
fn compute_search_confidence(
    result_title: Option<&str>,
    query_title: Option<&str>,
    query: &MetadataQuery,
) -> f32 {
    // If the query had an ISBN and result matches, higher confidence.
    if query.isbn.is_some() {
        return 0.8;
    }

    // Compare titles for fuzzy matching.
    match (result_title, query_title) {
        (Some(result), Some(query_t)) => {
            let r = normalize_for_comparison(result);
            let q = normalize_for_comparison(query_t);

            if r == q {
                0.8
            } else if r.contains(&q) || q.contains(&r) {
                0.7
            } else {
                // Try main-title-only comparison (handles embedded subtitles).
                let q_main = normalize_for_comparison(title_for_search(query_t));
                if q_main != q && (r == q_main || r.contains(&q_main) || q_main.contains(&r)) {
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
    use crate::test_util::StubSettings;

    fn stub_settings(enabled: bool, token: Option<&str>) -> Arc<StubSettings> {
        let mut entries = vec![
            ("metadata.enabled", serde_json::Value::Bool(true)),
            (
                "metadata.hardcover.enabled",
                serde_json::Value::Bool(enabled),
            ),
        ];
        if let Some(t) = token {
            entries.push((
                "metadata.hardcover.api_token",
                serde_json::Value::String(t.to_string()),
            ));
        }
        Arc::new(StubSettings::new(entries))
    }

    // ── is_available ────────────────────────────────────────────────

    #[test]
    fn is_available_returns_false_when_no_token() {
        let client = Arc::new(MetadataHttpClient::new("0.1.0", None));
        let provider = HardcoverProvider::new(client, stub_settings(true, None));
        assert!(!provider.is_available());
    }

    #[test]
    fn is_available_returns_false_when_disabled() {
        let client = Arc::new(MetadataHttpClient::new("0.1.0", None));
        let provider = HardcoverProvider::new(client, stub_settings(false, Some("test-token")));
        assert!(!provider.is_available());
    }

    #[test]
    fn is_available_returns_true_with_token_and_enabled() {
        let client = Arc::new(MetadataHttpClient::new("0.1.0", None));
        let provider = HardcoverProvider::new(client, stub_settings(true, Some("test-token")));
        assert!(provider.is_available());
    }

    #[test]
    fn is_available_returns_false_when_global_metadata_disabled() {
        let client = Arc::new(MetadataHttpClient::new("0.1.0", None));
        let settings = Arc::new(StubSettings::new(vec![
            ("metadata.enabled", serde_json::Value::Bool(false)),
            ("metadata.hardcover.enabled", serde_json::Value::Bool(true)),
            (
                "metadata.hardcover.api_token",
                serde_json::Value::String("test-token".to_string()),
            ),
        ]));
        let provider = HardcoverProvider::new(client, settings);
        assert!(!provider.is_available());
    }

    #[test]
    fn provider_name_is_hardcover() {
        let client = Arc::new(MetadataHttpClient::new("0.1.0", None));
        let provider = HardcoverProvider::new(client, stub_settings(false, None));
        assert_eq!(provider.name(), "hardcover");
    }

    // ── Language normalization ───────────────────────────────────────

    #[test]
    fn language_normalization_common() {
        assert_eq!(normalize_language("English"), Some("en".to_string()));
        assert_eq!(normalize_language("French"), Some("fr".to_string()));
        assert_eq!(normalize_language("German"), Some("de".to_string()));
        assert_eq!(normalize_language("Spanish"), Some("es".to_string()));
        assert_eq!(normalize_language("Japanese"), Some("ja".to_string()));
        assert_eq!(normalize_language("Chinese"), Some("zh".to_string()));
    }

    #[test]
    fn language_normalization_case_insensitive() {
        assert_eq!(normalize_language("english"), Some("en".to_string()));
        assert_eq!(normalize_language("ENGLISH"), Some("en".to_string()));
        assert_eq!(normalize_language("English"), Some("en".to_string()));
    }

    #[test]
    fn language_normalization_two_letter_code() {
        assert_eq!(normalize_language("en"), Some("en".to_string()));
        assert_eq!(normalize_language("fr"), Some("fr".to_string()));
    }

    #[test]
    fn language_normalization_unknown() {
        assert_eq!(normalize_language("Klingon"), None);
        assert_eq!(normalize_language(""), None);
    }

    // ── cached_tags parsing ─────────────────────────────────────────

    #[test]
    fn parse_cached_tags_array_of_objects_with_tag() {
        let value = serde_json::json!([
            {"tag": "Science Fiction"},
            {"tag": "Space Opera"},
            {"tag": "Classic"}
        ]);
        let tags = parse_cached_tags(Some(&value)).unwrap();
        assert_eq!(tags, vec!["Science Fiction", "Space Opera", "Classic"]);
    }

    #[test]
    fn parse_cached_tags_array_of_strings() {
        let value = serde_json::json!(["Science Fiction", "Fantasy"]);
        let tags = parse_cached_tags(Some(&value)).unwrap();
        assert_eq!(tags, vec!["Science Fiction", "Fantasy"]);
    }

    #[test]
    fn parse_cached_tags_json_string() {
        let value = serde_json::Value::String(
            r#"[{"tag": "Science Fiction"}, {"tag": "Classic"}]"#.to_string(),
        );
        let tags = parse_cached_tags(Some(&value)).unwrap();
        assert_eq!(tags, vec!["Science Fiction", "Classic"]);
    }

    #[test]
    fn parse_cached_tags_none() {
        assert!(parse_cached_tags(None).is_none());
    }

    #[test]
    fn parse_cached_tags_empty_array() {
        let value = serde_json::json!([]);
        assert!(parse_cached_tags(Some(&value)).is_none());
    }

    #[test]
    fn parse_cached_tags_array_of_objects_with_name() {
        let value = serde_json::json!([
            {"name": "Science Fiction"},
            {"name": "Fantasy"}
        ]);
        let tags = parse_cached_tags(Some(&value)).unwrap();
        assert_eq!(tags, vec!["Science Fiction", "Fantasy"]);
    }

    // ── cached_image parsing ────────────────────────────────────────

    #[test]
    fn parse_cached_image_url_string() {
        let value =
            serde_json::Value::String("https://cdn.hardcover.app/covers/123.jpg".to_string());
        let url = parse_cached_image(Some(&value)).unwrap();
        assert_eq!(url, "https://cdn.hardcover.app/covers/123.jpg");
    }

    #[test]
    fn parse_cached_image_json_string_with_url_key() {
        let value = serde_json::Value::String(
            r#"{"url": "https://cdn.hardcover.app/covers/123.jpg"}"#.to_string(),
        );
        let url = parse_cached_image(Some(&value)).unwrap();
        assert_eq!(url, "https://cdn.hardcover.app/covers/123.jpg");
    }

    #[test]
    fn parse_cached_image_object_with_url() {
        let value = serde_json::json!({
            "url": "https://cdn.hardcover.app/covers/123.jpg"
        });
        let url = parse_cached_image(Some(&value)).unwrap();
        assert_eq!(url, "https://cdn.hardcover.app/covers/123.jpg");
    }

    #[test]
    fn parse_cached_image_object_with_image() {
        let value = serde_json::json!({
            "image": "https://cdn.hardcover.app/covers/456.jpg"
        });
        let url = parse_cached_image(Some(&value)).unwrap();
        assert_eq!(url, "https://cdn.hardcover.app/covers/456.jpg");
    }

    #[test]
    fn parse_cached_image_none() {
        assert!(parse_cached_image(None).is_none());
    }

    #[test]
    fn parse_cached_image_null() {
        let value = serde_json::Value::Null;
        assert!(parse_cached_image(Some(&value)).is_none());
    }

    // ── Edition response parsing ────────────────────────────────────

    #[test]
    fn parse_edition_response() {
        let json = r#"{
            "editions": [{
                "id": 12345,
                "isbn_13": "9780441172719",
                "isbn_10": "0441172717",
                "asin": "B00GQAIJ2C",
                "title": "Dune",
                "pages": 412,
                "release_date": "1965-08-01",
                "edition_format": "paperback",
                "reading_format": null,
                "language": { "language": "English" },
                "publisher": { "name": "Chilton Books" },
                "cached_image": "{\"url\": \"https://cdn.hardcover.app/covers/dune.jpg\"}",
                "cached_contributors": null,
                "book": {
                    "id": 67890,
                    "title": "Dune",
                    "description": "Set on the desert planet Arrakis.",
                    "rating": 4.25,
                    "book_series": [{
                        "position": 1.0,
                        "featured": true,
                        "series": { "name": "Dune" }
                    }],
                    "contributions": [
                        { "contribution": "Author", "author": { "name": "Frank Herbert" } }
                    ],
                    "cached_tags": "[{\"tag\": \"Science Fiction\"}, {\"tag\": \"Space Opera\"}]"
                }
            }]
        }"#;

        let response: EditionsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.editions.len(), 1);

        let edition = &response.editions[0];
        assert_eq!(edition.isbn_13.as_deref(), Some("9780441172719"));
        assert_eq!(edition.isbn_10.as_deref(), Some("0441172717"));
        assert_eq!(edition.asin.as_deref(), Some("B00GQAIJ2C"));
        assert_eq!(edition.title.as_deref(), Some("Dune"));
        assert_eq!(edition.pages, Some(412));
        assert_eq!(edition.release_date.as_deref(), Some("1965-08-01"));
        assert_eq!(edition.language.as_ref().unwrap().language, "English");
        assert_eq!(edition.publisher.as_ref().unwrap().name, "Chilton Books");

        let book = edition.book.as_ref().unwrap();
        assert_eq!(book.id, Some(67890));
        assert_eq!(book.title.as_deref(), Some("Dune"));
        assert!(book.description.as_deref().unwrap().contains("Arrakis"));
        assert!((book.rating.unwrap() - 4.25).abs() < f64::EPSILON);
        assert_eq!(book.book_series.as_ref().unwrap().len(), 1);
        assert_eq!(book.contributions.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn build_metadata_from_edition_complete() {
        let json = r#"{
            "editions": [{
                "id": 12345,
                "isbn_13": "9780441172719",
                "isbn_10": "0441172717",
                "asin": "B00GQAIJ2C",
                "title": "Dune",
                "pages": 412,
                "release_date": "1965-08-01",
                "edition_format": "paperback",
                "reading_format": null,
                "language": { "language": "English" },
                "publisher": { "name": "Chilton Books" },
                "cached_image": "{\"url\": \"https://cdn.hardcover.app/covers/dune.jpg\"}",
                "cached_contributors": null,
                "book": {
                    "id": 67890,
                    "title": "Dune",
                    "description": "Set on the desert planet Arrakis.",
                    "rating": 4.25,
                    "book_series": [{
                        "position": 1.0,
                        "featured": true,
                        "series": { "name": "Dune" }
                    }],
                    "contributions": [
                        { "contribution": "Author", "author": { "name": "Frank Herbert" } }
                    ],
                    "cached_tags": "[{\"tag\": \"Science Fiction\"}, {\"tag\": \"Space Opera\"}]"
                }
            }]
        }"#;

        let response: EditionsResponse = serde_json::from_str(json).unwrap();
        let edition = &response.editions[0];
        let metadata = HardcoverProvider::build_metadata_from_edition(edition, "9780441172719");

        assert_eq!(metadata.provider_name, "hardcover");
        assert_eq!(metadata.title.as_deref(), Some("Dune"));
        assert_eq!(metadata.authors.len(), 1);
        assert_eq!(metadata.authors[0].name, "Frank Herbert");
        assert!(metadata.description.as_deref().unwrap().contains("Arrakis"));
        assert_eq!(metadata.language.as_deref(), Some("en"));
        assert_eq!(metadata.publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(metadata.publication_year, Some(1965));
        assert_eq!(metadata.page_count, Some(412));
        assert_eq!(
            metadata.cover_url.as_deref(),
            Some("https://cdn.hardcover.app/covers/dune.jpg")
        );
        assert_eq!(metadata.subjects, vec!["Science Fiction", "Space Opera"]);
        assert_eq!(metadata.series.as_ref().unwrap().name, "Dune");
        assert!((metadata.series.as_ref().unwrap().position.unwrap() - 1.0).abs() < f32::EPSILON);
        assert!((metadata.rating.unwrap() - 4.25).abs() < f32::EPSILON);
        assert!((metadata.confidence - 0.95).abs() < f32::EPSILON);

        // `reading_format` is null → falls back to `edition_format`.
        assert_eq!(metadata.physical_format.as_deref(), Some("paperback"));

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

        let asin = metadata
            .identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::Asin);
        assert!(asin.is_some());
        assert_eq!(asin.unwrap().value, "B00GQAIJ2C");

        let hardcover_id = metadata
            .identifiers
            .iter()
            .find(|id| id.identifier_type == IdentifierType::Hardcover);
        assert!(hardcover_id.is_some());
        assert_eq!(hardcover_id.unwrap().value, "67890");
    }

    // ── Missing optional fields ─────────────────────────────────────

    #[test]
    fn build_metadata_from_edition_minimal() {
        let json = r#"{
            "editions": [{
                "id": 99999,
                "isbn_13": null,
                "isbn_10": null,
                "asin": null,
                "title": "Unknown Book",
                "pages": null,
                "release_date": null,
                "edition_format": null,
                "reading_format": null,
                "language": null,
                "publisher": null,
                "cached_image": null,
                "cached_contributors": null,
                "book": null
            }]
        }"#;

        let response: EditionsResponse = serde_json::from_str(json).unwrap();
        let edition = &response.editions[0];
        let metadata = HardcoverProvider::build_metadata_from_edition(edition, "9781234567890");

        assert_eq!(metadata.provider_name, "hardcover");
        assert_eq!(metadata.title.as_deref(), Some("Unknown Book"));
        assert!(metadata.authors.is_empty());
        assert!(metadata.description.is_none());
        assert!(metadata.language.is_none());
        assert!(metadata.publisher.is_none());
        assert!(metadata.publication_year.is_none());
        assert!(metadata.page_count.is_none());
        assert!(metadata.cover_url.is_none());
        assert!(metadata.subjects.is_empty());
        assert!(metadata.series.is_none());
        assert!(metadata.rating.is_none());
        assert!(metadata.physical_format.is_none());

        // Should have the queried ISBN as an identifier.
        assert_eq!(metadata.identifiers.len(), 1);
        assert_eq!(
            metadata.identifiers[0].identifier_type,
            IdentifierType::Isbn13
        );
        assert_eq!(metadata.identifiers[0].value, "9781234567890");
    }

    // ── Contribution roles ────────────────────────────────────────────

    #[test]
    fn extract_authors_preserves_translator_role() {
        let json = r#"{
            "editions": [{
                "id": 11111,
                "isbn_13": "9780316273770",
                "isbn_10": null,
                "asin": null,
                "title": "The Lady of the Lake",
                "pages": 530,
                "release_date": "2017-03-14",
                "edition_format": "paperback",
                "reading_format": null,
                "language": { "language": "English" },
                "publisher": { "name": "Orbit" },
                "cached_image": null,
                "cached_contributors": null,
                "book": {
                    "id": 22222,
                    "title": "The Lady of the Lake",
                    "description": "Witcher saga.",
                    "rating": 4.1,
                    "book_series": [],
                    "contributions": [
                        { "contribution": "Author", "author": { "name": "Andrzej Sapkowski" } },
                        { "contribution": "Translator", "author": { "name": "David French" } }
                    ],
                    "cached_tags": null
                }
            }]
        }"#;

        let response: EditionsResponse = serde_json::from_str(json).unwrap();
        let edition = &response.editions[0];
        let metadata = HardcoverProvider::build_metadata_from_edition(edition, "9780316273770");

        assert_eq!(metadata.authors.len(), 2);
        assert_eq!(metadata.authors[0].name, "Andrzej Sapkowski");
        assert_eq!(metadata.authors[0].role, Some("author".to_string()));
        assert_eq!(metadata.authors[1].name, "David French");
        assert_eq!(metadata.authors[1].role, Some("translator".to_string()));
    }

    #[test]
    fn extract_authors_defaults_to_author_when_contribution_null() {
        let contribs = vec![HcContribution {
            contribution: None,
            author: Some(HcAuthor {
                name: "Unknown Role".to_string(),
            }),
        }];
        let authors = extract_authors_from_contributions(Some(&contribs));
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].role, Some("author".to_string()));
    }

    // ── Search response parsing ─────────────────────────────────────

    #[test]
    fn parse_search_response() {
        let json = r#"{
            "search": {
                "ids": [100, 200, 300],
                "results": null
            }
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        let search = response.search.unwrap();
        assert_eq!(search.ids.unwrap(), vec![100, 200, 300]);
    }

    #[test]
    fn parse_search_response_empty() {
        let json = r#"{
            "search": {
                "ids": [],
                "results": null
            }
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        let search = response.search.unwrap();
        assert!(search.ids.unwrap().is_empty());
    }

    #[test]
    fn parse_search_response_null_search() {
        let json = r#"{ "search": null }"#;
        let response: SearchResponse = serde_json::from_str(json).unwrap();
        assert!(response.search.is_none());
    }

    #[test]
    fn parse_search_response_string_ids() {
        let json = r#"{
            "search": {
                "ids": ["100", "200", "300"],
                "results": null
            }
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        let search = response.search.unwrap();
        assert_eq!(search.ids.unwrap(), vec![100, 200, 300]);
    }

    #[test]
    fn parse_search_response_mixed_ids() {
        let json = r#"{
            "search": {
                "ids": [100, "200", 300],
                "results": null
            }
        }"#;

        let response: SearchResponse = serde_json::from_str(json).unwrap();
        let search = response.search.unwrap();
        assert_eq!(search.ids.unwrap(), vec![100, 200, 300]);
    }

    // ── Books response parsing ──────────────────────────────────────

    #[test]
    fn parse_books_response() {
        let json = r#"{
            "books": [{
                "id": 67890,
                "title": "Dune",
                "description": "A science fiction classic.",
                "rating": 4.25,
                "contributions": [
                    { "contribution": "Author", "author": { "name": "Frank Herbert" } }
                ],
                "book_series": [{
                    "position": 1.0,
                    "featured": true,
                    "series": { "name": "Dune" }
                }],
                "default_physical_edition": {
                    "isbn_13": "9780441172719",
                    "isbn_10": "0441172717",
                    "pages": 412,
                    "release_date": "1965-08-01",
                    "edition_format": "hardcover",
                    "reading_format": { "format": "Read" },
                    "publisher": { "name": "Chilton Books" },
                    "language": { "language": "English" },
                    "cached_image": {"url": "https://cdn.hardcover.app/covers/dune.jpg"}
                },
                "default_ebook_edition": null,
                "cached_tags": [{"tag": "Science Fiction"}]
            }]
        }"#;

        let response: BooksResponse = serde_json::from_str(json).unwrap();
        let books = response.books.unwrap();
        assert_eq!(books.len(), 1);

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };

        let metadata = HardcoverProvider::build_metadata_from_book(&books[0], &query);

        assert_eq!(metadata.provider_name, "hardcover");
        assert_eq!(metadata.title.as_deref(), Some("Dune"));
        assert_eq!(metadata.authors.len(), 1);
        assert_eq!(metadata.authors[0].name, "Frank Herbert");
        assert_eq!(
            metadata.description.as_deref(),
            Some("A science fiction classic.")
        );
        assert_eq!(metadata.language.as_deref(), Some("en"));
        assert_eq!(metadata.publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(metadata.publication_year, Some(1965));
        assert_eq!(metadata.page_count, Some(412));
        assert_eq!(
            metadata.cover_url.as_deref(),
            Some("https://cdn.hardcover.app/covers/dune.jpg")
        );
        assert_eq!(metadata.subjects, vec!["Science Fiction"]);
        assert_eq!(metadata.series.as_ref().unwrap().name, "Dune");
        assert!((metadata.rating.unwrap() - 4.25).abs() < f32::EPSILON);
        assert_eq!(
            metadata.physical_format.as_deref(),
            Some("hardcover"),
            "`edition_format` from physical edition should populate `physical_format`"
        );
        // Exact title match -> 0.8 confidence.
        assert!((metadata.confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_books_response_string_id() {
        let json = r#"{
            "books": [{
                "id": "67890",
                "title": "Dune",
                "description": null,
                "rating": null,
                "contributions": [],
                "book_series": [],
                "default_physical_edition": null,
                "default_ebook_edition": null,
                "cached_tags": null
            }]
        }"#;

        let response: BooksResponse = serde_json::from_str(json).unwrap();
        let books = response.books.unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].id, Some(67890));
        assert_eq!(books[0].title.as_deref(), Some("Dune"));
    }

    // ── Search string construction ──────────────────────────────────

    #[test]
    fn build_search_string_title_and_author() {
        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };
        assert_eq!(build_search_string(&query), "Dune Frank Herbert");
    }

    #[test]
    fn build_search_string_title_only() {
        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            ..Default::default()
        };
        assert_eq!(build_search_string(&query), "Dune");
    }

    #[test]
    fn build_search_string_empty() {
        let query = MetadataQuery::default();
        assert_eq!(build_search_string(&query), "");
    }

    #[test]
    fn build_search_string_strips_subtitle() {
        let query = MetadataQuery {
            title: Some("Thinking, Fast and Slow: A Summary".to_string()),
            author: Some("Daniel Kahneman".to_string()),
            ..Default::default()
        };
        assert_eq!(
            build_search_string(&query),
            "Thinking, Fast and Slow Daniel Kahneman"
        );
    }

    // ── Confidence computation ──────────────────────────────────────

    #[test]
    fn confidence_exact_title_match() {
        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            ..Default::default()
        };
        let confidence = compute_search_confidence(Some("Dune"), Some("Dune"), &query);
        assert!((confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_partial_title_match() {
        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            ..Default::default()
        };
        let confidence = compute_search_confidence(Some("Dune Messiah"), Some("Dune"), &query);
        assert!((confidence - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_no_title_match() {
        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            ..Default::default()
        };
        let confidence = compute_search_confidence(Some("Foundation"), Some("Dune"), &query);
        assert!(confidence >= 0.5);
        assert!(confidence <= 0.8);
    }

    #[test]
    fn confidence_subtitle_in_query_matches_main_title_result() {
        let query = MetadataQuery {
            title: Some("Thinking, Fast and Slow: A Summary of Key Ideas".to_string()),
            ..Default::default()
        };
        let confidence = compute_search_confidence(
            Some("Thinking, Fast and Slow"),
            Some("Thinking, Fast and Slow: A Summary of Key Ideas"),
            &query,
        );
        assert!(
            (confidence - 0.7).abs() < f32::EPSILON,
            "expected 0.7 for main-title match, got {confidence}"
        );
    }

    // ── Normalization ───────────────────────────────────────────────

    #[test]
    fn normalize_strips_articles() {
        assert_eq!(normalize_for_comparison("The Hobbit"), "hobbit");
        assert_eq!(
            normalize_for_comparison("A Song of Ice and Fire"),
            "song of ice and fire"
        );
    }

    #[test]
    fn normalize_removes_punctuation() {
        assert_eq!(
            normalize_for_comparison("Dune: The Machine Crusade"),
            "dune the machine crusade"
        );
    }

    // ── Integration test (live API — ignored by default) ─────────────

    #[tokio::test]
    #[ignore = "requires live network access to Hardcover API and valid API token"]
    async fn live_isbn_lookup_dune() {
        let token = std::env::var("ARCHIVIS_HARDCOVER_TOKEN")
            .expect("ARCHIVIS_HARDCOVER_TOKEN env var must be set");

        let mut client = MetadataHttpClient::new("0.1.0", None);
        HardcoverProvider::register_rate_limiter(&mut client);
        let provider = HardcoverProvider::new(Arc::new(client), stub_settings(true, Some(&token)));

        let results = provider.lookup_isbn("9780441172719").await.unwrap();

        assert!(
            !results.is_empty(),
            "expected at least one result for Dune ISBN"
        );
        let metadata = &results[0];
        assert_eq!(metadata.provider_name, "hardcover");
        let title = metadata.title.as_deref().unwrap_or("");
        assert!(
            title.to_lowercase().contains("dune"),
            "expected title containing 'dune', got: {title}"
        );
        assert!(!metadata.authors.is_empty(), "expected at least one author");
        assert!(
            metadata.confidence >= 0.9,
            "expected high confidence, got: {}",
            metadata.confidence
        );
    }

    #[tokio::test]
    #[ignore = "requires live network access to Hardcover API and valid API token"]
    async fn live_search_dune() {
        let token = std::env::var("ARCHIVIS_HARDCOVER_TOKEN")
            .expect("ARCHIVIS_HARDCOVER_TOKEN env var must be set");

        let mut client = MetadataHttpClient::new("0.1.0", None);
        HardcoverProvider::register_rate_limiter(&mut client);
        let provider = HardcoverProvider::new(Arc::new(client), stub_settings(true, Some(&token)));

        let query = MetadataQuery {
            title: Some("Dune".to_string()),
            author: Some("Frank Herbert".to_string()),
            ..Default::default()
        };

        let results = provider.search(&query).await.unwrap();
        assert!(!results.is_empty(), "expected search results for Dune");
    }

    // ── Physical / reading format tests ─────────────────────────────

    #[test]
    fn reading_format_preferred_over_edition_format() {
        let json = r#"{
            "editions": [{
                "id": 12345,
                "isbn_13": "9781427201447",
                "isbn_10": null,
                "asin": null,
                "title": "Dune (Audiobook)",
                "pages": null,
                "release_date": null,
                "edition_format": "paperback",
                "reading_format": { "format": "Audiobook" },
                "language": null,
                "publisher": null,
                "cached_image": null,
                "cached_contributors": null,
                "book": null
            }]
        }"#;

        let response: EditionsResponse = serde_json::from_str(json).unwrap();
        let edition = &response.editions[0];
        let metadata = HardcoverProvider::build_metadata_from_edition(edition, "9781427201447");

        assert_eq!(
            metadata.physical_format.as_deref(),
            Some("Audiobook"),
            "`reading_format.format` should be preferred over `edition_format`"
        );
    }

    // ── build_metadata_from_book edition selection tests ─────────────

    fn make_hc_book_with_editions(
        ebook: Option<HcDefaultEdition>,
        physical: Option<HcDefaultEdition>,
    ) -> HcBook {
        HcBook {
            id: Some(1),
            title: Some("Test Book".to_string()),
            description: None,
            rating: None,
            contributions: None,
            book_series: None,
            default_ebook_edition: ebook,
            default_physical_edition: physical,
            image: None,
            cached_tags: None,
        }
    }

    fn make_default_edition(overrides: impl FnOnce(&mut HcDefaultEdition)) -> HcDefaultEdition {
        let mut ed = HcDefaultEdition {
            isbn_13: None,
            isbn_10: None,
            pages: None,
            release_date: None,
            edition_format: None,
            reading_format: None,
            publisher: None,
            language: None,
            image: None,
            cached_image: None,
        };
        overrides(&mut ed);
        ed
    }

    #[test]
    fn build_metadata_from_book_prefers_ebook_edition() {
        let ebook = make_default_edition(|e| {
            e.publisher = Some(HcPublisher {
                name: "Ebook Publisher".to_string(),
            });
            e.pages = Some(300);
        });
        let physical = make_default_edition(|e| {
            e.publisher = Some(HcPublisher {
                name: "Physical Publisher".to_string(),
            });
            e.pages = Some(400);
        });
        let book = make_hc_book_with_editions(Some(ebook), Some(physical));
        let query = MetadataQuery {
            title: Some("Test Book".to_string()),
            ..Default::default()
        };

        let metadata = HardcoverProvider::build_metadata_from_book(&book, &query);

        assert_eq!(metadata.publisher.as_deref(), Some("Ebook Publisher"));
        assert_eq!(metadata.page_count, Some(300));
    }

    #[test]
    fn build_metadata_from_book_falls_back_to_physical() {
        let physical = make_default_edition(|e| {
            e.publisher = Some(HcPublisher {
                name: "Physical Publisher".to_string(),
            });
            e.pages = Some(400);
        });
        let book = make_hc_book_with_editions(None, Some(physical));
        let query = MetadataQuery {
            title: Some("Test Book".to_string()),
            ..Default::default()
        };

        let metadata = HardcoverProvider::build_metadata_from_book(&book, &query);

        assert_eq!(metadata.publisher.as_deref(), Some("Physical Publisher"));
        assert_eq!(metadata.page_count, Some(400));
    }

    #[test]
    fn build_metadata_from_book_collects_isbns_from_both() {
        let ebook = make_default_edition(|e| {
            e.isbn_13 = Some("9780441172719".to_string());
        });
        let physical = make_default_edition(|e| {
            e.isbn_10 = Some("0441172717".to_string());
        });
        let book = make_hc_book_with_editions(Some(ebook), Some(physical));
        let query = MetadataQuery {
            title: Some("Test Book".to_string()),
            ..Default::default()
        };

        let metadata = HardcoverProvider::build_metadata_from_book(&book, &query);

        assert!(
            metadata
                .identifiers
                .iter()
                .any(|i| i.identifier_type == IdentifierType::Isbn13 && i.value == "9780441172719"),
            "ISBN-13 from ebook edition should be collected"
        );
        assert!(
            metadata
                .identifiers
                .iter()
                .any(|i| i.identifier_type == IdentifierType::Isbn10 && i.value == "0441172717"),
            "ISBN-10 from physical edition should be collected"
        );
    }

    #[test]
    fn build_metadata_from_book_cover_falls_back() {
        let ebook = make_default_edition(|_| {
            // no cached_image
        });
        let physical = make_default_edition(|e| {
            e.cached_image = Some(serde_json::json!(
                "https://cdn.hardcover.app/covers/phys.jpg"
            ));
        });
        let book = make_hc_book_with_editions(Some(ebook), Some(physical));
        let query = MetadataQuery {
            title: Some("Test Book".to_string()),
            ..Default::default()
        };

        let metadata = HardcoverProvider::build_metadata_from_book(&book, &query);

        assert_eq!(
            metadata.cover_url.as_deref(),
            Some("https://cdn.hardcover.app/covers/phys.jpg"),
            "cover should fall back to physical edition"
        );
    }

    #[test]
    fn build_metadata_from_book_populates_physical_format() {
        let ebook = make_default_edition(|e| {
            e.edition_format = Some("ebook".to_string());
        });
        let book = make_hc_book_with_editions(Some(ebook), None);
        let query = MetadataQuery {
            title: Some("Test Book".to_string()),
            ..Default::default()
        };

        let metadata = HardcoverProvider::build_metadata_from_book(&book, &query);

        assert_eq!(
            metadata.physical_format.as_deref(),
            Some("ebook"),
            "`edition_format` should populate `physical_format`"
        );
    }
}
