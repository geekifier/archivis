use std::net::IpAddr;

use axum::http::HeaderMap;
use ipnet::IpNet;

/// Information extracted from proxy authentication headers.
#[derive(Debug, Clone)]
pub struct ProxyUserInfo {
    pub username: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
}

/// Reverse proxy (`ForwardAuth`) authentication handler.
///
/// Validates that incoming requests originate from trusted proxy IPs and
/// extracts user identity from configurable HTTP headers.
#[derive(Debug, Clone)]
pub struct ProxyAuth {
    trusted_networks: Vec<IpNet>,
    user_header: String,
    email_header: Option<String>,
    groups_header: Option<String>,
}

impl ProxyAuth {
    /// Create a new `ProxyAuth` from raw configuration values.
    ///
    /// Parses CIDR strings and bare IP addresses into `IpNet` networks.
    /// Returns an error if any entry is invalid.
    pub fn new(
        trusted_proxies: &[String],
        user_header: String,
        email_header: Option<String>,
        groups_header: Option<String>,
    ) -> Result<Self, String> {
        let mut trusted_networks = Vec::with_capacity(trusted_proxies.len());
        for entry in trusted_proxies {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Try parsing as CIDR first, then as a bare IP address.
            let net: IpNet = trimmed
                .parse()
                .or_else(|_| {
                    trimmed
                        .parse::<IpAddr>()
                        .map(IpNet::from)
                        .map_err(|e| e.to_string())
                })
                .map_err(|e| format!("invalid trusted proxy entry `{trimmed}`: {e}"))?;
            trusted_networks.push(net);
        }

        Ok(Self {
            trusted_networks,
            user_header,
            email_header,
            groups_header,
        })
    }

    /// Check whether `addr` matches any of the configured trusted proxy networks.
    pub fn is_trusted_proxy(&self, addr: &IpAddr) -> bool {
        self.trusted_networks.iter().any(|net| net.contains(addr))
    }

    /// Extract user information from request headers.
    ///
    /// Returns `None` if the required user header is missing or empty.
    pub fn extract_user_info(&self, headers: &HeaderMap) -> Option<ProxyUserInfo> {
        let username = headers
            .get(&self.user_header)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();

        let email = self.email_header.as_ref().and_then(|h| {
            headers
                .get(h)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        });

        let groups = self
            .groups_header
            .as_ref()
            .and_then(|h| {
                headers.get(h).and_then(|v| v.to_str().ok()).map(|s| {
                    s.split(',')
                        .map(|g| g.trim().to_string())
                        .filter(|g| !g.is_empty())
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();

        Some(ProxyUserInfo {
            username,
            email,
            groups,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_proxy(trusted: &[&str]) -> ProxyAuth {
        ProxyAuth::new(
            &trusted.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
            "X-Forwarded-User".to_string(),
            Some("X-Forwarded-Email".to_string()),
            Some("X-Forwarded-Groups".to_string()),
        )
        .unwrap()
    }

    // ── CIDR parsing ───────────────────────────────────────────

    #[test]
    fn parse_ipv4_cidr() {
        let proxy = default_proxy(&["192.168.1.0/24"]);
        assert_eq!(proxy.trusted_networks.len(), 1);
    }

    #[test]
    fn parse_ipv6_cidr() {
        let proxy = default_proxy(&["fd00::/8"]);
        assert_eq!(proxy.trusted_networks.len(), 1);
    }

    #[test]
    fn parse_bare_ipv4_address() {
        let proxy = default_proxy(&["10.0.0.1"]);
        assert_eq!(proxy.trusted_networks.len(), 1);
    }

    #[test]
    fn parse_bare_ipv6_address() {
        let proxy = default_proxy(&["::1"]);
        assert_eq!(proxy.trusted_networks.len(), 1);
    }

    #[test]
    fn parse_multiple_entries() {
        let proxy = default_proxy(&["10.0.0.0/8", "172.16.0.0/12", "192.168.0.1"]);
        assert_eq!(proxy.trusted_networks.len(), 3);
    }

    #[test]
    fn parse_empty_entries_skipped() {
        let proxy = default_proxy(&["10.0.0.1", "", "  "]);
        assert_eq!(proxy.trusted_networks.len(), 1);
    }

    #[test]
    fn parse_invalid_cidr_returns_error() {
        let result = ProxyAuth::new(
            &["not-an-ip".to_string()],
            "X-Forwarded-User".to_string(),
            None,
            None,
        );
        assert!(result.is_err());
    }

    // ── IP matching ────────────────────────────────────────────

    #[test]
    fn ipv4_in_cidr_range() {
        let proxy = default_proxy(&["192.168.1.0/24"]);
        assert!(proxy.is_trusted_proxy(&"192.168.1.42".parse().unwrap()));
    }

    #[test]
    fn ipv4_outside_cidr_range() {
        let proxy = default_proxy(&["192.168.1.0/24"]);
        assert!(!proxy.is_trusted_proxy(&"192.168.2.1".parse().unwrap()));
    }

    #[test]
    fn ipv4_exact_match() {
        let proxy = default_proxy(&["10.0.0.5"]);
        assert!(proxy.is_trusted_proxy(&"10.0.0.5".parse().unwrap()));
        assert!(!proxy.is_trusted_proxy(&"10.0.0.6".parse().unwrap()));
    }

    #[test]
    fn ipv6_in_cidr_range() {
        let proxy = default_proxy(&["fd00::/8"]);
        assert!(proxy.is_trusted_proxy(&"fd12:3456::1".parse().unwrap()));
    }

    #[test]
    fn ipv6_exact_match() {
        let proxy = default_proxy(&["::1"]);
        assert!(proxy.is_trusted_proxy(&"::1".parse().unwrap()));
        assert!(!proxy.is_trusted_proxy(&"::2".parse().unwrap()));
    }

    #[test]
    fn empty_trusted_list_trusts_nobody() {
        let proxy = default_proxy(&[]);
        assert!(!proxy.is_trusted_proxy(&"127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn multiple_networks_any_match() {
        let proxy = default_proxy(&["10.0.0.0/8", "172.16.0.0/12"]);
        assert!(proxy.is_trusted_proxy(&"10.1.2.3".parse().unwrap()));
        assert!(proxy.is_trusted_proxy(&"172.20.0.1".parse().unwrap()));
        assert!(!proxy.is_trusted_proxy(&"192.168.0.1".parse().unwrap()));
    }

    // ── Header extraction ──────────────────────────────────────

    #[test]
    fn extract_all_headers() {
        let proxy = default_proxy(&[]);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", "alice".parse().unwrap());
        headers.insert("X-Forwarded-Email", "alice@example.com".parse().unwrap());
        headers.insert("X-Forwarded-Groups", "users,admins".parse().unwrap());

        let info = proxy.extract_user_info(&headers).unwrap();
        assert_eq!(info.username, "alice");
        assert_eq!(info.email.as_deref(), Some("alice@example.com"));
        assert_eq!(info.groups, vec!["users", "admins"]);
    }

    #[test]
    fn extract_username_only() {
        let proxy = ProxyAuth::new(&[], "X-Forwarded-User".to_string(), None, None).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", "bob".parse().unwrap());

        let info = proxy.extract_user_info(&headers).unwrap();
        assert_eq!(info.username, "bob");
        assert!(info.email.is_none());
        assert!(info.groups.is_empty());
    }

    #[test]
    fn missing_user_header_returns_none() {
        let proxy = default_proxy(&[]);
        let headers = HeaderMap::new();
        assert!(proxy.extract_user_info(&headers).is_none());
    }

    #[test]
    fn empty_user_header_returns_none() {
        let proxy = default_proxy(&[]);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", "".parse().unwrap());
        assert!(proxy.extract_user_info(&headers).is_none());
    }

    #[test]
    fn whitespace_only_user_header_returns_none() {
        let proxy = default_proxy(&[]);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", "  ".parse().unwrap());
        assert!(proxy.extract_user_info(&headers).is_none());
    }

    #[test]
    fn groups_with_whitespace_trimmed() {
        let proxy = default_proxy(&[]);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", "alice".parse().unwrap());
        headers.insert("X-Forwarded-Groups", " users , admins , ".parse().unwrap());

        let info = proxy.extract_user_info(&headers).unwrap();
        assert_eq!(info.groups, vec!["users", "admins"]);
    }

    #[test]
    fn empty_email_header_ignored() {
        let proxy = default_proxy(&[]);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", "alice".parse().unwrap());
        headers.insert("X-Forwarded-Email", "".parse().unwrap());

        let info = proxy.extract_user_info(&headers).unwrap();
        assert!(info.email.is_none());
    }
}
