use std::io::Cursor;

use archivis_core::errors::FormatError;
use archivis_core::models::BookFormat;
use tracing::debug;

/// Minimum number of bytes needed for any meaningful format detection.
const MIN_BYTES: usize = 8;

/// How many bytes of a file to inspect for text-based format heuristics (FB2, TXT).
const PROBE_SIZE: usize = 8192;

/// Magic bytes for known formats.
const PDF_MAGIC: &[u8] = b"%PDF-";
const ZIP_MAGIC: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
const DJVU_MAGIC: &[u8] = b"AT&TFORM";
const MOBI_MAGIC: &[u8] = b"BOOKMOBI";

/// Byte offset in PDB header where the MOBI type identifier lives.
const PDB_TYPE_OFFSET: usize = 60;

/// Detect the ebook format of `data` by examining magic bytes and file structure.
///
/// The function inspects raw bytes — it never trusts file extensions.
/// Returns `FormatError::Detection` for empty or too-small inputs.
pub fn detect(data: &[u8]) -> Result<BookFormat, FormatError> {
    if data.len() < MIN_BYTES {
        return Err(FormatError::Detection(
            "input too small for format detection".into(),
        ));
    }

    // PDF: starts with %PDF-
    if data.starts_with(PDF_MAGIC) {
        debug!("detected PDF via magic bytes");
        return Ok(BookFormat::Pdf);
    }

    // DJVU: starts with AT&TFORM
    if data.starts_with(DJVU_MAGIC) {
        debug!("detected DJVU via magic bytes");
        return Ok(BookFormat::Djvu);
    }

    // MOBI / AZW3: PDB header with BOOKMOBI at offset 60
    if data.len() > PDB_TYPE_OFFSET + MOBI_MAGIC.len()
        && data[PDB_TYPE_OFFSET..].starts_with(MOBI_MAGIC)
    {
        // Use mobi-book to distinguish KF8 (AZW3) from legacy MOBI
        if let Ok(book) = mobi_book::MobiBook::new(data) {
            if book.kf8_info().is_kf8 {
                debug!("detected AZW3 (KF8) via mobi-book");
                return Ok(BookFormat::Azw3);
            }
        }
        debug!("detected MOBI via PDB header magic");
        return Ok(BookFormat::Mobi);
    }

    // ZIP-based formats: EPUB and CBZ
    if data.starts_with(ZIP_MAGIC) {
        return detect_zip_format(data);
    }

    // FB2: XML with <FictionBook root element
    if detect_fb2(data) {
        debug!("detected FB2 via XML root element");
        return Ok(BookFormat::Fb2);
    }

    // TXT: valid UTF-8 with no null bytes in the probe window
    if detect_txt(data) {
        debug!("detected TXT via UTF-8 heuristic");
        return Ok(BookFormat::Txt);
    }

    debug!("format not recognised, returning Unknown");
    Ok(BookFormat::Unknown)
}

/// Distinguish EPUB from CBZ inside a ZIP archive.
fn detect_zip_format(data: &[u8]) -> Result<BookFormat, FormatError> {
    let cursor = Cursor::new(data);
    let archive = zip::ZipArchive::new(cursor)
        .map_err(|e| FormatError::Detection(format!("ZIP appears corrupt: {e}")))?;

    // EPUB: must contain a `mimetype` entry whose content is `application/epub+zip`
    if is_epub(&archive) {
        debug!("detected EPUB via mimetype entry in ZIP");
        return Ok(BookFormat::Epub);
    }

    // CBZ: ZIP containing image files
    if is_cbz(&archive) {
        debug!("detected CBZ via image file entries in ZIP");
        return Ok(BookFormat::Cbz);
    }

    // Generic ZIP that is neither EPUB nor CBZ
    debug!("ZIP archive does not match EPUB or CBZ, returning Unknown");
    Ok(BookFormat::Unknown)
}

/// Check if a ZIP archive is a valid EPUB by looking for the `mimetype` entry.
fn is_epub(archive: &zip::ZipArchive<Cursor<&[u8]>>) -> bool {
    // The EPUB spec requires the first file in the archive to be named
    // `mimetype` with the value `application/epub+zip`, stored (not compressed).
    // We are lenient: accept it anywhere in the archive.
    let mut archive = archive.clone();
    let Ok(mut entry) = archive.by_name("mimetype") else {
        return false;
    };

    let mut buf = Vec::with_capacity(64);
    if std::io::Read::read_to_end(&mut entry, &mut buf).is_err() {
        return false;
    }

    let content = String::from_utf8_lossy(&buf);
    content.trim() == "application/epub+zip"
}

/// Check if a ZIP archive looks like a CBZ (contains image files).
fn is_cbz(archive: &zip::ZipArchive<Cursor<&[u8]>>) -> bool {
    let image_extensions = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"];

    for i in 0..archive.len() {
        let mut cloned = archive.clone();
        let Ok(entry) = cloned.by_index(i) else {
            continue;
        };
        let name = entry.name().to_lowercase();

        // Skip directories and hidden/metadata files
        if name.ends_with('/') || name.starts_with("__macosx") || name.starts_with('.') {
            continue;
        }

        if image_extensions.iter().any(|ext| name.ends_with(ext)) {
            return true;
        }
    }

    false
}

/// Detect FB2 by searching for `<FictionBook` in the first bytes,
/// handling optional BOM and XML declaration.
fn detect_fb2(data: &[u8]) -> bool {
    let probe = &data[..data.len().min(PROBE_SIZE)];

    // Skip UTF-8 BOM if present
    let start = if probe.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };

    let Ok(text) = std::str::from_utf8(&probe[start..]) else {
        return false;
    };

    // Look for the FictionBook root element, ignoring XML declarations and whitespace
    text.contains("<FictionBook")
}

/// Heuristic for plain text: valid UTF-8 with no null bytes in the probe window.
fn detect_txt(data: &[u8]) -> bool {
    let probe = &data[..data.len().min(PROBE_SIZE)];

    // Reject if any null bytes present (binary indicator)
    if probe.contains(&0x00) {
        return false;
    }

    std::str::from_utf8(probe).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_errors() {
        assert!(detect(&[]).is_err());
    }

    #[test]
    fn too_small_input_errors() {
        assert!(detect(&[0x50, 0x4B]).is_err());
    }

    #[test]
    fn detect_pdf() {
        let mut data = b"%PDF-1.7 fake pdf content".to_vec();
        data.resize(512, 0);
        assert_eq!(detect(&data).unwrap(), BookFormat::Pdf);
    }

    #[test]
    fn detect_djvu() {
        let mut data = b"AT&TFORM\x00\x00\x00\x00DJVU".to_vec();
        data.resize(512, 0);
        assert_eq!(detect(&data).unwrap(), BookFormat::Djvu);
    }

    #[test]
    fn detect_mobi() {
        // PDB header: 60 bytes of padding, then BOOKMOBI
        let mut data = vec![0u8; 60];
        data.extend_from_slice(b"BOOKMOBI");
        data.resize(512, 0);
        assert_eq!(detect(&data).unwrap(), BookFormat::Mobi);
    }

    #[test]
    fn detect_txt() {
        let data = b"This is a plain text file with enough content to pass minimum size checks.";
        assert_eq!(detect(data).unwrap(), BookFormat::Txt);
    }

    #[test]
    fn detect_unknown_binary() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(detect(&data).unwrap(), BookFormat::Unknown);
    }

    #[test]
    fn detect_fb2_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
  <body><section><p>Hello</p></section></body>
</FictionBook>"#;
        assert_eq!(detect(xml.as_bytes()).unwrap(), BookFormat::Fb2);
    }

    #[test]
    fn detect_fb2_with_bom() {
        let mut data = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        data.extend_from_slice(b"<?xml version=\"1.0\"?>\n<FictionBook><body/></FictionBook>");
        assert_eq!(detect(&data).unwrap(), BookFormat::Fb2);
    }
}
