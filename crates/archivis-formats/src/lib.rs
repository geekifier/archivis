pub mod authors;
pub mod content_text;
pub mod detect;
pub mod epub;
pub mod filename;
pub mod isbn_scan;
pub mod mobi;
pub mod pdf;
pub mod sanitize;
pub mod scoring;
pub mod similarity;

use archivis_core::models::{IdentifierType, MetadataSource};

/// Parse a year from a date-like string (e.g. "2023-01-15", "2023", "D:20230601").
pub(crate) fn parse_year_from_date_str(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.len() >= 4 && s[..4].chars().all(|c| c.is_ascii_digit()) {
        s[..4].parse::<i32>().ok()
    } else {
        None
    }
}

/// Extracted metadata from an ebook file.
///
/// Shared across all format extractors. Fields are optional because
/// not every format provides every piece of metadata.
#[derive(Debug, Clone)]
pub struct ExtractedMetadata {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub publication_year: Option<i32>,
    pub identifiers: Vec<ExtractedIdentifier>,
    /// Tags or categories found in the file metadata.
    pub subjects: Vec<String>,
    pub series: Option<String>,
    pub series_position: Option<f32>,
    pub page_count: Option<i32>,
    /// Format specification version (e.g., "3.0" for EPUB 3.0, "1.7" for PDF 1.7).
    pub format_version: Option<String>,
    pub cover_image: Option<CoverData>,
    pub source: MetadataSource,
}

impl Default for ExtractedMetadata {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            authors: Vec::new(),
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: Vec::new(),
            subjects: Vec::new(),
            series: None,
            series_position: None,
            page_count: None,
            format_version: None,
            cover_image: None,
            source: MetadataSource::Embedded,
        }
    }
}

/// An identifier extracted from file metadata (ISBN, ASIN, etc.).
#[derive(Debug, Clone)]
pub struct ExtractedIdentifier {
    pub identifier_type: IdentifierType,
    pub value: String,
}

/// Raw cover image data with its media type.
#[derive(Debug, Clone)]
pub struct CoverData {
    pub bytes: Vec<u8>,
    pub media_type: String,
}

/// Result from parsing a filename into likely metadata fields.
#[derive(Debug, Clone, Default)]
pub struct ParsedFilename {
    pub title: Option<String>,
    pub author: Option<String>,
    pub series: Option<String>,
    pub series_position: Option<f32>,
    pub year: Option<u16>,
}

impl ParsedFilename {
    /// Score the completeness of this parsed result (0.0 – 1.0).
    pub fn completeness_score(&self) -> f32 {
        let mut score = 0.0_f32;
        if self.title.is_some() {
            score += 0.4;
        }
        if self.author.is_some() {
            score += 0.3;
        }
        if self.series.is_some() {
            score += 0.15;
        }
        if self.year.is_some() {
            score += 0.15;
        }
        score
    }
}
