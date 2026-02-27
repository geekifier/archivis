use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Supported ebook file formats, detected via magic bytes rather than extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookFormat {
    Epub,
    Pdf,
    Mobi,
    Cbz,
    Fb2,
    Txt,
    Djvu,
    Azw3,
    Unknown,
}

impl BookFormat {
    /// Common file extension for this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Mobi => "mobi",
            Self::Cbz => "cbz",
            Self::Fb2 => "fb2",
            Self::Txt => "txt",
            Self::Djvu => "djvu",
            Self::Azw3 => "azw3",
            Self::Unknown => "bin",
        }
    }

    /// MIME type for this format.
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Epub => "application/epub+zip",
            Self::Pdf => "application/pdf",
            Self::Mobi => "application/x-mobipocket-ebook",
            Self::Cbz => "application/vnd.comicbook+zip",
            Self::Fb2 => "application/x-fictionbook+xml",
            Self::Txt => "text/plain",
            Self::Djvu => "image/vnd.djvu",
            Self::Azw3 => "application/vnd.amazon.ebook",
            Self::Unknown => "application/octet-stream",
        }
    }
}

impl fmt::Display for BookFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Epub => write!(f, "EPUB"),
            Self::Pdf => write!(f, "PDF"),
            Self::Mobi => write!(f, "MOBI"),
            Self::Cbz => write!(f, "CBZ"),
            Self::Fb2 => write!(f, "FB2"),
            Self::Txt => write!(f, "TXT"),
            Self::Djvu => write!(f, "DJVU"),
            Self::Azw3 => write!(f, "AZW3"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl FromStr for BookFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "epub" => Ok(Self::Epub),
            "pdf" => Ok(Self::Pdf),
            "mobi" => Ok(Self::Mobi),
            "cbz" => Ok(Self::Cbz),
            "fb2" => Ok(Self::Fb2),
            "txt" => Ok(Self::Txt),
            "djvu" => Ok(Self::Djvu),
            "azw3" => Ok(Self::Azw3),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!("unknown book format: {s}")),
        }
    }
}

/// Indicates how well a book's metadata has been identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataStatus {
    /// Metadata confidently matched from a reliable source.
    Identified,
    /// Partial metadata found but needs human review.
    NeedsReview,
    /// No useful metadata could be extracted or matched.
    Unidentified,
}

impl fmt::Display for MetadataStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identified => write!(f, "Identified"),
            Self::NeedsReview => write!(f, "Needs Review"),
            Self::Unidentified => write!(f, "Unidentified"),
        }
    }
}

impl FromStr for MetadataStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "identified" => Ok(Self::Identified),
            "needs_review" => Ok(Self::NeedsReview),
            "unidentified" => Ok(Self::Unidentified),
            _ => Err(format!("unknown metadata status: {s}")),
        }
    }
}

/// Types of external identifiers that can be associated with a book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierType {
    Isbn13,
    Isbn10,
    Asin,
    GoogleBooks,
    OpenLibrary,
    Hardcover,
}

impl fmt::Display for IdentifierType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Isbn13 => write!(f, "ISBN-13"),
            Self::Isbn10 => write!(f, "ISBN-10"),
            Self::Asin => write!(f, "ASIN"),
            Self::GoogleBooks => write!(f, "Google Books"),
            Self::OpenLibrary => write!(f, "Open Library"),
            Self::Hardcover => write!(f, "Hardcover"),
        }
    }
}

impl FromStr for IdentifierType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "isbn13" | "isbn_13" => Ok(Self::Isbn13),
            "isbn10" | "isbn_10" => Ok(Self::Isbn10),
            "asin" => Ok(Self::Asin),
            "google_books" | "googlebooks" => Ok(Self::GoogleBooks),
            "open_library" | "openlibrary" => Ok(Self::OpenLibrary),
            "hardcover" => Ok(Self::Hardcover),
            _ => Err(format!("unknown identifier type: {s}")),
        }
    }
}

/// Controls how strictly metadata quality is scored during import.
///
/// Different profiles suit different use cases:
/// - `Strict`: ISBN-centric — best for library cleanup where provenance matters.
/// - `Balanced`: allows rich embedded metadata to reach "Identified" without ISBN.
/// - `Permissive`: trusts embedded metadata more, suitable for well-tagged collections.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoringProfile {
    Strict,
    #[default]
    Balanced,
    Permissive,
}

impl fmt::Display for ScoringProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strict => write!(f, "Strict"),
            Self::Balanced => write!(f, "Balanced"),
            Self::Permissive => write!(f, "Permissive"),
        }
    }
}

impl FromStr for ScoringProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "balanced" => Ok(Self::Balanced),
            "permissive" => Ok(Self::Permissive),
            _ => Err(format!("unknown scoring profile: {s}")),
        }
    }
}

/// Tracks where a piece of metadata originated, for conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "name", rename_all = "snake_case")]
pub enum MetadataSource {
    /// Extracted from the ebook file's embedded metadata (OPF, PDF info dict, etc.).
    Embedded,
    /// Parsed from the filename or directory structure.
    Filename,
    /// Retrieved from an external metadata provider.
    Provider(String),
    /// Manually entered or edited by a user.
    User,
    /// Found by scanning the book's content (e.g. ISBN detection via OCR or text extraction).
    ContentScan,
}

impl fmt::Display for MetadataSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Embedded => write!(f, "Embedded"),
            Self::Filename => write!(f, "Filename"),
            Self::Provider(name) => write!(f, "Provider: {name}"),
            Self::User => write!(f, "User"),
            Self::ContentScan => write!(f, "Content Scan"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── BookFormat ────────────────────────────────────────────────

    #[test]
    fn book_format_display() {
        assert_eq!(BookFormat::Epub.to_string(), "EPUB");
        assert_eq!(BookFormat::Pdf.to_string(), "PDF");
        assert_eq!(BookFormat::Mobi.to_string(), "MOBI");
        assert_eq!(BookFormat::Cbz.to_string(), "CBZ");
        assert_eq!(BookFormat::Fb2.to_string(), "FB2");
        assert_eq!(BookFormat::Txt.to_string(), "TXT");
        assert_eq!(BookFormat::Djvu.to_string(), "DJVU");
        assert_eq!(BookFormat::Azw3.to_string(), "AZW3");
        assert_eq!(BookFormat::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn book_format_from_str() {
        assert_eq!("epub".parse::<BookFormat>().unwrap(), BookFormat::Epub);
        assert_eq!("PDF".parse::<BookFormat>().unwrap(), BookFormat::Pdf);
        assert_eq!("Azw3".parse::<BookFormat>().unwrap(), BookFormat::Azw3);
        assert!("docx".parse::<BookFormat>().is_err());
    }

    #[test]
    fn book_format_serde_roundtrip() {
        let format = BookFormat::Epub;
        let json = serde_json::to_string(&format).unwrap();
        assert_eq!(json, r#""epub""#);
        let deserialized: BookFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, format);
    }

    #[test]
    fn book_format_extension_and_mime() {
        assert_eq!(BookFormat::Epub.extension(), "epub");
        assert_eq!(BookFormat::Epub.mime_type(), "application/epub+zip");
        assert_eq!(BookFormat::Pdf.extension(), "pdf");
        assert_eq!(BookFormat::Pdf.mime_type(), "application/pdf");
    }

    // ── MetadataStatus ───────────────────────────────────────────

    #[test]
    fn metadata_status_display() {
        assert_eq!(MetadataStatus::Identified.to_string(), "Identified");
        assert_eq!(MetadataStatus::NeedsReview.to_string(), "Needs Review");
        assert_eq!(MetadataStatus::Unidentified.to_string(), "Unidentified");
    }

    #[test]
    fn metadata_status_from_str() {
        assert_eq!(
            "identified".parse::<MetadataStatus>().unwrap(),
            MetadataStatus::Identified,
        );
        assert_eq!(
            "needs_review".parse::<MetadataStatus>().unwrap(),
            MetadataStatus::NeedsReview,
        );
        assert_eq!(
            "needs-review".parse::<MetadataStatus>().unwrap(),
            MetadataStatus::NeedsReview,
        );
        assert!("bogus".parse::<MetadataStatus>().is_err());
    }

    #[test]
    fn metadata_status_serde_roundtrip() {
        let status = MetadataStatus::NeedsReview;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""needs_review""#);
        let deserialized: MetadataStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }

    // ── IdentifierType ───────────────────────────────────────────

    #[test]
    fn identifier_type_display() {
        assert_eq!(IdentifierType::Isbn13.to_string(), "ISBN-13");
        assert_eq!(IdentifierType::Isbn10.to_string(), "ISBN-10");
        assert_eq!(IdentifierType::Asin.to_string(), "ASIN");
        assert_eq!(IdentifierType::GoogleBooks.to_string(), "Google Books");
        assert_eq!(IdentifierType::OpenLibrary.to_string(), "Open Library");
        assert_eq!(IdentifierType::Hardcover.to_string(), "Hardcover");
    }

    #[test]
    fn identifier_type_from_str() {
        assert_eq!(
            "isbn13".parse::<IdentifierType>().unwrap(),
            IdentifierType::Isbn13,
        );
        assert_eq!(
            "isbn-13".parse::<IdentifierType>().unwrap(),
            IdentifierType::Isbn13,
        );
        assert_eq!(
            "isbn_13".parse::<IdentifierType>().unwrap(),
            IdentifierType::Isbn13,
        );
        assert_eq!(
            "google_books".parse::<IdentifierType>().unwrap(),
            IdentifierType::GoogleBooks,
        );
        assert!("doi".parse::<IdentifierType>().is_err());
    }

    #[test]
    fn identifier_type_serde_roundtrip() {
        let id_type = IdentifierType::Isbn13;
        let json = serde_json::to_string(&id_type).unwrap();
        assert_eq!(json, r#""isbn13""#);
        let deserialized: IdentifierType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, id_type);
    }

    // ── MetadataSource ───────────────────────────────────────────

    #[test]
    fn metadata_source_display() {
        assert_eq!(MetadataSource::Embedded.to_string(), "Embedded");
        assert_eq!(MetadataSource::Filename.to_string(), "Filename");
        assert_eq!(
            MetadataSource::Provider("Hardcover".into()).to_string(),
            "Provider: Hardcover",
        );
        assert_eq!(MetadataSource::User.to_string(), "User");
        assert_eq!(MetadataSource::ContentScan.to_string(), "Content Scan");
    }

    #[test]
    fn metadata_source_serde_unit_variant() {
        let source = MetadataSource::Embedded;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, r#"{"type":"embedded"}"#);
        let deserialized: MetadataSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, source);
    }

    #[test]
    fn metadata_source_serde_provider_variant() {
        let source = MetadataSource::Provider("Hardcover".into());
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, r#"{"type":"provider","name":"Hardcover"}"#);
        let deserialized: MetadataSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, source);
    }

    #[test]
    fn metadata_source_serde_content_scan_variant() {
        let source = MetadataSource::ContentScan;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, r#"{"type":"content_scan"}"#);
        let deserialized: MetadataSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, source);
    }

    // ── ScoringProfile ──────────────────────────────────────────────

    #[test]
    fn scoring_profile_default_is_balanced() {
        assert_eq!(ScoringProfile::default(), ScoringProfile::Balanced);
    }

    #[test]
    fn scoring_profile_display() {
        assert_eq!(ScoringProfile::Strict.to_string(), "Strict");
        assert_eq!(ScoringProfile::Balanced.to_string(), "Balanced");
        assert_eq!(ScoringProfile::Permissive.to_string(), "Permissive");
    }

    #[test]
    fn scoring_profile_from_str() {
        assert_eq!(
            "strict".parse::<ScoringProfile>().unwrap(),
            ScoringProfile::Strict,
        );
        assert_eq!(
            "Balanced".parse::<ScoringProfile>().unwrap(),
            ScoringProfile::Balanced,
        );
        assert_eq!(
            "PERMISSIVE".parse::<ScoringProfile>().unwrap(),
            ScoringProfile::Permissive,
        );
        assert!("unknown".parse::<ScoringProfile>().is_err());
    }

    #[test]
    fn scoring_profile_serde_roundtrip() {
        let profile = ScoringProfile::Balanced;
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(json, r#""balanced""#);
        let deserialized: ScoringProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, profile);
    }
}
