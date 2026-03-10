use archivis_core::errors::DbError;
use archivis_core::models::{MatchMode, MetadataRule, MetadataRuleType, RuleOutcome};
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Row type for mapping from the `metadata_rules` table.
struct MetadataRuleRow {
    id: String,
    rule_type: String,
    match_value: String,
    match_mode: String,
    outcome: String,
    enabled: i64,
    builtin: i64,
    created_at: String,
}

impl MetadataRuleRow {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, DbError> {
        Ok(Self {
            id: row
                .try_get("id")
                .map_err(|e| DbError::Query(e.to_string()))?,
            rule_type: row
                .try_get("rule_type")
                .map_err(|e| DbError::Query(e.to_string()))?,
            match_value: row
                .try_get("match_value")
                .map_err(|e| DbError::Query(e.to_string()))?,
            match_mode: row
                .try_get("match_mode")
                .map_err(|e| DbError::Query(e.to_string()))?,
            outcome: row
                .try_get("outcome")
                .map_err(|e| DbError::Query(e.to_string()))?,
            enabled: row
                .try_get("enabled")
                .map_err(|e| DbError::Query(e.to_string()))?,
            builtin: row
                .try_get("builtin")
                .map_err(|e| DbError::Query(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| DbError::Query(e.to_string()))?,
        })
    }

    fn into_model(self) -> Result<MetadataRule, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid metadata_rule UUID: {e}")))?;
        let rule_type: MetadataRuleType = self
            .rule_type
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let match_mode: MatchMode = self
            .match_mode
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let outcome: RuleOutcome = self
            .outcome
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let created_at = parse_datetime(&self.created_at, "created_at")?;

        Ok(MetadataRule {
            id,
            rule_type,
            match_value: self.match_value,
            match_mode,
            outcome,
            enabled: self.enabled != 0,
            builtin: self.builtin != 0,
            created_at,
        })
    }
}

/// Parse an ISO 8601 datetime string, handling both RFC 3339 and `SQLite` default formats.
fn parse_datetime(s: &str, field: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|ndt| ndt.and_utc())
        })
        .map_err(|e| DbError::Query(format!("invalid {field}: {e}")))
}

const SELECT_COLS: &str =
    "id, rule_type, match_value, match_mode, outcome, enabled, builtin, created_at";

pub struct MetadataRuleRepository;

impl MetadataRuleRepository {
    /// List all rules (for API/settings UI).
    pub async fn list_all(pool: &SqlitePool) -> Result<Vec<MetadataRule>, DbError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM metadata_rules ORDER BY builtin DESC, created_at ASC"
        );
        let rows = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        rows.iter()
            .map(|r| MetadataRuleRow::from_row(r)?.into_model())
            .collect()
    }

    /// List only enabled rules (for import/resolution — loaded once per batch).
    pub async fn list_enabled(pool: &SqlitePool) -> Result<Vec<MetadataRule>, DbError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM metadata_rules WHERE enabled = 1 ORDER BY builtin DESC, created_at ASC"
        );
        let rows = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        rows.iter()
            .map(|r| MetadataRuleRow::from_row(r)?.into_model())
            .collect()
    }

    /// Get a single rule by ID.
    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<MetadataRule>, DbError> {
        let id_str = id.to_string();
        let sql = format!("SELECT {SELECT_COLS} FROM metadata_rules WHERE id = ?");
        let row = sqlx::query(&sql)
            .bind(&id_str)
            .fetch_optional(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        row.as_ref()
            .map(|r| MetadataRuleRow::from_row(r)?.into_model())
            .transpose()
    }

    /// Create a new metadata rule.
    pub async fn create(
        pool: &SqlitePool,
        rule_type: MetadataRuleType,
        match_value: &str,
        match_mode: MatchMode,
        outcome: RuleOutcome,
    ) -> Result<MetadataRule, DbError> {
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let rule_type_str = rule_type.to_string();
        let match_mode_str = match_mode.to_string();
        let outcome_str = outcome.to_string();

        sqlx::query(
            "INSERT INTO metadata_rules (id, rule_type, match_value, match_mode, outcome) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id_str)
        .bind(&rule_type_str)
        .bind(match_value)
        .bind(&match_mode_str)
        .bind(&outcome_str)
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                DbError::Constraint(format!(
                    "metadata rule already exists for {rule_type_str}: {match_value}"
                ))
            } else {
                DbError::Query(e.to_string())
            }
        })?;

        Self::get_by_id(pool, id).await?.ok_or(DbError::NotFound {
            entity: "metadata_rule",
            id: id_str,
        })
    }

    /// Update a metadata rule. Only non-`None` fields are updated.
    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        match_value: Option<&str>,
        match_mode: Option<MatchMode>,
        enabled: Option<bool>,
    ) -> Result<MetadataRule, DbError> {
        let id_str = id.to_string();
        let match_mode_str = match_mode.map(|m| m.to_string());
        let enabled_int = enabled.map(i64::from);

        let result = sqlx::query(
            "UPDATE metadata_rules SET
                match_value = COALESCE(?, match_value),
                match_mode = COALESCE(?, match_mode),
                enabled = COALESCE(?, enabled)
            WHERE id = ?",
        )
        .bind(match_value)
        .bind(&match_mode_str)
        .bind(enabled_int)
        .bind(&id_str)
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                DbError::Constraint("metadata rule already exists for this type and value".into())
            } else {
                DbError::Query(e.to_string())
            }
        })?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "metadata_rule",
                id: id_str,
            });
        }

        Self::get_by_id(pool, id)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "metadata_rule",
                id: id.to_string(),
            })
    }

    /// Delete a metadata rule. Returns an error if the rule is builtin.
    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();

        // Check if builtin before deleting.
        let row = sqlx::query("SELECT builtin FROM metadata_rules WHERE id = ?")
            .bind(&id_str)
            .fetch_optional(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        match row {
            None => {
                return Err(DbError::NotFound {
                    entity: "metadata_rule",
                    id: id_str,
                });
            }
            Some(ref r) => {
                let builtin: i64 = r
                    .try_get("builtin")
                    .map_err(|e| DbError::Query(e.to_string()))?;
                if builtin != 0 {
                    return Err(DbError::Constraint(
                        "cannot delete a builtin metadata rule".into(),
                    ));
                }
            }
        }

        sqlx::query("DELETE FROM metadata_rules WHERE id = ?")
            .bind(&id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }
}
