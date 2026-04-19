//! One-shot normalization of legacy DB rows against the current registry.
//!
//! This runs at boot, before the `SettingStore` is constructed. It:
//!
//! * Canonicalizes each row by the registry-declared type (e.g. `"2000"` →
//!   `2000`, `"false"` → `false`, `2000.0` → `2000` for integer types).
//! * Deletes rows equal to the registry default.
//! * Hard-fails on unknown keys or keys that belong to the bootstrap scope.
//!
//! After this step the DB contains only valid runtime rows, canonicalized.

use archivis_core::settings::{
    canonicalize, get_bootstrap_meta, get_runtime_meta, runtime_default, SettingValue,
};
use archivis_db::{DbPool, SettingRepository};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum NormalizeError {
    #[error("unknown runtime setting in database: {key} (value: {value})")]
    UnknownKey { key: String, value: String },
    #[error(
        "bootstrap setting found in database: {key} — bootstrap keys are read-only and must be set via config file, env, or CLI"
    )]
    BootstrapInDb { key: String },
    #[error("invalid value for {key}: {message}")]
    Invalid { key: String, message: String },
    #[error("database error: {0}")]
    Db(#[from] archivis_core::errors::DbError),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Outcome of normalization.
#[derive(Debug)]
pub struct NormalizeResult {
    pub rows: Vec<(String, SettingValue)>,
}

/// Load all DB rows, canonicalize them, write back any needed corrections,
/// delete rows equal to the default, and fail hard on unknown / bootstrap keys.
pub async fn normalize_settings(pool: &DbPool) -> Result<NormalizeResult, NormalizeError> {
    let raw = SettingRepository::get_all(pool).await?;

    let mut rewrites: Vec<(String, String)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();
    let mut canonical_rows: Vec<(String, SettingValue)> = Vec::new();

    for (key, raw_value) in raw {
        // Reject bootstrap keys: they should never live in the DB.
        if get_bootstrap_meta(&key).is_some() {
            return Err(NormalizeError::BootstrapInDb { key });
        }

        // Unknown keys → hard fail.
        let Some(meta) = get_runtime_meta(&key) else {
            return Err(NormalizeError::UnknownKey {
                key,
                value: raw_value,
            });
        };

        // Parse stored string as JSON; fall back to treating it as a string.
        let parsed: Value = serde_json::from_str::<Value>(&raw_value)
            .unwrap_or_else(|_| Value::String(raw_value.clone()));

        let canonical =
            canonicalize(meta.value_type, &parsed).map_err(|e| NormalizeError::Invalid {
                key: key.clone(),
                message: e.message,
            })?;

        // Ensure value passes the validator (may still be DB-pinned garbage).
        (meta.validator)(&canonical).map_err(|e| NormalizeError::Invalid {
            key: key.clone(),
            message: e.message,
        })?;

        let default_val = runtime_default(meta);

        if canonical == default_val {
            deletes.push(key);
            continue;
        }

        // If the canonical rewrite differs from what's stored, queue a rewrite.
        let canonical_text = serde_json::to_string(&canonical)?;
        if canonical_text != raw_value {
            rewrites.push((key.clone(), canonical_text));
        }

        canonical_rows.push((key, canonical));
    }

    // Persist normalization writes.
    for (key, value) in &rewrites {
        SettingRepository::set(pool, key, value).await?;
    }
    for key in &deletes {
        SettingRepository::delete(pool, key).await?;
    }

    Ok(NormalizeResult {
        rows: canonical_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archivis_core::settings::SettingType;
    use serde_json::json;

    async fn make_pool() -> (DbPool, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.db");
        let pool = archivis_db::create_pool(&path).await.unwrap();
        archivis_db::run_migrations(&pool).await.unwrap();
        (pool, tmp)
    }

    #[tokio::test]
    async fn string_encoded_integer_is_rewritten() {
        let (pool, _tmp) = make_pool().await;
        SettingRepository::set(&pool, "isbn_scan.pdf_pages", "\"10\"")
            .await
            .unwrap();

        let result = normalize_settings(&pool).await.unwrap();
        let (k, v) = &result.rows[0];
        assert_eq!(k, "isbn_scan.pdf_pages");
        assert_eq!(v, &json!(10));

        let raw = SettingRepository::get(&pool, "isbn_scan.pdf_pages")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(raw, "10");
        let _ = SettingType::Integer;
    }

    #[tokio::test]
    async fn row_equal_to_default_is_deleted() {
        let (pool, _tmp) = make_pool().await;
        SettingRepository::set(&pool, "isbn_scan.pdf_pages", "5")
            .await
            .unwrap();

        let result = normalize_settings(&pool).await.unwrap();
        assert!(result.rows.is_empty());
        let raw = SettingRepository::get(&pool, "isbn_scan.pdf_pages")
            .await
            .unwrap();
        assert!(raw.is_none());
    }

    #[tokio::test]
    async fn unknown_key_hard_fails() {
        let (pool, _tmp) = make_pool().await;
        SettingRepository::set(&pool, "some.bogus.key", "42")
            .await
            .unwrap();
        let err = normalize_settings(&pool).await.unwrap_err();
        assert!(matches!(err, NormalizeError::UnknownKey { .. }));
    }

    #[tokio::test]
    async fn bootstrap_in_db_hard_fails() {
        let (pool, _tmp) = make_pool().await;
        SettingRepository::set(&pool, "port", "9090").await.unwrap();
        let err = normalize_settings(&pool).await.unwrap_err();
        assert!(matches!(err, NormalizeError::BootstrapInDb { .. }));
    }
}
