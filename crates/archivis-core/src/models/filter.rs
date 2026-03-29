use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::isbn::{normalize_asin, normalize_isbn};

use super::enums::{BookFormat, MetadataStatus, ResolutionOutcome, ResolutionState};

/// How multiple tags should be matched in a filter query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagMatchMode {
    /// Book must have at least one of the specified tags.
    #[default]
    Any,
    /// Book must have all of the specified tags.
    All,
}

/// Normalized filter state shared by list, count, selection, and bulk ops.
///
/// Excludes view-only concerns (pagination, sort, includes, layout).
/// Lives in `archivis-core` so it is available to API, tasks, and DB crates.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryFilterState {
    pub text_query: Option<String>,

    pub author_id: Option<Uuid>,
    pub series_id: Option<Uuid>,
    pub publisher_id: Option<Uuid>,
    pub tag_ids: Vec<Uuid>,
    pub tag_match: TagMatchMode,

    pub format: Option<BookFormat>,
    pub metadata_status: Option<MetadataStatus>,
    pub resolution_state: Option<ResolutionState>,
    pub resolution_outcome: Option<ResolutionOutcome>,
    pub trusted: Option<bool>,
    pub locked: Option<bool>,
    pub language: Option<String>,
    pub year_min: Option<i32>,
    pub year_max: Option<i32>,

    pub has_cover: Option<bool>,
    pub has_description: Option<bool>,
    pub has_identifiers: Option<bool>,

    /// Generic identifier lookup.
    ///
    /// `identifier_type = None` means "search all supported identifier types".
    pub identifier_type: Option<String>,
    pub identifier_value: Option<String>,
}

impl LibraryFilterState {
    /// Normalize the filter state into canonical form.
    ///
    /// Applied before equality checks, scope-token signing, and any filter execution.
    pub fn canonicalize(&mut self) {
        // Trim `text_query`; set to `None` if empty
        if let Some(ref mut q) = self.text_query {
            let trimmed = q.trim().to_string();
            if trimmed.is_empty() {
                self.text_query = None;
            } else {
                *q = trimmed;
            }
        }

        // Trim string fields; set to `None` if empty
        trim_or_clear(&mut self.language);

        // Normalize generic identifier filter
        self.identifier_type = self.identifier_type.take().and_then(|raw| canonicalize_identifier_type(&raw));
        self.identifier_value = self
            .identifier_value
            .take()
            .and_then(|raw| canonicalize_identifier_value(self.identifier_type.as_deref(), &raw));
        if self.identifier_value.is_none() {
            self.identifier_type = None;
        }

        // Sort and dedup `tag_ids`
        self.tag_ids.sort();
        self.tag_ids.dedup();

        // Swap year bounds if inverted
        if let (Some(min), Some(max)) = (self.year_min, self.year_max) {
            if min > max {
                self.year_min = Some(max);
                self.year_max = Some(min);
            }
        }

        // Clamp year bounds to reasonable range
        if let Some(ref mut y) = self.year_min {
            *y = (*y).clamp(0, 9999);
        }
        if let Some(ref mut y) = self.year_max {
            *y = (*y).clamp(0, 9999);
        }
    }

    pub fn has_identifier_filter(&self) -> bool {
        self.identifier_value.is_some()
    }
}

pub fn canonicalize_identifier_type(raw: &str) -> Option<String> {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");

    if normalized.is_empty() {
        return None;
    }

    Some(match normalized.as_str() {
        "olid" | "open_library_id" | "openlibrary" => "open_library".into(),
        "hardcover_id" => "hardcover".into(),
        "googlebooks" => "google_books".into(),
        other => other.to_string(),
    })
}

pub fn canonicalize_identifier_value(identifier_type: Option<&str>, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    match identifier_type.and_then(canonicalize_identifier_type) {
        Some(kind) if matches!(kind.as_str(), "isbn" | "isbn10" | "isbn13") => {
            Some(normalize_isbn(trimmed))
        }
        Some(kind) if kind == "asin" => Some(normalize_asin(trimmed)),
        _ => Some(trimmed.to_string()),
    }
}

pub fn is_supported_identifier_type(raw: &str) -> bool {
    matches!(
        canonicalize_identifier_type(raw).as_deref(),
        Some("isbn" | "isbn10" | "isbn13" | "asin" | "google_books" | "open_library" | "hardcover" | "lccn")
    )
}

fn trim_or_clear(field: &mut Option<String>) {
    if let Some(ref mut s) = field {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            *field = None;
        } else {
            *s = trimmed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_trims_text_query() {
        let mut f = LibraryFilterState {
            text_query: Some("  hello world  ".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.text_query.as_deref(), Some("hello world"));
    }

    #[test]
    fn canonicalize_clears_empty_text_query() {
        let mut f = LibraryFilterState {
            text_query: Some("   ".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.text_query, None);
    }

    #[test]
    fn canonicalize_normalizes_isbn_identifier_value() {
        let mut f = LibraryFilterState {
            identifier_type: Some("isbn".into()),
            identifier_value: Some("978-3-16-148410-0".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.identifier_type.as_deref(), Some("isbn"));
        assert_eq!(f.identifier_value.as_deref(), Some("9783161484100"));
    }

    #[test]
    fn canonicalize_uppercases_asin_identifier_value() {
        let mut f = LibraryFilterState {
            identifier_type: Some("asin".into()),
            identifier_value: Some("  b08n5wrwnw  ".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.identifier_type.as_deref(), Some("asin"));
        assert_eq!(f.identifier_value.as_deref(), Some("B08N5WRWNW"));
    }

    #[test]
    fn canonicalize_normalizes_identifier_type_aliases() {
        let mut f = LibraryFilterState {
            identifier_type: Some("open_library_id".into()),
            identifier_value: Some("OL123W".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.identifier_type.as_deref(), Some("open_library"));
    }

    #[test]
    fn canonicalize_clears_identifier_type_when_value_missing() {
        let mut f = LibraryFilterState {
            identifier_type: Some("isbn".into()),
            identifier_value: Some("   ".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.identifier_type, None);
        assert_eq!(f.identifier_value, None);
    }

    #[test]
    fn canonicalize_swaps_inverted_years() {
        let mut f = LibraryFilterState {
            year_min: Some(2020),
            year_max: Some(2010),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.year_min, Some(2010));
        assert_eq!(f.year_max, Some(2020));
    }

    #[test]
    fn canonicalize_clamps_year_bounds() {
        let mut f = LibraryFilterState {
            year_min: Some(-5),
            year_max: Some(99999),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.year_min, Some(0));
        assert_eq!(f.year_max, Some(9999));
    }

    #[test]
    fn canonicalize_dedup_tag_ids() {
        let id = Uuid::new_v4();
        let mut f = LibraryFilterState {
            tag_ids: vec![id, id],
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.tag_ids, vec![id]);
    }

    #[test]
    fn has_identifier_filter_none() {
        let f = LibraryFilterState::default();
        assert!(!f.has_identifier_filter());
    }

    #[test]
    fn has_identifier_filter_when_value_set() {
        let f = LibraryFilterState {
            identifier_value: Some("123".into()),
            ..Default::default()
        };
        assert!(f.has_identifier_filter());
    }
}
