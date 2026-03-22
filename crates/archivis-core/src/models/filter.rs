use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

    /// At most one identifier filter active at a time.
    /// Enforced by API validation — if more than one is set, return 400.
    pub isbn: Option<String>,
    pub asin: Option<String>,
    pub open_library_id: Option<String>,
    pub hardcover_id: Option<String>,
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

        // Normalize identifiers
        if let Some(ref mut isbn) = self.isbn {
            // Strip hyphens and spaces from ISBN
            let clean: String = isbn
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '-')
                .collect();
            if clean.is_empty() {
                self.isbn = None;
            } else {
                *isbn = clean;
            }
        }
        if let Some(ref mut asin) = self.asin {
            let trimmed = asin.trim().to_uppercase();
            if trimmed.is_empty() {
                self.asin = None;
            } else {
                *asin = trimmed;
            }
        }
        trim_or_clear(&mut self.open_library_id);
        trim_or_clear(&mut self.hardcover_id);

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

    /// Count how many identifier filters are active (at most 1 allowed).
    pub fn active_identifier_count(&self) -> usize {
        usize::from(self.isbn.is_some())
            + usize::from(self.asin.is_some())
            + usize::from(self.open_library_id.is_some())
            + usize::from(self.hardcover_id.is_some())
    }
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
    fn canonicalize_strips_isbn_hyphens() {
        let mut f = LibraryFilterState {
            isbn: Some("978-3-16-148410-0".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.isbn.as_deref(), Some("9783161484100"));
    }

    #[test]
    fn canonicalize_uppercases_asin() {
        let mut f = LibraryFilterState {
            asin: Some("  b08n5wrwnw  ".into()),
            ..Default::default()
        };
        f.canonicalize();
        assert_eq!(f.asin.as_deref(), Some("B08N5WRWNW"));
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
    fn active_identifier_count_none() {
        let f = LibraryFilterState::default();
        assert_eq!(f.active_identifier_count(), 0);
    }

    #[test]
    fn active_identifier_count_one() {
        let f = LibraryFilterState {
            isbn: Some("123".into()),
            ..Default::default()
        };
        assert_eq!(f.active_identifier_count(), 1);
    }

    #[test]
    fn active_identifier_count_multiple() {
        let f = LibraryFilterState {
            isbn: Some("123".into()),
            asin: Some("B00".into()),
            ..Default::default()
        };
        assert_eq!(f.active_identifier_count(), 2);
    }
}
