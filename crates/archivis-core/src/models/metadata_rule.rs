use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A metadata policy rule that controls import/resolution behavior.
///
/// Rules match on a `rule_type` + `match_value` pair and produce an `outcome`
/// when matched. For example, a publisher rule with outcome `TrustMetadata`
/// marks books from that publisher as `Identified` at import time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataRule {
    pub id: Uuid,
    pub rule_type: MetadataRuleType,
    pub match_value: String,
    pub match_mode: MatchMode,
    pub outcome: RuleOutcome,
    pub enabled: bool,
    pub builtin: bool,
    pub created_at: DateTime<Utc>,
}

/// Check if any rule in the slice trusts the given publisher name.
pub fn is_trusted_publisher(rules: &[MetadataRule], publisher: &str) -> bool {
    rules.iter().any(|rule| {
        rule.rule_type == MetadataRuleType::Publisher
            && rule.outcome == RuleOutcome::TrustMetadata
            && rule.matches_publisher(publisher)
    })
}

impl MetadataRule {
    /// Check if a publisher name matches this rule (assuming `rule_type == Publisher`).
    ///
    /// Returns `false` if the rule is disabled or `rule_type` is not `Publisher`.
    pub fn matches_publisher(&self, publisher: &str) -> bool {
        if !self.enabled || self.rule_type != MetadataRuleType::Publisher {
            return false;
        }
        match self.match_mode {
            MatchMode::Exact => publisher.eq_ignore_ascii_case(&self.match_value),
            MatchMode::Contains => {
                let publisher_lower = publisher.to_lowercase();
                let match_lower = self.match_value.to_lowercase();
                publisher_lower.contains(&match_lower)
            }
        }
    }
}

/// What kind of metadata field the rule matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataRuleType {
    Publisher,
}

impl fmt::Display for MetadataRuleType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Publisher => write!(f, "publisher"),
        }
    }
}

impl FromStr for MetadataRuleType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "publisher" => Ok(Self::Publisher),
            _ => Err(format!("unknown metadata rule type: {s}")),
        }
    }
}

/// How the `match_value` is compared against the book's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// The value must match exactly (case-insensitive).
    Exact,
    /// The value must appear as a substring (case-insensitive).
    Contains,
}

impl fmt::Display for MatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => write!(f, "exact"),
            Self::Contains => write!(f, "contains"),
        }
    }
}

impl FromStr for MatchMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "exact" => Ok(Self::Exact),
            "contains" => Ok(Self::Contains),
            _ => Err(format!("unknown match mode: {s}")),
        }
    }
}

/// What action to take when a rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOutcome {
    /// Trust the embedded metadata: mark as `Identified` at import, skip
    /// provider resolution.
    TrustMetadata,
}

impl fmt::Display for RuleOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrustMetadata => write!(f, "trust_metadata"),
        }
    }
}

impl FromStr for RuleOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "trust_metadata" => Ok(Self::TrustMetadata),
            _ => Err(format!("unknown rule outcome: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MetadataRuleType ────────────────────────────────────────────

    #[test]
    fn rule_type_display() {
        assert_eq!(MetadataRuleType::Publisher.to_string(), "publisher");
    }

    #[test]
    fn rule_type_from_str() {
        assert_eq!(
            "publisher".parse::<MetadataRuleType>().unwrap(),
            MetadataRuleType::Publisher,
        );
        assert_eq!(
            "Publisher".parse::<MetadataRuleType>().unwrap(),
            MetadataRuleType::Publisher,
        );
        assert!("unknown".parse::<MetadataRuleType>().is_err());
    }

    #[test]
    fn rule_type_serde_roundtrip() {
        let rt = MetadataRuleType::Publisher;
        let json = serde_json::to_string(&rt).unwrap();
        assert_eq!(json, r#""publisher""#);
        let deserialized: MetadataRuleType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, rt);
    }

    // ── MatchMode ───────────────────────────────────────────────────

    #[test]
    fn match_mode_display() {
        assert_eq!(MatchMode::Exact.to_string(), "exact");
        assert_eq!(MatchMode::Contains.to_string(), "contains");
    }

    #[test]
    fn match_mode_from_str() {
        assert_eq!("exact".parse::<MatchMode>().unwrap(), MatchMode::Exact);
        assert_eq!(
            "contains".parse::<MatchMode>().unwrap(),
            MatchMode::Contains
        );
        assert_eq!("EXACT".parse::<MatchMode>().unwrap(), MatchMode::Exact);
        assert!("prefix".parse::<MatchMode>().is_err());
    }

    #[test]
    fn match_mode_serde_roundtrip() {
        let mode = MatchMode::Contains;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#""contains""#);
        let deserialized: MatchMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, mode);
    }

    // ── RuleOutcome ─────────────────────────────────────────────────

    #[test]
    fn outcome_display() {
        assert_eq!(RuleOutcome::TrustMetadata.to_string(), "trust_metadata");
    }

    #[test]
    fn outcome_from_str() {
        assert_eq!(
            "trust_metadata".parse::<RuleOutcome>().unwrap(),
            RuleOutcome::TrustMetadata,
        );
        assert!("skip_resolution".parse::<RuleOutcome>().is_err());
    }

    #[test]
    fn outcome_serde_roundtrip() {
        let outcome = RuleOutcome::TrustMetadata;
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(json, r#""trust_metadata""#);
        let deserialized: RuleOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, outcome);
    }

    // ── MetadataRule matching ───────────────────────────────────────

    fn make_rule(match_value: &str, match_mode: MatchMode) -> MetadataRule {
        MetadataRule {
            id: Uuid::new_v4(),
            rule_type: MetadataRuleType::Publisher,
            match_value: match_value.into(),
            match_mode,
            outcome: RuleOutcome::TrustMetadata,
            enabled: true,
            builtin: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn matches_publisher_exact() {
        let rule = make_rule("Standard Ebooks", MatchMode::Exact);
        assert!(rule.matches_publisher("Standard Ebooks"));
        assert!(rule.matches_publisher("standard ebooks")); // case-insensitive
        assert!(!rule.matches_publisher("Standard Ebooks Press"));
    }

    #[test]
    fn matches_publisher_contains() {
        let rule = make_rule("Project Gutenberg", MatchMode::Contains);
        assert!(rule.matches_publisher("Project Gutenberg"));
        assert!(rule.matches_publisher("Project Gutenberg Literary Archive Foundation"));
        assert!(rule.matches_publisher("project gutenberg")); // case-insensitive
        assert!(!rule.matches_publisher("Gutenberg Press"));
    }

    #[test]
    fn disabled_rule_does_not_match() {
        let mut rule = make_rule("Standard Ebooks", MatchMode::Exact);
        rule.enabled = false;
        assert!(!rule.matches_publisher("Standard Ebooks"));
    }

    // ── is_trusted_publisher ──────────────────────────────────────

    #[test]
    fn is_trusted_publisher_matches() {
        let rules = vec![make_rule("Standard Ebooks", MatchMode::Exact)];
        assert!(is_trusted_publisher(&rules, "Standard Ebooks"));
        assert!(is_trusted_publisher(&rules, "standard ebooks"));
        assert!(!is_trusted_publisher(&rules, "Other Publisher"));
    }

    #[test]
    fn is_trusted_publisher_empty_rules() {
        assert!(!is_trusted_publisher(&[], "Standard Ebooks"));
    }

    #[test]
    fn is_trusted_publisher_skips_disabled() {
        let mut rule = make_rule("Standard Ebooks", MatchMode::Exact);
        rule.enabled = false;
        assert!(!is_trusted_publisher(&[rule], "Standard Ebooks"));
    }
}
