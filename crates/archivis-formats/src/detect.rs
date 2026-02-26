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
        return Ok(detect_zip_format(data));
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
fn detect_zip_format(data: &[u8]) -> BookFormat {
    let cursor = Cursor::new(data);
    if let Ok(archive) = zip::ZipArchive::new(cursor) {
        // EPUB: must contain a `mimetype` entry whose content is `application/epub+zip`
        if is_epub(&archive) {
            debug!("detected EPUB via mimetype entry in ZIP");
            return BookFormat::Epub;
        }

        // CBZ: ZIP containing image files
        if is_cbz(&archive) {
            debug!("detected CBZ via image file entries in ZIP");
            return BookFormat::Cbz;
        }

        // Generic ZIP that is neither EPUB nor CBZ
        debug!("ZIP archive does not match EPUB or CBZ, returning Unknown");
        BookFormat::Unknown
    } else {
        // Central directory missing (truncated data) — fall back to local headers.
        debug!("ZIP central directory not found, falling back to local header scan");
        detect_zip_from_local_headers(data)
    }
}

/// ZIP local file header fixed size (before variable-length fields).
const ZIP_LOCAL_HEADER_SIZE: usize = 30;

/// Fallback ZIP format detection by parsing local file headers directly from raw
/// bytes. This works even when the ZIP central directory is missing (i.e. the
/// data is truncated to just the first few KB of the file).
fn detect_zip_from_local_headers(data: &[u8]) -> BookFormat {
    // EPUB: first entry must be "mimetype" stored uncompressed
    if detect_epub_from_local_header(data) {
        debug!("detected EPUB via local file header (truncated ZIP)");
        return BookFormat::Epub;
    }

    // CBZ: walk entries looking for image filenames
    if detect_cbz_from_local_headers(data) {
        debug!("detected CBZ via local file headers (truncated ZIP)");
        return BookFormat::Cbz;
    }

    BookFormat::Unknown
}

/// Check for EPUB by inspecting the first ZIP local file header.
///
/// Per the EPUB spec the first entry must be named `mimetype`, stored
/// uncompressed, and contain `application/epub+zip`.
fn detect_epub_from_local_header(data: &[u8]) -> bool {
    if data.len() < ZIP_LOCAL_HEADER_SIZE {
        return false;
    }

    // Compression method at offset 8 — must be 0 (stored).
    let compression = u16::from_le_bytes([data[8], data[9]]);
    if compression != 0 {
        return false;
    }

    let filename_len = u16::from_le_bytes([data[26], data[27]]) as usize;
    let extra_len = u16::from_le_bytes([data[28], data[29]]) as usize;

    let filename_end = ZIP_LOCAL_HEADER_SIZE + filename_len;
    if data.len() < filename_end {
        return false;
    }

    if &data[ZIP_LOCAL_HEADER_SIZE..filename_end] != b"mimetype" {
        return false;
    }

    let content_start = filename_end + extra_len;
    let expected = b"application/epub+zip";
    let content_end = content_start + expected.len();

    if data.len() < content_end {
        return false;
    }

    &data[content_start..content_end] == expected
}

/// Walk ZIP local file headers looking for image file entries (CBZ indicator).
fn detect_cbz_from_local_headers(data: &[u8]) -> bool {
    const IMAGE_EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp"];

    let mut offset = 0;

    while offset + ZIP_LOCAL_HEADER_SIZE <= data.len() {
        if &data[offset..offset + 4] != ZIP_MAGIC {
            break;
        }

        let filename_len = u16::from_le_bytes([data[offset + 26], data[offset + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[offset + 28], data[offset + 29]]) as usize;
        let compressed_size = u32::from_le_bytes([
            data[offset + 18],
            data[offset + 19],
            data[offset + 20],
            data[offset + 21],
        ]) as usize;

        let filename_start = offset + ZIP_LOCAL_HEADER_SIZE;
        let filename_end = filename_start + filename_len;

        if filename_end > data.len() {
            break;
        }

        let filename = String::from_utf8_lossy(&data[filename_start..filename_end]).to_lowercase();

        if !filename.ends_with('/')
            && !filename.starts_with("__macosx")
            && !filename.starts_with('.')
            && IMAGE_EXTENSIONS.iter().any(|ext| filename.ends_with(ext))
        {
            return true;
        }

        // Advance to the next local file header.
        let next = filename_end + extra_len + compressed_size;
        if next <= offset {
            break;
        }
        offset = next;
    }

    false
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

    /// Build a ZIP local file header with the given filename and uncompressed content.
    #[allow(clippy::cast_possible_truncation)]
    fn build_local_file_header(filename: &[u8], content: &[u8], compression: u16) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(ZIP_MAGIC); // signature
        header.extend_from_slice(&20u16.to_le_bytes()); // version needed
        header.extend_from_slice(&0u16.to_le_bytes()); // general purpose bit flag
        header.extend_from_slice(&compression.to_le_bytes()); // compression method
        header.extend_from_slice(&0u16.to_le_bytes()); // last mod time
        header.extend_from_slice(&0u16.to_le_bytes()); // last mod date
        header.extend_from_slice(&0u32.to_le_bytes()); // crc-32
        header.extend_from_slice(&(content.len() as u32).to_le_bytes()); // compressed size
        header.extend_from_slice(&(content.len() as u32).to_le_bytes()); // uncompressed size
        header.extend_from_slice(&(filename.len() as u16).to_le_bytes()); // filename length
        header.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        header.extend_from_slice(filename);
        header.extend_from_slice(content);
        header
    }

    #[test]
    fn detect_epub_from_truncated_zip() {
        let data = build_local_file_header(b"mimetype", b"application/epub+zip", 0);
        assert_eq!(detect(&data).unwrap(), BookFormat::Epub);
    }

    #[test]
    fn detect_cbz_from_truncated_zip() {
        let mut data = build_local_file_header(b"cover.jpg", &[], 8);
        // Add a second entry to exercise the walk loop.
        data.extend_from_slice(&build_local_file_header(b"page01.png", &[], 8));
        assert_eq!(detect(&data).unwrap(), BookFormat::Cbz);
    }

    #[test]
    fn detect_unknown_zip_truncated() {
        // ZIP with an entry that is neither an EPUB mimetype nor an image.
        let data = build_local_file_header(b"readme.txt", b"hello", 0);
        assert_eq!(detect(&data).unwrap(), BookFormat::Unknown);
    }
}
