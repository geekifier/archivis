//! KEPUB conversion orchestrator.
//!
//! Reads the source EPUB ZIP, walks each entry, applies per-media-type
//! rewrites (XHTML → koboSpan, OPF → manifest registration), and emits a
//! deterministic output ZIP via [`crate::container::DeterministicZipWriter`].

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use archivis_core::errors::FormatError;
use tracing::warn;

use crate::assets::{KOBO_JS, KOBO_JS_PATH};
use crate::container::DeterministicZipWriter;
use crate::content;
use crate::opf;

pub const TARGET_MIME: &str = "application/kepub+zip";

/// Filenames that calibre and friends inject which the device does not need.
const EXCLUDE_PATHS: &[&str] = &[
    "META-INF/calibre_bookmarks.txt",
    "iTunesArtwork",
    "iTunesMetadata.plist",
    ".DS_Store",
];

/// Convert an EPUB byte stream to a KEPUB byte stream.
///
/// Failures during per-document rewrite are logged and the affected
/// document is passed through unchanged; only ZIP-level errors and OPF
/// rewrite failures bubble up.
pub fn convert(epub: &[u8]) -> Result<Vec<u8>, FormatError> {
    let cursor = Cursor::new(epub);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| FormatError::Parse {
        format: "KEPUB".into(),
        message: format!("invalid ZIP archive: {e}"),
    })?;

    // Locate the OPF.
    let opf_path = find_opf_path(&mut archive)?;
    let opf_dir = opf_directory(&opf_path);
    let opf_bytes = read_entry(&mut archive, &opf_path)?;

    // Parse spine info to identify XHTML documents.
    let parsed = opf::parse_spine(&opf_bytes, &opf_dir)?;
    let fixed_layout = parsed.is_fixed_layout();

    // Pre-read every entry into a sorted map so we can emit lexicographically.
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for i in 0..archive.len() {
        let name = {
            let f = archive
                .by_index_raw(i)
                .map_err(|e| parse_err(&format!("zip index {i}: {e}")))?;
            f.name().to_string()
        };
        if name == "mimetype" {
            // Will be re-emitted by the deterministic writer.
            continue;
        }
        if EXCLUDE_PATHS.contains(&name.as_str()) {
            continue;
        }
        if name.ends_with('/') {
            // Skip directory entries.
            continue;
        }
        let data = read_entry_by_index(&mut archive, i)?;
        entries.insert(name, data);
    }

    // Compute the relative href from each XHTML document to root-level
    // kobo.js. We place kobo.js at root for v1.
    let kobo_root_path = KOBO_JS_PATH.to_string();

    // Inject kobo.js source (idempotent; pipeline always rewrites).
    entries.insert(kobo_root_path.clone(), KOBO_JS.to_vec());

    // Rewrite OPF: add manifest entry, accounting for opf_dir.
    let opf_kobo_href = relative_href(&opf_dir, &kobo_root_path);
    let new_opf = opf::add_kobo_manifest_entry(&opf_bytes, &opf_kobo_href)?;
    entries.insert(opf_path.clone(), new_opf);

    // Rewrite each XHTML spine document (best-effort).
    let xhtml_paths: Vec<String> = parsed
        .spine
        .iter()
        .filter(|d| d.media_type == "application/xhtml+xml" || d.media_type == "text/html")
        .map(|d| d.path.clone())
        .collect();

    for path in xhtml_paths {
        if fixed_layout {
            // Fixed-layout: pass through, no span injection.
            continue;
        }
        let Some(orig) = entries.get(&path).cloned() else {
            continue;
        };
        let kobo_href = relative_href(&parent_dir(&path), &kobo_root_path);
        match content::rewrite(&orig, &kobo_href) {
            Ok(rewritten) => {
                entries.insert(path, rewritten);
            }
            Err(e) => {
                warn!(
                    path = %path,
                    error = %e,
                    "kepub: per-document rewrite failed; passing through original"
                );
            }
        }
    }

    // Emit deterministic output.
    let mut out = DeterministicZipWriter::new();
    out.write_mimetype()?;
    for (path, data) in entries {
        out.write_deflated(&path, &data)?;
    }
    out.finish()
}

fn find_opf_path<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<String, FormatError> {
    let xml = read_entry(archive, "META-INF/container.xml")?;
    archivis_formats::epub::parse_container_rootfile(&xml).map_err(|m| parse_err(&m))
}

fn opf_directory(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(String::new(), |(d, _)| d.to_string())
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(String::new(), |(d, _)| d.to_string())
}

/// Compute the href from `from_dir` to `target_path`, using `..` segments.
/// Both paths are expressed inside the EPUB ZIP (forward-slash separated,
/// no leading slash).
fn relative_href(from_dir: &str, target_path: &str) -> String {
    if from_dir.is_empty() {
        return target_path.to_string();
    }
    let depth = from_dir.split('/').filter(|s| !s.is_empty()).count();
    let mut prefix = String::new();
    for _ in 0..depth {
        prefix.push_str("../");
    }
    prefix.push_str(target_path);
    prefix
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>, FormatError> {
    let mut f = archive
        .by_name(path)
        .map_err(|e| parse_err(&format!("zip entry {path}: {e}")))?;
    let cap = usize::try_from(f.size()).unwrap_or(0);
    let mut buf = Vec::with_capacity(cap);
    f.read_to_end(&mut buf).map_err(FormatError::Io)?;
    Ok(buf)
}

fn read_entry_by_index<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    i: usize,
) -> Result<Vec<u8>, FormatError> {
    let mut f = archive
        .by_index(i)
        .map_err(|e| parse_err(&format!("zip index {i}: {e}")))?;
    let cap = usize::try_from(f.size()).unwrap_or(0);
    let mut buf = Vec::with_capacity(cap);
    f.read_to_end(&mut buf).map_err(FormatError::Io)?;
    Ok(buf)
}

fn parse_err(msg: &str) -> FormatError {
    FormatError::Parse {
        format: "KEPUB".into(),
        message: msg.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_href_root_target() {
        assert_eq!(relative_href("", "kobo.js"), "kobo.js");
        assert_eq!(relative_href("OEBPS", "kobo.js"), "../kobo.js");
        assert_eq!(relative_href("OEBPS/text", "kobo.js"), "../../kobo.js");
    }

    #[test]
    fn opf_directory_basic() {
        assert_eq!(opf_directory("OEBPS/content.opf"), "OEBPS");
        assert_eq!(opf_directory("content.opf"), "");
        assert_eq!(opf_directory("a/b/c.opf"), "a/b");
    }
}
