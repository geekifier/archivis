use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use tracing::{debug, warn};

static TRAILING_DATES_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r",?\s*\d{4}\s*-\s*\d{0,4}\s*$|,?\s*-\s*\d{4}\s*$|,?\s+\d{4}\s*$")
        .expect("valid regex")
});

static PARENTHETICALS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\([^)]*\)\s*").expect("valid regex"));

static EXTENT_PAGES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)\s*(?:p\b|pages?\b)").expect("valid regex"));

use archivis_core::models::IdentifierType;
use archivis_core::settings::SettingsReader;

use crate::client::MetadataHttpClient;
use crate::errors::ProviderError;
use crate::provider::MetadataProvider;
use crate::provider_names;
use crate::similarity::title_for_search;
use crate::types::{
    parse_year_from_str, titlecase_title, MetadataQuery, ProviderAuthor, ProviderCapabilities,
    ProviderIdentifier, ProviderMetadata, ProviderQuality, ProviderSeries,
};

static CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    quality: ProviderQuality::Authoritative,
    default_rate_limit_rpm: MAX_REQUESTS_PER_MINUTE,
    supported_id_lookups: &[
        IdentifierType::Isbn13,
        IdentifierType::Isbn10,
        IdentifierType::Lccn,
    ],
    features: &[],
};

const PROVIDER_NAME: &str = provider_names::LOC;
/// LOC SRU endpoint. The HTTPS variant (`https://lx2.loc.gov/sru/lcdb`)
/// is sometimes unreliable (502s), so we use the HTTP endpoint which is
/// more stable. The data returned is public bibliographic catalog data.
const SRU_BASE_URL: &str = "http://lx2.loc.gov:210/lcdb";
const MAX_REQUESTS_PER_MINUTE: u32 = 20;

/// CAPTCHA detection: LOC returns an HTML page when it bans your IP.
const CAPTCHA_RETRY_SECS: u64 = 60;

/// Library of Congress metadata provider.
///
/// Uses the LOC SRU (Search/Retrieve via URL) endpoint, which returns
/// MODS XML. No API key is required, but the rate limit is strict
/// (20 req/min with 1-hour IP ban on violation).
///
/// Reads `metadata.enabled` and `metadata.loc.enabled` from settings
/// at call time so runtime changes take effect immediately.
pub struct LocProvider {
    client: Arc<MetadataHttpClient>,
    settings: Arc<dyn SettingsReader>,
}

impl LocProvider {
    pub fn new(client: Arc<MetadataHttpClient>, settings: Arc<dyn SettingsReader>) -> Self {
        Self { client, settings }
    }

    fn is_enabled(&self) -> bool {
        let global = self
            .settings
            .get_setting("metadata.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let provider = self
            .settings
            .get_setting("metadata.loc.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        global && provider
    }

    /// Register this provider's rate limiter with the shared HTTP client.
    pub fn register_rate_limiter(client: &mut MetadataHttpClient) {
        client.register_provider(PROVIDER_NAME, MAX_REQUESTS_PER_MINUTE);
    }

    /// Register a custom rate limit with the shared HTTP client.
    pub fn register_rate_limiter_with_limit(client: &mut MetadataHttpClient, max_rpm: u32) {
        client.register_provider(PROVIDER_NAME, max_rpm);
    }

    /// Build an SRU `searchRetrieve` URL with a CQL query.
    ///
    /// Uses Bath profile indexes: `bath.isbn`, `bath.title`, `bath.author`.
    fn build_sru_url(cql_query: &str, max_records: u32) -> String {
        let mut url = url::Url::parse(SRU_BASE_URL).expect("valid SRU base URL");
        url.query_pairs_mut()
            .append_pair("version", "1.1")
            .append_pair("operation", "searchRetrieve")
            .append_pair("query", cql_query)
            .append_pair("maximumRecords", &max_records.to_string())
            .append_pair("recordSchema", "mods");
        url.to_string()
    }

    /// Perform an SRU request and return the response body as XML text.
    async fn sru_request(&self, url: &str) -> Result<String, ProviderError> {
        let response = self.client.get(PROVIDER_NAME, url).await?;

        // Check for CAPTCHA: LOC returns HTML when it bans the IP
        if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
            if let Ok(ct) = content_type.to_str() {
                if ct.contains("text/html") {
                    warn!(
                        provider = PROVIDER_NAME,
                        "received HTML response (likely CAPTCHA/rate-limit block)"
                    );
                    return Err(ProviderError::RateLimited {
                        retry_after: Some(Duration::from_secs(CAPTCHA_RETRY_SECS)),
                    });
                }
            }
        }

        let status = response.status().as_u16();
        if status != 200 {
            return Err(ProviderError::ApiError {
                status,
                message: format!("SRU returned HTTP {status}"),
            });
        }

        let body = response.text().await.map_err(ProviderError::from)?;
        Ok(body)
    }

    /// Parse MODS XML from an SRU response and return metadata candidates.
    fn parse_sru_response(xml: &str) -> Vec<ModsRecord> {
        let mut reader = Reader::from_str(xml);
        let mut records = Vec::new();
        let mut in_record_data = false;
        let mut record_xml = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(ref e)) if e.local_name().as_ref() == b"recordData" => {
                    in_record_data = true;
                    record_xml.clear();
                }
                Ok(Event::End(ref e)) if e.local_name().as_ref() == b"recordData" => {
                    in_record_data = false;
                    if let Some(record) = parse_mods_record(&record_xml) {
                        records.push(record);
                    }
                }
                Ok(Event::Text(ref e)) if in_record_data => {
                    // recordData may contain escaped XML
                    if let Ok(text) = e.unescape() {
                        record_xml.push_str(&text);
                    }
                }
                Ok(Event::CData(ref e)) if in_record_data => {
                    if let Ok(text) = std::str::from_utf8(e.as_ref()) {
                        record_xml.push_str(text);
                    }
                }
                // If the MODS content is nested as real XML, capture it differently
                Ok(ref event) if in_record_data => {
                    // Write raw event to string
                    let mut writer = quick_xml::Writer::new(Vec::new());
                    if writer.write_event(event.borrow()).is_ok() {
                        if let Ok(s) = String::from_utf8(writer.into_inner()) {
                            record_xml.push_str(&s);
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    warn!(provider = PROVIDER_NAME, error = %e, "XML parse error in SRU envelope");
                    break;
                }
                _ => {}
            }
        }

        records
    }

    /// Convert a `ModsRecord` into a `ProviderMetadata`.
    fn record_to_metadata(record: ModsRecord, confidence: f32) -> ProviderMetadata {
        // Combine `nonSort` prefix with title-cased title
        let title = match (record.non_sort, record.title) {
            (Some(prefix), Some(t)) => Some(format!("{prefix}{}", titlecase_title(&t))),
            (None, Some(t)) => Some(titlecase_title(&t)),
            (Some(prefix), None) => Some(prefix),
            (None, None) => None,
        };

        // Deduplicate authors by lowercased cleaned name (first occurrence wins)
        let mut seen_names = std::collections::HashSet::new();
        let authors: Vec<ProviderAuthor> = record
            .names
            .into_iter()
            .filter_map(|n| {
                let cleaned = clean_author_name(&n.name);
                let key = cleaned.to_lowercase();
                if seen_names.insert(key) {
                    Some(ProviderAuthor {
                        name: cleaned,
                        role: n.role.as_deref().map(normalize_marc_role),
                    })
                } else {
                    None
                }
            })
            .collect();

        let mut identifiers = Vec::new();
        for isbn in &record.isbns {
            if let Some(id) = ProviderIdentifier::isbn(isbn) {
                identifiers.push(id);
            } else {
                warn!(
                    provider = PROVIDER_NAME,
                    isbn = isbn,
                    "skipping invalid ISBN from LOC record"
                );
            }
        }
        if let Some(ref lccn) = record.lccn {
            identifiers.push(ProviderIdentifier {
                identifier_type: IdentifierType::Lccn,
                value: lccn.clone(),
            });
        }

        let page_count = record.extent.as_deref().and_then(parse_extent_pages);

        let publication_year = record
            .date_issued
            .as_deref()
            .map(parse_loc_date)
            .as_deref()
            .and_then(parse_year_from_str);

        let language = record
            .language_code
            .as_deref()
            .and_then(archivis_core::language::normalize_language)
            .map(String::from);

        // Combine LCC and DDC classifications into subjects along with topics
        let mut subjects = record.topics;
        for class in &record.lcc_classifications {
            subjects.push(format!("LCC: {class}"));
        }
        for class in &record.ddc_classifications {
            subjects.push(format!("DDC: {class}"));
        }

        ProviderMetadata {
            provider_name: PROVIDER_NAME.to_string(),
            title,
            subtitle: record.subtitle.map(|s| titlecase_title(&s)),
            authors,
            description: None,
            language,
            publisher: record.publisher,
            publication_year,
            identifiers,
            subjects,
            series: record.series_title.map(|name| ProviderSeries {
                name: titlecase_title(&name),
                position: record
                    .series_position
                    .as_deref()
                    .and_then(parse_series_position),
            }),
            page_count,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence,
            merged_from: Vec::new(),
            field_sources: BTreeMap::new(),
        }
    }
}

#[async_trait]
impl MetadataProvider for LocProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn is_available(&self) -> bool {
        self.is_enabled()
    }

    fn capabilities(&self) -> &'static ProviderCapabilities {
        &CAPABILITIES
    }

    async fn lookup_isbn(&self, isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.is_enabled() {
            return Err(ProviderError::NotConfigured(
                "LOC provider is disabled".to_string(),
            ));
        }

        let url = Self::build_sru_url(&format!("bath.isbn={isbn}"), 3);
        debug!(provider = PROVIDER_NAME, isbn = isbn, "ISBN lookup");

        let xml = self.sru_request(&url).await?;
        let records = Self::parse_sru_response(&xml);

        let results: Vec<ProviderMetadata> = records
            .into_iter()
            .map(|r| Self::record_to_metadata(r, 0.95))
            .collect();

        debug!(
            provider = PROVIDER_NAME,
            isbn = isbn,
            results = results.len(),
            "ISBN lookup complete"
        );
        Ok(results)
    }

    async fn search(&self, query: &MetadataQuery) -> Result<Vec<ProviderMetadata>, ProviderError> {
        if !self.is_enabled() {
            return Err(ProviderError::NotConfigured(
                "LOC provider is disabled".to_string(),
            ));
        }

        let title = query.title.as_deref().unwrap_or_default();
        let author = query.author.as_deref().unwrap_or_default();

        if title.is_empty() && author.is_empty() {
            return Ok(Vec::new());
        }

        // Build CQL query using Bath profile indexes
        let search_title = title_for_search(title);
        let cql_query = if !search_title.is_empty() && !author.is_empty() {
            format!("bath.title=\"{search_title}\" and bath.author=\"{author}\"")
        } else if !search_title.is_empty() {
            format!("bath.title=\"{search_title}\"")
        } else {
            format!("bath.author=\"{author}\"")
        };

        let url = Self::build_sru_url(&cql_query, 5);
        debug!(
            provider = PROVIDER_NAME,
            title = title,
            author = author,
            "search"
        );

        let xml = self.sru_request(&url).await?;
        let records = Self::parse_sru_response(&xml);

        let query_title_lower = search_title.to_lowercase();
        let results: Vec<ProviderMetadata> = records
            .into_iter()
            .map(|r| {
                // Score confidence based on title similarity
                let confidence = r.title.as_deref().map_or(0.5, |t| {
                    let record_title = title_for_search(t).to_lowercase();
                    if record_title == query_title_lower {
                        0.9
                    } else if record_title.contains(&query_title_lower)
                        || query_title_lower.contains(&record_title)
                    {
                        0.75
                    } else {
                        0.5
                    }
                });
                Self::record_to_metadata(r, confidence)
            })
            .collect();

        debug!(
            provider = PROVIDER_NAME,
            results = results.len(),
            "search complete"
        );
        Ok(results)
    }

    async fn fetch_cover(&self, _cover_url: &str) -> Result<Vec<u8>, ProviderError> {
        Err(ProviderError::NotConfigured(
            "LOC does not provide cover images".to_string(),
        ))
    }
}

// ── MODS XML parsing ────────────────────────────────────────────────

/// Intermediate representation of a MODS record extracted from XML.
#[derive(Debug, Default)]
struct ModsRecord {
    /// Title prefix from `<nonSort>` (e.g. "The ").
    non_sort: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    names: Vec<ModsName>,
    publisher: Option<String>,
    date_issued: Option<String>,
    extent: Option<String>,
    language_code: Option<String>,
    topics: Vec<String>,
    isbns: Vec<String>,
    lccn: Option<String>,
    lcc_classifications: Vec<String>,
    ddc_classifications: Vec<String>,
    series_title: Option<String>,
    series_position: Option<String>,
}

#[derive(Debug, Default)]
struct ModsName {
    name: String,
    role: Option<String>,
    is_primary: bool,
}

/// Parse a single MODS `<mods>` element into a `ModsRecord`.
#[allow(clippy::too_many_lines)]
fn parse_mods_record(xml: &str) -> Option<ModsRecord> {
    let mut reader = Reader::from_str(xml);
    let mut record = ModsRecord::default();

    // State tracking
    let mut current_text = String::new();

    // `name` element state
    let mut current_name = ModsName::default();
    let mut in_name = false;
    let mut name_usage_primary = false;

    // `identifier` type attribute
    let mut identifier_type = String::new();

    // `classification` authority attribute
    let mut classification_authority = String::new();

    // Track ancestor context via boolean flags
    let mut in_primary_title_info = false;
    let mut in_origin_info = false;
    let mut in_physical_description = false;
    let mut in_related_item_series = false;
    let mut in_series_title_info = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let local = e.local_name();

                match local.as_ref() {
                    b"relatedItem" => {
                        let is_series = e
                            .attributes()
                            .flatten()
                            .any(|a| a.key.as_ref() == b"type" && a.value.as_ref() == b"series");
                        in_related_item_series = is_series;
                    }
                    b"titleInfo" if in_related_item_series => {
                        in_series_title_info = true;
                    }
                    b"titleInfo" => {
                        // Only treat as primary title if no @type attribute
                        let has_type = e.attributes().flatten().any(|a| a.key.as_ref() == b"type");
                        in_primary_title_info = !has_type;
                    }
                    b"name" => {
                        in_name = true;
                        current_name = ModsName::default();
                        name_usage_primary = e
                            .attributes()
                            .flatten()
                            .any(|a| a.key.as_ref() == b"usage" && a.value.as_ref() == b"primary");
                    }
                    b"identifier" => {
                        identifier_type.clear();
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"type" {
                                identifier_type = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                    }
                    b"classification" => {
                        classification_authority.clear();
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"authority" {
                                classification_authority =
                                    String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                    }
                    b"originInfo" => in_origin_info = true,
                    b"physicalDescription" => in_physical_description = true,
                    _ => {}
                }

                current_text.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    current_text.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                let text = current_text.trim().to_string();

                match local.as_ref() {
                    b"nonSort" if in_primary_title_info => {
                        // Preserve trailing whitespace (e.g. "The ")
                        let raw = current_text.trim_start().to_string();
                        if !raw.is_empty() && record.non_sort.is_none() {
                            record.non_sort = Some(raw);
                        }
                    }
                    b"title" if in_series_title_info && !text.is_empty() => {
                        if record.series_title.is_none() {
                            record.series_title = Some(text);
                        }
                    }
                    b"partNumber" if in_series_title_info && !text.is_empty() => {
                        if record.series_position.is_none() {
                            record.series_position = Some(text);
                        }
                    }
                    b"title" if in_primary_title_info && !text.is_empty() => {
                        if record.title.is_none() {
                            record.title = Some(text);
                        }
                    }
                    b"subTitle" if in_primary_title_info && !text.is_empty() => {
                        if record.subtitle.is_none() {
                            record.subtitle = Some(text);
                        }
                    }
                    b"titleInfo" => {
                        in_series_title_info = false;
                        in_primary_title_info = false;
                    }
                    b"relatedItem" => {
                        in_related_item_series = false;
                    }
                    b"namePart" if in_name && !text.is_empty() => {
                        if current_name.name.is_empty() {
                            current_name.name = text;
                        } else {
                            // Additional namePart — may be dates, etc.
                            // Skip date-like parts
                            if !text.chars().any(|c| c.is_ascii_digit()) {
                                current_name.name = format!("{} {}", current_name.name, text);
                            }
                        }
                    }
                    b"roleTerm" if in_name && !text.is_empty() => {
                        current_name.role = Some(text.to_lowercase());
                    }
                    b"name" => {
                        in_name = false;
                        current_name.is_primary = name_usage_primary;
                        if !current_name.name.is_empty() {
                            record.names.push(std::mem::take(&mut current_name));
                        }
                    }
                    b"publisher" | b"namePart"
                        if !in_name && !text.is_empty() && in_origin_info =>
                    {
                        if record.publisher.is_none() {
                            // Strip trailing comma/semicolon from publisher names
                            let cleaned = text.trim_end_matches([',', ';']).trim().to_string();
                            if !cleaned.is_empty() {
                                record.publisher = Some(cleaned);
                            }
                        }
                    }
                    b"dateIssued" if !text.is_empty() => {
                        if record.date_issued.is_none() {
                            record.date_issued = Some(text);
                        }
                    }
                    b"extent" if in_physical_description && !text.is_empty() => {
                        if record.extent.is_none() {
                            record.extent = Some(text);
                        }
                    }
                    b"languageTerm" if !text.is_empty() => {
                        if record.language_code.is_none() {
                            record.language_code = Some(text);
                        }
                    }
                    b"topic" | b"geographic" if !text.is_empty() => {
                        if !record.topics.contains(&text) {
                            record.topics.push(text);
                        }
                    }
                    b"identifier" if !text.is_empty() => match identifier_type.as_str() {
                        "isbn" => {
                            record.isbns.push(text);
                        }
                        "lccn" => {
                            if record.lccn.is_none() {
                                record.lccn = Some(text);
                            }
                        }
                        _ => {}
                    },
                    b"classification" if !text.is_empty() => {
                        match classification_authority.as_str() {
                            "lcc" => record.lcc_classifications.push(text),
                            "ddc" => record.ddc_classifications.push(text),
                            _ => {}
                        }
                    }
                    b"originInfo" => in_origin_info = false,
                    b"physicalDescription" => in_physical_description = false,
                    _ => {}
                }

                current_text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                warn!(provider = PROVIDER_NAME, error = %e, "XML parse error in MODS record");
                break;
            }
            _ => {}
        }
    }

    // Only return records that have at least a title
    if record.title.is_some() {
        Some(record)
    } else {
        None
    }
}

// ── Helper functions ────────────────────────────────────────────────

/// Clean LOC-style author names.
///
/// LOC names are typically in "Last, First" format with dates and qualifiers:
/// `"Tolkien, J. R. R. (John Ronald Reuel), 1892-1973"` → `"J. R. R. Tolkien"`
pub(crate) fn clean_author_name(name: &str) -> String {
    let name = name.trim();

    // Remove trailing dates (e.g. ", 1892-1973" or ", 1965-")
    let name = strip_trailing_dates(name);

    // Remove parenthetical qualifiers (e.g. "(John Ronald Reuel)")
    let name = strip_parentheticals(&name);

    // Strip MARC field-terminating period
    let name = strip_marc_trailing_period(&name);

    // Strip trailing comma/semicolon (LOC names often end with comma)
    let name = name.trim_end_matches([',', ';']).trim();

    // Handle "Last, First" inversion
    if let Some((last, first)) = name.split_once(", ") {
        let first = first.trim().trim_end_matches([',', ';']).trim();
        let last = last.trim();
        if !first.is_empty() && !last.is_empty() {
            return format!("{first} {last}");
        }
    }

    name.to_string()
}

/// Strip trailing date patterns like ", 1892-1973", ", 1965-", ", -1973"
fn strip_trailing_dates(name: &str) -> String {
    TRAILING_DATES_RE.replace(name, "").trim().to_string()
}

/// Remove parenthetical expressions like "(John Ronald Reuel)"
fn strip_parentheticals(name: &str) -> String {
    PARENTHETICALS_RE.replace_all(name, " ").trim().to_string()
}

/// Strip MARC field-terminating period, preserving periods on initials.
///
/// MARC 100/700 fields always end with punctuation. When the name ends
/// with a multi-character word + period, that period is purely MARC
/// punctuation. When it ends with a single letter + period (an initial),
/// the period is part of the name.
fn strip_marc_trailing_period(name: &str) -> String {
    if !name.ends_with('.') {
        return name.to_string();
    }
    let before = &name[..name.len() - 1];
    let last_word = before.split_whitespace().next_back().unwrap_or("");
    // Single letter before period = initial → keep the period
    if last_word.len() == 1 && last_word.chars().all(char::is_alphabetic) {
        name.to_string()
    } else {
        before.trim_end().to_string()
    }
}

/// Parse a MODS `partNumber` string into a numeric position.
///
/// Handles: `"1"`, `"2"`, `"bk. 2"`, `"vol. 3"`, `"1.5"`
fn parse_series_position(raw: &str) -> Option<f32> {
    raw.split_whitespace()
        .filter_map(|tok| tok.trim_end_matches('.').parse::<f32>().ok())
        .next_back()
}

/// Parse page count from LOC extent strings.
///
/// Examples:
/// - `"xiii, 256 p."` → `256`
/// - `"viii, 423 pages ; 24 cm."` → `423`
/// - `"1 volume (various pagings)"` → `None`
pub(crate) fn parse_extent_pages(extent: &str) -> Option<i32> {
    EXTENT_PAGES_RE
        .captures_iter(extent)
        .last()
        .and_then(|cap| cap[1].parse::<i32>().ok())
}

/// Parse LOC-style dates, stripping "c" prefix and brackets.
///
/// Examples:
/// - `"c2005"` → `"2005"`
/// - `"[1965]"` → `"1965"`
/// - `"©2010"` → `"2010"`
pub(crate) fn parse_loc_date(date: &str) -> String {
    date.trim()
        .trim_start_matches('c')
        .trim_start_matches('©')
        .trim_matches('[')
        .trim_matches(']')
        .trim()
        .to_string()
}

/// Normalize MARC relator codes and abbreviations to human-readable roles.
///
/// Strips trailing `.` before matching. Unknown roles pass through lowercased.
fn normalize_marc_role(role: &str) -> String {
    let role = role.trim().trim_end_matches('.');
    match role.to_lowercase().as_str() {
        "aut" | "author" => "author".to_string(),
        "edt" | "ed" | "editor" => "editor".to_string(),
        "trl" | "tr" | "translator" => "translator".to_string(),
        "ill" | "illustrator" => "illustrator".to_string(),
        "com" | "compiler" => "compiler".to_string(),
        "ctb" | "contributor" => "contributor".to_string(),
        "nrt" | "narrator" => "narrator".to_string(),
        "adp" | "adapter" => "adapter".to_string(),
        "ann" | "annotator" => "annotator".to_string(),
        "aui" | "author of introduction" => "author of introduction".to_string(),
        "cre" | "creator" => "creator".to_string(),
        "pht" | "photographer" => "photographer".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Author name cleaning ────────────────────────────────────────

    #[test]
    fn clean_simple_inverted_name() {
        assert_eq!(clean_author_name("Tolkien, J. R. R."), "J. R. R. Tolkien");
    }

    #[test]
    fn clean_name_with_dates() {
        assert_eq!(
            clean_author_name("Tolkien, J. R. R., 1892-1973"),
            "J. R. R. Tolkien"
        );
    }

    #[test]
    fn clean_name_with_parenthetical_and_dates() {
        assert_eq!(
            clean_author_name("Tolkien, J. R. R. (John Ronald Reuel), 1892-1973"),
            "J. R. R. Tolkien"
        );
    }

    #[test]
    fn clean_name_with_open_date_range() {
        assert_eq!(clean_author_name("Obama, Barack, 1961-"), "Barack Obama");
    }

    #[test]
    fn clean_single_name() {
        assert_eq!(clean_author_name("Voltaire"), "Voltaire");
    }

    #[test]
    fn clean_name_no_dates() {
        assert_eq!(clean_author_name("King, Stephen"), "Stephen King");
    }

    // ── Page count extraction ───────────────────────────────────────

    #[test]
    fn parse_pages_standard() {
        assert_eq!(parse_extent_pages("xiii, 256 p."), Some(256));
    }

    #[test]
    fn parse_pages_with_illustrations() {
        assert_eq!(
            parse_extent_pages("viii, 423 pages : ill. ; 24 cm."),
            Some(423)
        );
    }

    #[test]
    fn parse_pages_simple() {
        assert_eq!(parse_extent_pages("320 p."), Some(320));
    }

    #[test]
    fn parse_pages_various_pagings() {
        assert_eq!(parse_extent_pages("1 volume (various pagings)"), None);
    }

    #[test]
    fn parse_pages_none() {
        assert_eq!(parse_extent_pages("3 sound discs"), None);
    }

    // ── Date parsing ────────────────────────────────────────────────

    #[test]
    fn parse_date_c_prefix() {
        assert_eq!(parse_loc_date("c2005"), "2005");
    }

    #[test]
    fn parse_date_brackets() {
        assert_eq!(parse_loc_date("[1965]"), "1965");
    }

    #[test]
    fn parse_date_copyright() {
        assert_eq!(parse_loc_date("©2010"), "2010");
    }

    #[test]
    fn parse_date_plain() {
        assert_eq!(parse_loc_date("2023"), "2023");
    }

    // ── Language code conversion (via shared normalize_language) ───

    #[test]
    fn iso639_english() {
        assert_eq!(
            archivis_core::language::normalize_language("eng"),
            Some("en")
        );
    }

    #[test]
    fn iso639_french_bibliographic() {
        assert_eq!(
            archivis_core::language::normalize_language("fre"),
            Some("fr")
        );
    }

    #[test]
    fn iso639_french_terminological() {
        assert_eq!(
            archivis_core::language::normalize_language("fra"),
            Some("fr")
        );
    }

    #[test]
    fn iso639_case_insensitive() {
        assert_eq!(
            archivis_core::language::normalize_language("ENG"),
            Some("en")
        );
    }

    #[test]
    fn iso639_unknown() {
        assert_eq!(archivis_core::language::normalize_language("xxx"), None);
    }

    // ── MODS XML parsing ────────────────────────────────────────────

    #[test]
    fn parse_mods_basic_record() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo>
                <title>The Lord of the Rings</title>
                <subTitle>The Fellowship of the Ring</subTitle>
            </titleInfo>
            <name type="personal" usage="primary">
                <namePart>Tolkien, J. R. R.</namePart>
                <namePart type="date">1892-1973</namePart>
                <role>
                    <roleTerm type="text">author</roleTerm>
                </role>
            </name>
            <originInfo>
                <publisher>Houghton Mifflin</publisher>
                <dateIssued>1954</dateIssued>
            </originInfo>
            <physicalDescription>
                <extent>xiii, 423 p. : ill. ; 22 cm.</extent>
            </physicalDescription>
            <language>
                <languageTerm type="code" authority="iso639-2b">eng</languageTerm>
            </language>
            <subject authority="lcsh">
                <topic>Fantasy fiction</topic>
            </subject>
            <subject authority="lcsh">
                <geographic>Middle Earth</geographic>
            </subject>
            <identifier type="isbn">0618346252</identifier>
            <identifier type="lccn">2003048928</identifier>
            <classification authority="lcc">PR6039.O32</classification>
            <classification authority="ddc">823/.912</classification>
        </mods>
        "#;

        let record = parse_mods_record(xml).expect("should parse MODS record");
        assert_eq!(record.title.as_deref(), Some("The Lord of the Rings"));
        assert_eq!(
            record.subtitle.as_deref(),
            Some("The Fellowship of the Ring")
        );
        assert_eq!(record.names.len(), 1);
        assert_eq!(record.names[0].name, "Tolkien, J. R. R.");
        assert_eq!(record.names[0].role.as_deref(), Some("author"));
        assert!(record.names[0].is_primary);
        assert_eq!(record.publisher.as_deref(), Some("Houghton Mifflin"));
        assert_eq!(record.date_issued.as_deref(), Some("1954"));
        assert_eq!(
            record.extent.as_deref(),
            Some("xiii, 423 p. : ill. ; 22 cm.")
        );
        assert_eq!(record.language_code.as_deref(), Some("eng"));
        assert!(record.topics.contains(&"Fantasy fiction".to_string()));
        assert!(record.topics.contains(&"Middle Earth".to_string()));
        assert_eq!(record.isbns, vec!["0618346252"]);
        assert_eq!(record.lccn.as_deref(), Some("2003048928"));
        assert_eq!(record.lcc_classifications, vec!["PR6039.O32"]);
        assert_eq!(record.ddc_classifications, vec!["823/.912"]);
    }

    #[test]
    fn parse_mods_no_title_returns_none() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <name type="personal">
                <namePart>Unknown</namePart>
            </name>
        </mods>
        "#;
        assert!(parse_mods_record(xml).is_none());
    }

    #[test]
    fn parse_mods_alternate_title_not_primary() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo>
                <title>Primary Title</title>
            </titleInfo>
            <titleInfo type="alternative">
                <title>Alt Title</title>
            </titleInfo>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.title.as_deref(), Some("Primary Title"));
    }

    #[test]
    fn parse_sru_response_envelope() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <searchRetrieveResponse>
            <numberOfRecords>1</numberOfRecords>
            <records>
                <record>
                    <recordData><mods xmlns="http://www.loc.gov/mods/v3">
                        <titleInfo><title>Test Book</title></titleInfo>
                        <identifier type="isbn">9780123456789</identifier>
                    </mods></recordData>
                </record>
            </records>
        </searchRetrieveResponse>
        "#;

        let records = LocProvider::parse_sru_response(xml);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title.as_deref(), Some("Test Book"));
        assert_eq!(records[0].isbns, vec!["9780123456789"]);
    }

    #[test]
    fn parse_sru_response_multiple_records() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <searchRetrieveResponse>
            <records>
                <record>
                    <recordData><mods xmlns="http://www.loc.gov/mods/v3">
                        <titleInfo><title>Book One</title></titleInfo>
                    </mods></recordData>
                </record>
                <record>
                    <recordData><mods xmlns="http://www.loc.gov/mods/v3">
                        <titleInfo><title>Book Two</title></titleInfo>
                    </mods></recordData>
                </record>
            </records>
        </searchRetrieveResponse>
        "#;

        let records = LocProvider::parse_sru_response(xml);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].title.as_deref(), Some("Book One"));
        assert_eq!(records[1].title.as_deref(), Some("Book Two"));
    }

    #[test]
    fn record_to_metadata_converts_correctly() {
        let record = ModsRecord {
            non_sort: None,
            title: Some("Dune".to_string()),
            subtitle: None,
            names: vec![ModsName {
                name: "Herbert, Frank, 1920-1986".to_string(),
                role: Some("author".to_string()),
                is_primary: true,
            }],
            publisher: Some("Chilton Books".to_string()),
            date_issued: Some("c1965".to_string()),
            extent: Some("viii, 412 p.".to_string()),
            language_code: Some("eng".to_string()),
            topics: vec!["Science fiction".to_string()],
            isbns: vec!["9780441172719".to_string()],
            lccn: Some("65022576".to_string()),
            lcc_classifications: vec!["PZ4.H536".to_string()],
            ddc_classifications: vec!["813/.54".to_string()],
            series_title: None,
            series_position: None,
        };

        let metadata = LocProvider::record_to_metadata(record, 0.95);
        assert_eq!(metadata.provider_name, "loc");
        assert_eq!(metadata.title.as_deref(), Some("Dune"));
        assert_eq!(metadata.authors.len(), 1);
        assert_eq!(metadata.authors[0].name, "Frank Herbert");
        assert_eq!(metadata.publisher.as_deref(), Some("Chilton Books"));
        assert_eq!(metadata.publication_year, Some(1965));
        assert_eq!(metadata.page_count, Some(412));
        assert_eq!(metadata.language.as_deref(), Some("en"));
        assert!(metadata.subjects.contains(&"Science fiction".to_string()));
        assert!(metadata.subjects.contains(&"LCC: PZ4.H536".to_string()));
        assert!(metadata.subjects.contains(&"DDC: 813/.54".to_string()));
        assert_eq!(metadata.identifiers.len(), 2);
        assert!(metadata
            .identifiers
            .iter()
            .any(|id| id.identifier_type == IdentifierType::Isbn13 && id.value == "9780441172719"));
        assert!(metadata
            .identifiers
            .iter()
            .any(|id| id.identifier_type == IdentifierType::Lccn && id.value == "65022576"));
        assert!(metadata.cover_url.is_none());
        assert!((metadata.confidence - 0.95).abs() < f32::EPSILON);
        crate::types::assert_isbns_valid(&metadata);
    }

    #[test]
    fn parse_mods_publisher_trailing_punctuation() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo><title>Test</title></titleInfo>
            <originInfo>
                <publisher>Random House,</publisher>
            </originInfo>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.publisher.as_deref(), Some("Random House"));
    }

    #[test]
    fn clean_name_trailing_comma() {
        assert_eq!(clean_author_name("Herbert, Frank,"), "Frank Herbert");
    }

    // ── nonSort handling ────────────────────────────────────────────

    #[test]
    fn parse_mods_non_sort_prefix() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo>
                <nonSort xml:space="preserve">The </nonSort>
                <title>lord of the rings</title>
            </titleInfo>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.non_sort.as_deref(), Some("The "));
        assert_eq!(record.title.as_deref(), Some("lord of the rings"));
    }

    #[test]
    fn record_to_metadata_combines_non_sort_with_title() {
        let record = ModsRecord {
            non_sort: Some("The ".to_string()),
            title: Some("lord of the rings".to_string()),
            ..ModsRecord::default()
        };
        let metadata = LocProvider::record_to_metadata(record, 0.9);
        assert_eq!(metadata.title.as_deref(), Some("The Lord of the Rings"));
    }

    // ── Real LOC XML format ─────────────────────────────────────────

    #[test]
    fn parse_mods_agent_publisher() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo><title>Dune</title></titleInfo>
            <originInfo>
                <place><placeTerm type="text">New York</placeTerm></place>
                <agent><namePart>Ace Books</namePart></agent>
                <dateIssued>[2019]</dateIssued>
            </originInfo>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.publisher.as_deref(), Some("Ace Books"));
        assert_eq!(record.date_issued.as_deref(), Some("[2019]"));
    }

    #[test]
    fn parse_sru_response_with_namespaced_envelope() {
        let xml = r#"<?xml version="1.0"?>
        <zs:searchRetrieveResponse xmlns:zs="http://www.loc.gov/zing/srw/">
            <zs:version>1.1</zs:version>
            <zs:numberOfRecords>1</zs:numberOfRecords>
            <zs:records>
                <zs:record>
                    <zs:recordSchema>mods</zs:recordSchema>
                    <zs:recordPacking>xml</zs:recordPacking>
                    <zs:recordData><mods xmlns="http://www.loc.gov/mods/v3">
                        <titleInfo>
                            <nonSort xml:space="preserve">The </nonSort>
                            <title>lord of the rings</title>
                        </titleInfo>
                        <name type="personal" usage="primary">
                            <namePart>Tolkien, J. R. R. (John Ronald Reuel),</namePart>
                            <namePart type="date">1892-1973</namePart>
                        </name>
                        <originInfo>
                            <agent><namePart>Houghton Mifflin</namePart></agent>
                            <dateIssued>[2003]</dateIssued>
                        </originInfo>
                        <language>
                            <languageTerm authority="iso639-2b" type="code">eng</languageTerm>
                        </language>
                        <physicalDescription>
                            <extent>xvi, 1137 p.</extent>
                        </physicalDescription>
                        <identifier type="isbn">9780618346257</identifier>
                        <identifier type="lccn">2004541379</identifier>
                        <classification authority="lcc">PR6039.O32 L6 2003b</classification>
                        <classification authority="ddc">823/.912</classification>
                    </mods></zs:recordData>
                </zs:record>
            </zs:records>
        </zs:searchRetrieveResponse>
        "#;

        let records = LocProvider::parse_sru_response(xml);
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.non_sort.as_deref(), Some("The "));
        assert_eq!(r.title.as_deref(), Some("lord of the rings"));
        assert_eq!(r.names.len(), 1);
        assert_eq!(r.names[0].name, "Tolkien, J. R. R. (John Ronald Reuel),");
        assert!(r.names[0].is_primary);
        assert_eq!(r.publisher.as_deref(), Some("Houghton Mifflin"));
        assert_eq!(r.date_issued.as_deref(), Some("[2003]"));
        assert_eq!(r.language_code.as_deref(), Some("eng"));
        assert!(r.isbns.contains(&"9780618346257".to_string()));
        assert_eq!(r.lccn.as_deref(), Some("2004541379"));

        // Verify full conversion
        let metadata = LocProvider::record_to_metadata(records.into_iter().next().unwrap(), 0.95);
        assert_eq!(metadata.title.as_deref(), Some("The Lord of the Rings"));
        assert_eq!(metadata.authors[0].name, "J. R. R. Tolkien");
        assert_eq!(metadata.publisher.as_deref(), Some("Houghton Mifflin"));
        assert_eq!(metadata.publication_year, Some(2003));
        assert_eq!(metadata.language.as_deref(), Some("en"));
        assert_eq!(metadata.page_count, Some(1137));
        crate::types::assert_isbns_valid(&metadata);
    }

    // ── SRU URL building ────────────────────────────────────────────

    #[test]
    fn build_sru_url_isbn() {
        let url_str = LocProvider::build_sru_url("bath.isbn=9780441172719", 3);
        let parsed = url::Url::parse(&url_str).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params["query"], "bath.isbn=9780441172719");
        assert_eq!(params["maximumRecords"], "3");
        assert_eq!(params["recordSchema"], "mods");
        assert_eq!(params["operation"], "searchRetrieve");
    }

    #[test]
    fn build_sru_url_title() {
        let url_str = LocProvider::build_sru_url("bath.title=\"The Lord of the Rings\"", 5);
        let parsed = url::Url::parse(&url_str).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params["query"], "bath.title=\"The Lord of the Rings\"");
        assert_eq!(params["maximumRecords"], "5");
    }

    #[test]
    fn build_sru_url_title_and_author() {
        let url_str =
            LocProvider::build_sru_url("bath.title=\"Dune\" and bath.author=\"Herbert\"", 5);
        let parsed = url::Url::parse(&url_str).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(
            params["query"],
            "bath.title=\"Dune\" and bath.author=\"Herbert\""
        );
    }

    // ── ISBN MARC punctuation ─────────────────────────────────────

    #[test]
    fn record_to_metadata_strips_isbn_marc_punctuation() {
        let record = ModsRecord {
            title: Some("Test".to_string()),
            isbns: vec![
                "0743535308 :".to_string(),
                "978-0-7435-3530-4 (pbk.)".to_string(),
            ],
            ..ModsRecord::default()
        };
        let metadata = LocProvider::record_to_metadata(record, 0.9);
        assert!(metadata
            .identifiers
            .iter()
            .any(|id| id.identifier_type == IdentifierType::Isbn10 && id.value == "0743535308"));
        assert!(metadata
            .identifiers
            .iter()
            .any(|id| id.identifier_type == IdentifierType::Isbn13 && id.value == "9780743535304"));
        crate::types::assert_isbns_valid(&metadata);
    }

    // ── Author deduplication ──────────────────────────────────────

    #[test]
    fn record_to_metadata_deduplicates_authors() {
        let record = ModsRecord {
            title: Some("Moral Letters".to_string()),
            names: vec![
                ModsName {
                    name: "Seneca, Lucius Annaeus".to_string(),
                    role: Some("author".to_string()),
                    is_primary: true,
                },
                ModsName {
                    name: "Campbell, Robin".to_string(),
                    role: Some("translator".to_string()),
                    is_primary: false,
                },
                ModsName {
                    name: "Seneca, Lucius Annaeus".to_string(),
                    role: Some("author of introduction".to_string()),
                    is_primary: false,
                },
            ],
            ..ModsRecord::default()
        };
        let metadata = LocProvider::record_to_metadata(record, 0.9);
        assert_eq!(metadata.authors.len(), 2);
        assert_eq!(metadata.authors[0].name, "Lucius Annaeus Seneca");
        assert_eq!(metadata.authors[1].name, "Robin Campbell");
    }

    // ── Role normalization ────────────────────────────────────────

    #[test]
    fn record_to_metadata_normalizes_marc_roles() {
        let record = ModsRecord {
            title: Some("Anthology".to_string()),
            names: vec![
                ModsName {
                    name: "Smith, John".to_string(),
                    role: Some("ed".to_string()),
                    is_primary: true,
                },
                ModsName {
                    name: "Doe, Jane".to_string(),
                    role: Some("trl".to_string()),
                    is_primary: false,
                },
            ],
            ..ModsRecord::default()
        };
        let metadata = LocProvider::record_to_metadata(record, 0.9);
        assert_eq!(metadata.authors[0].role.as_deref(), Some("editor"));
        assert_eq!(metadata.authors[1].role.as_deref(), Some("translator"));
    }

    #[test]
    fn normalize_marc_role_strips_trailing_dot() {
        assert_eq!(normalize_marc_role("ed."), "editor");
        assert_eq!(normalize_marc_role("trl."), "translator");
    }

    #[test]
    fn normalize_marc_role_passes_through_unknown() {
        assert_eq!(normalize_marc_role("xyz"), "xyz");
    }

    #[test]
    fn normalize_marc_role_known_codes() {
        assert_eq!(normalize_marc_role("edt"), "editor");
        assert_eq!(normalize_marc_role("aut"), "author");
        assert_eq!(normalize_marc_role("ill"), "illustrator");
        assert_eq!(normalize_marc_role("com"), "compiler");
        assert_eq!(normalize_marc_role("ctb"), "contributor");
        assert_eq!(normalize_marc_role("nrt"), "narrator");
        assert_eq!(normalize_marc_role("aui"), "author of introduction");
    }

    // ── MARC trailing period stripping ───────────────────────────────

    #[test]
    fn clean_name_marc_trailing_period() {
        assert_eq!(clean_author_name("Huber, Anna Lee."), "Anna Lee Huber");
    }

    #[test]
    fn clean_name_marc_period_preserves_initial() {
        assert_eq!(clean_author_name("Le Guin, Ursula K."), "Ursula K. Le Guin");
    }

    #[test]
    fn clean_name_marc_period_single_name() {
        assert_eq!(clean_author_name("Voltaire."), "Voltaire");
    }

    // ── Series position parsing ──────────────────────────────────────

    #[test]
    fn parse_series_position_plain_number() {
        assert!((parse_series_position("1").unwrap() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_position_prefixed() {
        assert!((parse_series_position("bk. 2").unwrap() - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_position_decimal() {
        assert!((parse_series_position("1.5").unwrap() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_series_position_no_number() {
        assert!(parse_series_position("vol").is_none());
    }

    // ── Series extraction from MODS ──────────────────────────────────

    #[test]
    fn parse_mods_series_with_position() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo><title>Mortal Arts</title></titleInfo>
            <relatedItem type="series">
                <titleInfo>
                    <title>A Lady Darby mystery</title>
                    <partNumber>1</partNumber>
                </titleInfo>
            </relatedItem>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.series_title.as_deref(), Some("A Lady Darby mystery"));
        assert_eq!(record.series_position.as_deref(), Some("1"));
    }

    #[test]
    fn parse_mods_series_without_position() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo><title>Some Book</title></titleInfo>
            <relatedItem type="series">
                <titleInfo>
                    <title>Great Series</title>
                </titleInfo>
            </relatedItem>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.series_title.as_deref(), Some("Great Series"));
        assert!(record.series_position.is_none());
    }

    #[test]
    fn parse_mods_non_series_related_item_ignored() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo><title>Some Book</title></titleInfo>
            <relatedItem type="otherFormat">
                <titleInfo>
                    <title>Electronic version</title>
                </titleInfo>
            </relatedItem>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert!(record.series_title.is_none());
    }

    #[test]
    fn record_to_metadata_includes_titlecased_series() {
        let record = ModsRecord {
            title: Some("Mortal Arts".to_string()),
            series_title: Some("A Lady Darby mystery".to_string()),
            series_position: Some("1".to_string()),
            ..ModsRecord::default()
        };
        let metadata = LocProvider::record_to_metadata(record, 0.9);
        let series = metadata.series.expect("should have series");
        assert_eq!(series.name, "A Lady Darby Mystery");
        assert!((series.position.unwrap() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_mods_series_does_not_clobber_primary_title() {
        let xml = r#"
        <mods xmlns="http://www.loc.gov/mods/v3">
            <titleInfo><title>Mortal Arts</title></titleInfo>
            <relatedItem type="series">
                <titleInfo>
                    <title>A Lady Darby mystery</title>
                    <partNumber>1</partNumber>
                </titleInfo>
            </relatedItem>
        </mods>
        "#;
        let record = parse_mods_record(xml).unwrap();
        assert_eq!(record.title.as_deref(), Some("Mortal Arts"));
        assert_eq!(record.series_title.as_deref(), Some("A Lady Darby mystery"));
    }
}
