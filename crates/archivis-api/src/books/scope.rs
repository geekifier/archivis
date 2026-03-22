use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use archivis_core::models::LibraryFilterState;

type HmacSha256 = Hmac<Sha256>;

const MAC_LEN: usize = 32;

/// Sign a `LibraryFilterState` into an opaque scope token.
///
/// Token format: `base64url(HMAC-SHA256 || canonical-JSON)`.
pub fn sign_scope(key: &[u8; 32], filter: &LibraryFilterState) -> String {
    let json = serde_json::to_vec(filter).expect("LibraryFilterState is always serializable");
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(&json);
    let tag = mac.finalize().into_bytes();

    let mut payload = Vec::with_capacity(MAC_LEN + json.len());
    payload.extend_from_slice(&tag);
    payload.extend_from_slice(&json);

    URL_SAFE_NO_PAD.encode(&payload)
}

/// Verify a scope token and extract the embedded `LibraryFilterState`.
///
/// Returns `Err` if the token is malformed, truncated, or tampered with.
pub fn verify_scope(key: &[u8; 32], token: &str) -> Result<LibraryFilterState, ScopeError> {
    let raw = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| ScopeError::InvalidToken)?;

    if raw.len() <= MAC_LEN {
        return Err(ScopeError::InvalidToken);
    }

    let (tag_bytes, json_bytes) = raw.split_at(MAC_LEN);

    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(json_bytes);
    mac.verify_slice(tag_bytes)
        .map_err(|_| ScopeError::TamperedToken)?;

    serde_json::from_slice(json_bytes).map_err(|_| ScopeError::InvalidToken)
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("invalid scope token")]
    InvalidToken,
    #[error("scope token has been tampered with")]
    TamperedToken,
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use archivis_core::models::TagMatchMode;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    fn sample_filter() -> LibraryFilterState {
        LibraryFilterState {
            text_query: Some("brandon sanderson".into()),
            language: Some("en".into()),
            tag_ids: vec![Uuid::nil()],
            tag_match: TagMatchMode::All,
            ..Default::default()
        }
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = test_key();
        let filter = sample_filter();
        let token = sign_scope(&key, &filter);
        let recovered = verify_scope(&key, &token).unwrap();
        assert_eq!(recovered, filter);
    }

    #[test]
    fn empty_filter_roundtrip() {
        let key = test_key();
        let filter = LibraryFilterState::default();
        let token = sign_scope(&key, &filter);
        let recovered = verify_scope(&key, &token).unwrap();
        assert_eq!(recovered, filter);
    }

    #[test]
    fn wrong_key_rejects() {
        let key = test_key();
        let filter = sample_filter();
        let token = sign_scope(&key, &filter);
        let wrong_key = [99u8; 32];
        let result = verify_scope(&wrong_key, &token);
        assert!(matches!(result, Err(ScopeError::TamperedToken)));
    }

    #[test]
    fn tampered_payload_rejects() {
        let key = test_key();
        let filter = sample_filter();
        let token = sign_scope(&key, &filter);

        // Decode, flip a byte in the JSON portion, re-encode
        let mut raw = URL_SAFE_NO_PAD.decode(&token).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        let tampered = URL_SAFE_NO_PAD.encode(&raw);

        let result = verify_scope(&key, &tampered);
        assert!(matches!(result, Err(ScopeError::TamperedToken)));
    }

    #[test]
    fn tampered_mac_rejects() {
        let key = test_key();
        let filter = sample_filter();
        let token = sign_scope(&key, &filter);

        // Decode, flip a byte in the MAC portion, re-encode
        let mut raw = URL_SAFE_NO_PAD.decode(&token).unwrap();
        raw[0] ^= 0xFF;
        let tampered = URL_SAFE_NO_PAD.encode(&raw);

        let result = verify_scope(&key, &tampered);
        assert!(matches!(result, Err(ScopeError::TamperedToken)));
    }

    #[test]
    fn truncated_token_rejects() {
        let result = verify_scope(&test_key(), "AAAA");
        assert!(matches!(result, Err(ScopeError::InvalidToken)));
    }

    #[test]
    fn empty_token_rejects() {
        let result = verify_scope(&test_key(), "");
        assert!(matches!(result, Err(ScopeError::InvalidToken)));
    }

    #[test]
    fn garbage_token_rejects() {
        let result = verify_scope(&test_key(), "not-valid-base64!!!");
        assert!(matches!(result, Err(ScopeError::InvalidToken)));
    }
}
