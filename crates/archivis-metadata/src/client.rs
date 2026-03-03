use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::errors::ProviderError;

/// Token bucket rate limiter for a single provider.
struct RateLimiter {
    tokens: AtomicU32,
    max_tokens: u32,
    refill_interval: Duration,
    last_refill: Mutex<Instant>,
}

impl RateLimiter {
    fn new(max_requests_per_minute: u32) -> Self {
        // Each token represents one request. Refill interval is calculated
        // so that over one minute, max_requests_per_minute tokens are added.
        let refill_interval = if max_requests_per_minute == 0 {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(60) / max_requests_per_minute
        };

        Self {
            tokens: AtomicU32::new(max_requests_per_minute),
            max_tokens: max_requests_per_minute,
            refill_interval,
            last_refill: Mutex::new(Instant::now()),
        }
    }

    /// Refill tokens based on elapsed time since last refill.
    async fn refill(&self) {
        let mut last_refill = self.last_refill.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill);
        let new_tokens =
            u32::try_from(elapsed.as_millis() / self.refill_interval.as_millis().max(1))
                .unwrap_or(u32::MAX);

        if new_tokens > 0 {
            let current = self.tokens.load(Ordering::Relaxed);
            let refilled = (current + new_tokens).min(self.max_tokens);
            self.tokens.store(refilled, Ordering::Relaxed);
            *last_refill = now;
        }
    }

    /// Attempt to acquire a token, waiting if necessary.
    async fn acquire(&self) {
        loop {
            self.refill().await;

            let current = self.tokens.load(Ordering::Relaxed);
            if current > 0
                && self
                    .tokens
                    .compare_exchange(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                return;
            }

            // No tokens available — wait for one refill interval before retrying.
            tokio::time::sleep(self.refill_interval).await;
        }
    }
}

/// Rate-limited HTTP client wrapper shared by all metadata providers.
///
/// Handles per-provider rate limiting, retries with exponential backoff,
/// and sets a consistent User-Agent header for API identification.
pub struct MetadataHttpClient {
    client: reqwest::Client,
    rate_limiters: HashMap<String, Arc<RateLimiter>>,
    user_agent: String,
}

impl MetadataHttpClient {
    /// Create a new HTTP client.
    ///
    /// `version` — application version string (e.g. "0.1.0").
    /// `contact` — optional contact email for API identification.
    pub fn new(version: &str, contact: Option<&str>) -> Self {
        let user_agent = contact.map_or_else(
            || format!("Archivis/{version} (+https://github.com/geekifier/archivis)"),
            |email| format!("Archivis/{version} ({email}; +https://github.com/geekifier/archivis)"),
        );

        let mut default_headers = HeaderMap::new();
        if let Ok(ua_value) = HeaderValue::from_str(&user_agent) {
            default_headers.insert(USER_AGENT, ua_value);
        }

        let client = reqwest::Client::builder()
            .default_headers(default_headers)
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            rate_limiters: HashMap::new(),
            user_agent,
        }
    }

    /// Register a per-provider rate limiter.
    pub fn register_provider(&mut self, name: &str, max_requests_per_minute: u32) {
        self.rate_limiters.insert(
            name.to_string(),
            Arc::new(RateLimiter::new(max_requests_per_minute)),
        );
    }

    /// Returns the User-Agent string.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Perform a GET request, respecting rate limits and retrying on
    /// 429/503 with exponential backoff (max 3 retries).
    pub async fn get(
        &self,
        provider_name: &str,
        url: &str,
    ) -> Result<reqwest::Response, ProviderError> {
        self.wait_for_token(provider_name).await;

        let mut retries = 0u32;
        let max_retries = 3;

        loop {
            debug!(provider = provider_name, url = url, "GET request");

            let response = self.client.get(url).send().await?;
            let status = response.status().as_u16();

            if status == 429 || status == 503 {
                retries += 1;
                if retries > max_retries {
                    warn!(
                        provider = provider_name,
                        status = status,
                        "max retries exceeded"
                    );
                    return Err(ProviderError::ApiError {
                        status,
                        message: format!("request failed after {max_retries} retries"),
                    });
                }

                let backoff = Duration::from_secs(1 << (retries - 1));
                warn!(
                    provider = provider_name,
                    status = status,
                    retry = retries,
                    backoff_secs = backoff.as_secs(),
                    "retrying after backoff"
                );
                tokio::time::sleep(backoff).await;
                self.wait_for_token(provider_name).await;
                continue;
            }

            return Ok(response);
        }
    }

    /// Perform a POST request with a JSON body, respecting rate limits and
    /// retrying on 429/503 with exponential backoff (max 3 retries).
    pub async fn post_json<T: serde::Serialize + Sync + ?Sized>(
        &self,
        provider_name: &str,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response, ProviderError> {
        self.wait_for_token(provider_name).await;

        let mut retries = 0u32;
        let max_retries = 3;

        loop {
            debug!(provider = provider_name, url = url, "POST request");

            let response = self
                .client
                .post(url)
                .header(CONTENT_TYPE, "application/json")
                .json(body)
                .send()
                .await?;

            let status = response.status().as_u16();

            if status == 429 || status == 503 {
                retries += 1;
                if retries > max_retries {
                    warn!(
                        provider = provider_name,
                        status = status,
                        "max retries exceeded"
                    );
                    return Err(ProviderError::ApiError {
                        status,
                        message: format!("request failed after {max_retries} retries"),
                    });
                }

                let backoff = Duration::from_secs(1 << (retries - 1));
                warn!(
                    provider = provider_name,
                    status = status,
                    retry = retries,
                    backoff_secs = backoff.as_secs(),
                    "retrying after backoff"
                );
                tokio::time::sleep(backoff).await;
                self.wait_for_token(provider_name).await;
                continue;
            }

            return Ok(response);
        }
    }

    /// Perform a POST request with a JSON body and additional headers,
    /// respecting rate limits and retrying on 429/503 with exponential
    /// backoff (max 3 retries).
    ///
    /// Extra headers are provided as `(name, value)` pairs and are added
    /// on top of the default headers.
    pub async fn post_json_with_headers<T: serde::Serialize + Sync + ?Sized>(
        &self,
        provider_name: &str,
        url: &str,
        body: &T,
        extra_headers: &[(&str, &str)],
    ) -> Result<reqwest::Response, ProviderError> {
        self.wait_for_token(provider_name).await;

        let mut retries = 0u32;
        let max_retries = 3;

        loop {
            debug!(
                provider = provider_name,
                url = url,
                "POST request (with headers)"
            );

            let mut request = self
                .client
                .post(url)
                .header(CONTENT_TYPE, "application/json");

            for &(name, value) in extra_headers {
                request = request.header(name, value);
            }

            let response = request.json(body).send().await?;

            let status = response.status().as_u16();

            if status == 429 || status == 503 {
                retries += 1;
                if retries > max_retries {
                    warn!(
                        provider = provider_name,
                        status = status,
                        "max retries exceeded"
                    );
                    return Err(ProviderError::ApiError {
                        status,
                        message: format!("request failed after {max_retries} retries"),
                    });
                }

                let backoff = Duration::from_secs(1 << (retries - 1));
                warn!(
                    provider = provider_name,
                    status = status,
                    retry = retries,
                    backoff_secs = backoff.as_secs(),
                    "retrying after backoff"
                );
                tokio::time::sleep(backoff).await;
                self.wait_for_token(provider_name).await;
                continue;
            }

            return Ok(response);
        }
    }

    /// Returns the underlying `reqwest::Client` for direct use when
    /// rate limiting is not needed (e.g., fetching cover images from CDNs).
    pub fn raw_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Wait for a rate limiter token for the given provider.
    /// If no rate limiter is registered for this provider, returns immediately.
    async fn wait_for_token(&self, provider_name: &str) {
        if let Some(limiter) = self.rate_limiters.get(provider_name) {
            limiter.acquire().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_with_contact() {
        let client = MetadataHttpClient::new("0.1.0", Some("admin@example.com"));
        assert_eq!(
            client.user_agent(),
            "Archivis/0.1.0 (admin@example.com; +https://github.com/geekifier/archivis)"
        );
    }

    #[test]
    fn user_agent_without_contact() {
        let client = MetadataHttpClient::new("0.1.0", None);
        assert_eq!(
            client.user_agent(),
            "Archivis/0.1.0 (+https://github.com/geekifier/archivis)"
        );
    }

    #[test]
    fn register_provider_rate_limiter() {
        let mut client = MetadataHttpClient::new("0.1.0", None);
        client.register_provider("open_library", 100);
        client.register_provider("hardcover", 50);

        assert!(client.rate_limiters.contains_key("open_library"));
        assert!(client.rate_limiters.contains_key("hardcover"));
    }

    #[tokio::test]
    async fn rate_limiter_allows_requests_within_budget() {
        let limiter = RateLimiter::new(10);

        // Should be able to acquire several tokens immediately.
        for _ in 0..5 {
            limiter.acquire().await;
        }

        let remaining = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(remaining, 5);
    }

    #[tokio::test]
    async fn rate_limiter_refills_tokens() {
        let limiter = RateLimiter::new(60); // 1 token per second

        // Drain all tokens.
        for _ in 0..60 {
            limiter.acquire().await;
        }
        assert_eq!(limiter.tokens.load(Ordering::Relaxed), 0);

        // Wait enough time for some tokens to refill.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        limiter.refill().await;

        let tokens = limiter.tokens.load(Ordering::Relaxed);
        assert!(
            tokens >= 1,
            "expected at least 1 refilled token, got {tokens}"
        );
    }

    #[tokio::test]
    async fn rate_limiter_does_not_exceed_max() {
        let limiter = RateLimiter::new(5);

        // Wait to allow refill beyond max.
        tokio::time::sleep(Duration::from_millis(100)).await;
        limiter.refill().await;

        let tokens = limiter.tokens.load(Ordering::Relaxed);
        assert!(tokens <= 5, "tokens should not exceed max: {tokens}");
    }
}
