use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use tracing::debug;

use crate::ParsedFilename;

/// File extensions to strip before parsing.
const KNOWN_EXTENSIONS: &[&str] = &[
    ".epub", ".pdf", ".mobi", ".azw3", ".cbz", ".fb2", ".txt", ".djvu",
];

/// Minimum plausible publication year.
const MIN_YEAR: u16 = 1000;

/// Maximum plausible publication year.
const MAX_YEAR: u16 = 2100;

// ---------------------------------------------------------------------------
// Compiled regex patterns (lazily initialised, thread-safe)
// ---------------------------------------------------------------------------

/// `Author - Title (Year)`
static RE_AUTHOR_TITLE_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?)\s+-\s+(.+?)\s*\((\d{4})\)\s*$").expect("valid regex"));

/// `Author - Title [Series #N]`
static RE_AUTHOR_TITLE_SERIES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+?)\s+-\s+(.+?)\s*\[(.+?)(?:\s*#\s*(\d+(?:\.\d+)?))?\]\s*$")
        .expect("valid regex")
});

/// `Author - Title`
static RE_AUTHOR_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?)\s+-\s+(.+?)\s*$").expect("valid regex"));

/// `Title [Series #N]`
static RE_TITLE_SERIES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+?)\s*\[(.+?)(?:\s*#\s*(\d+(?:\.\d+)?))?\]\s*$").expect("valid regex")
});

/// `Title (Year)`
static RE_TITLE_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?)\s*\((\d{4})\)\s*$").expect("valid regex"));

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a filename (without directory path) into metadata components.
///
/// Strips known ebook extensions and attempts a sequence of regex patterns,
/// returning the first successful match.
pub fn parse_filename(filename: &str) -> ParsedFilename {
    let stem = strip_extension(filename);
    let stem = normalise(&stem);

    let result = try_author_title_year(&stem)
        .or_else(|| try_author_title_series(&stem))
        .or_else(|| try_author_title(&stem))
        .or_else(|| try_title_series(&stem))
        .or_else(|| try_title_year(&stem))
        .unwrap_or_else(|| title_only(&stem));

    debug!(
        filename,
        ?result,
        score = result.completeness_score(),
        "parsed filename"
    );

    result
}

/// Parse a full file path, extracting signals from both the directory
/// structure and the filename.
///
/// Directory components are only used to fill in fields that the filename
/// itself does not provide.
pub fn parse_path(path: &Path) -> ParsedFilename {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    let mut result = parse_filename(filename);

    // Whether the filename parse produced only a bare title (no structured
    // metadata). In that case, the parent directory name is a better title
    // candidate than the raw file stem.
    let filename_is_title_only =
        result.author.is_none() && result.series.is_none() && result.year.is_none();

    // Attempt to fill gaps from the directory hierarchy.
    let components: Vec<&str> = path
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| {
                    if let std::path::Component::Normal(s) = c {
                        s.to_str()
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    match components.len() {
        0 => {}
        1 => {
            // `Author/file.ext`
            if result.author.is_none() {
                result.author = clean_value(&normalise(components[0]));
            }
        }
        _ => {
            // `…/Author/Title/file.ext` — use the last two directory components.
            let grandparent = components[components.len() - 2];
            let parent = components[components.len() - 1];

            if result.author.is_none() {
                result.author = clean_value(&normalise(grandparent));
            }

            let parent_clean = normalise(parent);
            let stem = strip_extension(filename);
            let stem_clean = normalise(&stem);

            if result.title.is_none() {
                // Use parent directory as title if it differs from the stem.
                if stem_clean != parent_clean {
                    result.title = clean_value(&parent_clean);
                }
            } else if filename_is_title_only && stem_clean != parent_clean {
                // The filename was just a bare stem (e.g. `book.epub`) with no
                // structured metadata. The parent directory (e.g. `Dune/`) is
                // more likely the real title.
                result.title = clean_value(&parent_clean);
            }
        }
    }

    debug!(
        path = %path.display(),
        ?result,
        score = result.completeness_score(),
        "parsed path"
    );

    result
}

// ---------------------------------------------------------------------------
// Pattern matchers
// ---------------------------------------------------------------------------

fn try_author_title_year(stem: &str) -> Option<ParsedFilename> {
    let caps = RE_AUTHOR_TITLE_YEAR.captures(stem)?;
    Some(ParsedFilename {
        author: clean_value(caps.get(1)?.as_str()),
        title: clean_value(caps.get(2)?.as_str()),
        year: parse_year(caps.get(3)?.as_str()),
        ..Default::default()
    })
}

fn try_author_title_series(stem: &str) -> Option<ParsedFilename> {
    let caps = RE_AUTHOR_TITLE_SERIES.captures(stem)?;
    Some(ParsedFilename {
        author: clean_value(caps.get(1)?.as_str()),
        title: clean_value(caps.get(2)?.as_str()),
        series: clean_value(caps.get(3)?.as_str()),
        series_position: caps.get(4).and_then(|m| m.as_str().parse::<f32>().ok()),
        ..Default::default()
    })
}

fn try_author_title(stem: &str) -> Option<ParsedFilename> {
    let caps = RE_AUTHOR_TITLE.captures(stem)?;
    Some(ParsedFilename {
        author: clean_value(caps.get(1)?.as_str()),
        title: clean_value(caps.get(2)?.as_str()),
        ..Default::default()
    })
}

fn try_title_series(stem: &str) -> Option<ParsedFilename> {
    let caps = RE_TITLE_SERIES.captures(stem)?;
    Some(ParsedFilename {
        title: clean_value(caps.get(1)?.as_str()),
        series: clean_value(caps.get(2)?.as_str()),
        series_position: caps.get(3).and_then(|m| m.as_str().parse::<f32>().ok()),
        ..Default::default()
    })
}

fn try_title_year(stem: &str) -> Option<ParsedFilename> {
    let caps = RE_TITLE_YEAR.captures(stem)?;
    Some(ParsedFilename {
        title: clean_value(caps.get(1)?.as_str()),
        year: parse_year(caps.get(2)?.as_str()),
        ..Default::default()
    })
}

fn title_only(stem: &str) -> ParsedFilename {
    ParsedFilename {
        title: clean_value(stem),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip a known ebook extension from the filename, case-insensitively.
fn strip_extension(filename: &str) -> String {
    let lower = filename.to_lowercase();
    for ext in KNOWN_EXTENSIONS {
        if lower.ends_with(ext) {
            return filename[..filename.len() - ext.len()].to_string();
        }
    }
    filename.to_string()
}

/// Replace underscores with spaces.
fn normalise(s: &str) -> String {
    s.replace('_', " ")
}

/// Trim whitespace and return `None` for empty strings.
fn clean_value(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse a 4-digit string as a year, returning `None` if out of range.
fn parse_year(s: &str) -> Option<u16> {
    let year: u16 = s.parse().ok()?;
    if (MIN_YEAR..=MAX_YEAR).contains(&year) {
        Some(year)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Helper to compare a `ParsedFilename` against expected values.
    fn assert_parsed(input: &str, result: &ParsedFilename, expected: &ParsedFilename) {
        assert_eq!(result.title, expected.title, "title mismatch for '{input}'");
        assert_eq!(
            result.author, expected.author,
            "author mismatch for '{input}'"
        );
        assert_eq!(
            result.series, expected.series,
            "series mismatch for '{input}'"
        );
        assert_eq!(
            result.series_position, expected.series_position,
            "series_position mismatch for '{input}'"
        );
        assert_eq!(result.year, expected.year, "year mismatch for '{input}'");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn filename_parsing_table() {
        let cases: Vec<(&str, ParsedFilename)> = vec![
            // Pattern 1: Author - Title (Year)
            (
                "Frank Herbert - Dune (1965).epub",
                ParsedFilename {
                    author: Some("Frank Herbert".into()),
                    title: Some("Dune".into()),
                    year: Some(1965),
                    ..Default::default()
                },
            ),
            // Pattern 2: Author - Title [Series #N]
            (
                "Frank Herbert - Dune [Dune Chronicles #1].epub",
                ParsedFilename {
                    author: Some("Frank Herbert".into()),
                    title: Some("Dune".into()),
                    series: Some("Dune Chronicles".into()),
                    series_position: Some(1.0),
                    ..Default::default()
                },
            ),
            // Pattern 3: Author - Title
            (
                "Frank Herbert - Dune.epub",
                ParsedFilename {
                    author: Some("Frank Herbert".into()),
                    title: Some("Dune".into()),
                    ..Default::default()
                },
            ),
            // Pattern 4: Title [Series #N]
            (
                "Dune [Dune Chronicles #1].epub",
                ParsedFilename {
                    title: Some("Dune".into()),
                    series: Some("Dune Chronicles".into()),
                    series_position: Some(1.0),
                    ..Default::default()
                },
            ),
            // Pattern 5: Title (Year)
            (
                "Dune (1965).epub",
                ParsedFilename {
                    title: Some("Dune".into()),
                    year: Some(1965),
                    ..Default::default()
                },
            ),
            // Pattern 6: Title only
            (
                "Dune.epub",
                ParsedFilename {
                    title: Some("Dune".into()),
                    ..Default::default()
                },
            ),
            // Underscores replaced with spaces
            (
                "Frank_Herbert_-_Dune.epub",
                ParsedFilename {
                    author: Some("Frank Herbert".into()),
                    title: Some("Dune".into()),
                    ..Default::default()
                },
            ),
            // Hyphenated author name
            (
                "Jean-Paul Sartre - Being and Nothingness.epub",
                ParsedFilename {
                    author: Some("Jean-Paul Sartre".into()),
                    title: Some("Being and Nothingness".into()),
                    ..Default::default()
                },
            ),
            // Series with no position number
            (
                "Dune [Dune Chronicles].epub",
                ParsedFilename {
                    title: Some("Dune".into()),
                    series: Some("Dune Chronicles".into()),
                    series_position: None,
                    ..Default::default()
                },
            ),
            // Fractional series position
            (
                "Legends [Dune Chronicles #1.5].epub",
                ParsedFilename {
                    title: Some("Legends".into()),
                    series: Some("Dune Chronicles".into()),
                    series_position: Some(1.5),
                    ..Default::default()
                },
            ),
            // PDF extension
            (
                "Isaac Asimov - Foundation (1951).pdf",
                ParsedFilename {
                    author: Some("Isaac Asimov".into()),
                    title: Some("Foundation".into()),
                    year: Some(1951),
                    ..Default::default()
                },
            ),
            // MOBI extension
            (
                "Neuromancer.mobi",
                ParsedFilename {
                    title: Some("Neuromancer".into()),
                    ..Default::default()
                },
            ),
            // No extension at all
            (
                "Frank Herbert - Dune",
                ParsedFilename {
                    author: Some("Frank Herbert".into()),
                    title: Some("Dune".into()),
                    ..Default::default()
                },
            ),
            // Author - Title [Series #N] with year-like content doesn't confuse
            (
                "Brandon Sanderson - The Way of Kings [The Stormlight Archive #1].epub",
                ParsedFilename {
                    author: Some("Brandon Sanderson".into()),
                    title: Some("The Way of Kings".into()),
                    series: Some("The Stormlight Archive".into()),
                    series_position: Some(1.0),
                    ..Default::default()
                },
            ),
        ];

        for (input, expected) in &cases {
            let result = parse_filename(input);
            assert_parsed(input, &result, expected);
        }
    }

    #[test]
    fn year_validation() {
        // Valid year
        let result = parse_filename("Dune (1965).epub");
        assert_eq!(result.year, Some(1965));

        // Year below minimum
        let result = parse_filename("Dune (0500).epub");
        assert_eq!(result.year, None);
        // Falls back to title-only since year is invalid — the regex still
        // matches but year gets filtered out.
        assert_eq!(result.title, Some("Dune".into()));

        // Year above maximum
        let result = parse_filename("Dune (2200).epub");
        assert_eq!(result.year, None);
    }

    #[test]
    fn path_with_author_directory() {
        let path = PathBuf::from("Frank Herbert/Dune.epub");
        let result = parse_path(&path);
        assert_eq!(result.title, Some("Dune".into()));
        assert_eq!(result.author, Some("Frank Herbert".into()));
    }

    #[test]
    fn path_with_author_and_title_directories() {
        let path = PathBuf::from("Frank Herbert/Dune/Dune.epub");
        let result = parse_path(&path);
        assert_eq!(result.title, Some("Dune".into()));
        assert_eq!(result.author, Some("Frank Herbert".into()));
    }

    #[test]
    fn path_directory_does_not_override_filename_info() {
        let path = PathBuf::from("Some Author/Some Title/Isaac Asimov - Foundation (1951).epub");
        let result = parse_path(&path);
        // Filename takes priority: author and title come from the filename.
        assert_eq!(result.author, Some("Isaac Asimov".into()));
        assert_eq!(result.title, Some("Foundation".into()));
        assert_eq!(result.year, Some(1951));
    }

    #[test]
    fn path_fills_author_from_directory() {
        // Filename only has title, directory provides author.
        let path = PathBuf::from("Frank Herbert/Dune (1965).epub");
        let result = parse_path(&path);
        assert_eq!(result.title, Some("Dune".into()));
        assert_eq!(result.year, Some(1965));
        assert_eq!(result.author, Some("Frank Herbert".into()));
    }

    #[test]
    fn completeness_score_full() {
        let parsed = ParsedFilename {
            title: Some("Dune".into()),
            author: Some("Frank Herbert".into()),
            series: Some("Dune Chronicles".into()),
            series_position: Some(1.0),
            year: Some(1965),
        };
        // 0.4 (title) + 0.3 (author) + 0.15 (series) + 0.15 (year) = 1.0
        let score = parsed.completeness_score();
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "expected 1.0, got {score}"
        );
    }

    #[test]
    fn completeness_score_title_only() {
        let parsed = ParsedFilename {
            title: Some("Dune".into()),
            ..Default::default()
        };
        let score = parsed.completeness_score();
        assert!(
            (score - 0.4).abs() < f32::EPSILON,
            "expected 0.4, got {score}"
        );
    }

    #[test]
    fn completeness_score_empty() {
        let parsed = ParsedFilename::default();
        assert!((parsed.completeness_score()).abs() < f32::EPSILON);
    }

    #[test]
    fn strip_various_extensions() {
        for ext in &[
            ".epub", ".pdf", ".mobi", ".azw3", ".cbz", ".fb2", ".txt", ".djvu",
        ] {
            let filename = format!("MyBook{ext}");
            let result = parse_filename(&filename);
            assert_eq!(
                result.title,
                Some("MyBook".into()),
                "failed to strip extension {ext}"
            );
        }
    }

    #[test]
    fn unicode_filename() {
        let result = parse_filename("Лев Толстой - Война и мир (1869).epub");
        assert_eq!(result.author, Some("Лев Толстой".into()));
        assert_eq!(result.title, Some("Война и мир".into()));
        assert_eq!(result.year, Some(1869));
    }

    #[test]
    fn deeply_nested_path_uses_last_two_components() {
        let path = PathBuf::from("library/fiction/Frank Herbert/Dune/book.epub");
        let result = parse_path(&path);
        assert_eq!(result.author, Some("Frank Herbert".into()));
        assert_eq!(result.title, Some("Dune".into()));
    }
}
