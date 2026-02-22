use std::fmt;
use std::time::Duration;

/// Errors that metadata providers can produce.
#[derive(Debug)]
pub enum ProviderError {
    /// HTTP request failed (network, timeout).
    HttpError(reqwest::Error),
    /// Provider returned an error response.
    ApiError { status: u16, message: String },
    /// Rate limit exceeded — caller should retry after delay.
    RateLimited { retry_after: Option<Duration> },
    /// Response could not be parsed.
    ParseError(String),
    /// Provider is not configured (missing API key, disabled).
    NotConfigured(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpError(e) => write!(f, "HTTP request failed: {e}"),
            Self::ApiError { status, message } => {
                write!(f, "API error (HTTP {status}): {message}")
            }
            Self::RateLimited { retry_after } => {
                if let Some(duration) = retry_after {
                    write!(f, "rate limited, retry after {duration:?}")
                } else {
                    write!(f, "rate limited")
                }
            }
            Self::ParseError(msg) => write!(f, "failed to parse response: {msg}"),
            Self::NotConfigured(msg) => write!(f, "provider not configured: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HttpError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(err: reqwest::Error) -> Self {
        Self::HttpError(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_error_display() {
        // We can't easily construct a reqwest::Error, so test the other variants.
        let err = ProviderError::ApiError {
            status: 500,
            message: "internal server error".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "API error (HTTP 500): internal server error"
        );
    }

    #[test]
    fn rate_limited_with_duration() {
        let err = ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        };
        let display = err.to_string();
        assert!(display.contains("rate limited"));
        assert!(display.contains("30"));
    }

    #[test]
    fn rate_limited_without_duration() {
        let err = ProviderError::RateLimited { retry_after: None };
        assert_eq!(err.to_string(), "rate limited");
    }

    #[test]
    fn parse_error_display() {
        let err = ProviderError::ParseError("unexpected JSON".to_string());
        assert_eq!(err.to_string(), "failed to parse response: unexpected JSON");
    }

    #[test]
    fn not_configured_display() {
        let err = ProviderError::NotConfigured("missing API key".to_string());
        assert_eq!(err.to_string(), "provider not configured: missing API key");
    }
}
