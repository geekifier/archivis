use std::fmt;

use url::{ParseError, Position, Url};

/// Stable external origin used for generating absolute user-facing URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicBaseUrl {
    url: Url,
}

impl PublicBaseUrl {
    pub fn parse(input: &str) -> Result<Self, PublicBaseUrlError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(PublicBaseUrlError::Empty);
        }

        let url = Url::parse(trimmed).map_err(|err| match err {
            ParseError::RelativeUrlWithoutBase => PublicBaseUrlError::NotAbsolute,
            ParseError::EmptyHost => PublicBaseUrlError::MissingHost,
            _ => PublicBaseUrlError::Parse(err),
        })?;

        match url.scheme() {
            "http" | "https" => {}
            scheme => return Err(PublicBaseUrlError::UnsupportedScheme(scheme.to_string())),
        }

        if url.host().is_none() {
            return Err(PublicBaseUrlError::MissingHost);
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(PublicBaseUrlError::CredentialsNotAllowed);
        }

        if url.query().is_some() {
            return Err(PublicBaseUrlError::QueryNotAllowed);
        }

        if url.fragment().is_some() {
            return Err(PublicBaseUrlError::FragmentNotAllowed);
        }

        if url.path() != "/" {
            return Err(PublicBaseUrlError::NonRootPath);
        }

        Ok(Self { url })
    }

    pub fn as_origin(&self) -> &str {
        &self.url[..Position::BeforePath]
    }

    pub fn join_path(&self, path: &str) -> Result<Url, PublicBaseUrlJoinError> {
        if !path.starts_with('/') {
            return Err(PublicBaseUrlJoinError::RelativePath);
        }
        if path.starts_with("//") {
            return Err(PublicBaseUrlJoinError::NetworkPathReference);
        }

        self.url
            .join(path)
            .map_err(PublicBaseUrlJoinError::InvalidPath)
    }
}

impl fmt::Display for PublicBaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_origin())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PublicBaseUrlError {
    #[error("must not be empty")]
    Empty,
    #[error("must be an absolute URL")]
    NotAbsolute,
    #[error("must include a host")]
    MissingHost,
    #[error("must use http or https (got {0})")]
    UnsupportedScheme(String),
    #[error("must not include username or password")]
    CredentialsNotAllowed,
    #[error("must not include a query string")]
    QueryNotAllowed,
    #[error("must not include a fragment")]
    FragmentNotAllowed,
    #[error("must not include a path; Archivis currently expects a root URL")]
    NonRootPath,
    #[error("invalid URL: {0}")]
    Parse(ParseError),
}

#[derive(Debug, thiserror::Error)]
pub enum PublicBaseUrlJoinError {
    #[error("path must start with '/'")]
    RelativePath,
    #[error("path must not start with '//'")]
    NetworkPathReference,
    #[error("failed to join path: {0}")]
    InvalidPath(ParseError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_origin_and_normalizes_display() {
        let url = PublicBaseUrl::parse("https://books.example.com/").unwrap();
        assert_eq!(url.to_string(), "https://books.example.com");
    }

    #[test]
    fn parses_http_localhost_with_port() {
        let url = PublicBaseUrl::parse("http://localhost:9514").unwrap();
        assert_eq!(url.to_string(), "http://localhost:9514");
    }

    #[test]
    fn rejects_relative_url() {
        let err = PublicBaseUrl::parse("/archivis").unwrap_err();
        assert!(matches!(err, PublicBaseUrlError::NotAbsolute));
    }

    #[test]
    fn rejects_non_http_scheme() {
        let err = PublicBaseUrl::parse("ftp://books.example.com").unwrap_err();
        assert!(matches!(err, PublicBaseUrlError::UnsupportedScheme(_)));
    }

    #[test]
    fn rejects_credentials() {
        let err = PublicBaseUrl::parse("https://user:pass@books.example.com").unwrap_err();
        assert!(matches!(err, PublicBaseUrlError::CredentialsNotAllowed));
    }

    #[test]
    fn rejects_query_and_fragment() {
        let query_err = PublicBaseUrl::parse("https://books.example.com?x=1").unwrap_err();
        assert!(matches!(query_err, PublicBaseUrlError::QueryNotAllowed));

        let fragment_err = PublicBaseUrl::parse("https://books.example.com/#reader").unwrap_err();
        assert!(matches!(
            fragment_err,
            PublicBaseUrlError::FragmentNotAllowed
        ));
    }

    #[test]
    fn rejects_non_root_path() {
        let err = PublicBaseUrl::parse("https://books.example.com/archivis").unwrap_err();
        assert!(matches!(err, PublicBaseUrlError::NonRootPath));
    }

    #[test]
    fn joins_absolute_app_paths() {
        let base = PublicBaseUrl::parse("https://books.example.com").unwrap();
        let joined = base.join_path("/api/books/123").unwrap();
        assert_eq!(
            joined.to_string(),
            "https://books.example.com/api/books/123"
        );
    }

    #[test]
    fn rejects_ambiguous_join_paths() {
        let base = PublicBaseUrl::parse("https://books.example.com").unwrap();

        let relative = base.join_path("api/books/123").unwrap_err();
        assert!(matches!(relative, PublicBaseUrlJoinError::RelativePath));

        let network = base.join_path("//evil.example.com").unwrap_err();
        assert!(matches!(
            network,
            PublicBaseUrlJoinError::NetworkPathReference
        ));
    }
}
