//! Deterministic ZIP writer for KEPUB output.
//!
//! Determinism is required so that an HTTP `ETag` derived from the source
//! file hash plus the transformer id/version stays stable across calls.
//! The writer enforces:
//!
//! * `mimetype` entry first, content `application/kepub+zip`,
//!   `CompressionMethod::Stored`.
//! * All other entries deflated.
//! * Fixed timestamp `1980-01-01 00:00:00`.
//! * Fixed Unix permissions `0o644`.
//! * Entries appended in lexicographic order (callers sort before adding).

use std::io::{Cursor, Write};

use archivis_core::errors::FormatError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::pipeline::TARGET_MIME;

/// Builder around `zip::ZipWriter` that sets deterministic options.
pub struct DeterministicZipWriter {
    writer: ZipWriter<Cursor<Vec<u8>>>,
    mimetype_written: bool,
    last_path: Option<String>,
}

impl DeterministicZipWriter {
    pub fn new() -> Self {
        let cursor = Cursor::new(Vec::new());
        Self {
            writer: ZipWriter::new(cursor),
            mimetype_written: false,
            last_path: None,
        }
    }

    /// Write the EPUB-mandated `mimetype` entry. Must be called first;
    /// returns an error if any other entry was already written.
    pub fn write_mimetype(&mut self) -> Result<(), FormatError> {
        if self.mimetype_written {
            return Err(parse_err("mimetype written twice"));
        }
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(fixed_timestamp())
            .unix_permissions(0o644);
        self.writer
            .start_file("mimetype", opts)
            .map_err(|e| parse_err(&format!("mimetype start: {e}")))?;
        self.writer
            .write_all(TARGET_MIME.as_bytes())
            .map_err(FormatError::Io)?;
        self.mimetype_written = true;
        self.last_path = Some("mimetype".into());
        Ok(())
    }

    /// Append a deflated entry. Caller is responsible for adding entries
    /// in lexicographic order; an ordering violation returns
    /// `FormatError::Parse`.
    pub fn write_deflated(&mut self, path: &str, data: &[u8]) -> Result<(), FormatError> {
        if !self.mimetype_written {
            return Err(parse_err("mimetype must be written first"));
        }
        if path == "mimetype" {
            return Err(parse_err("cannot rewrite mimetype as deflated entry"));
        }
        if let Some(prev) = &self.last_path {
            if prev != "mimetype" && prev.as_str() >= path {
                return Err(parse_err(&format!(
                    "non-lexicographic entry order: {prev} → {path}"
                )));
            }
        }
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(fixed_timestamp())
            .unix_permissions(0o644);
        self.writer
            .start_file(path.to_string(), opts)
            .map_err(|e| parse_err(&format!("start {path}: {e}")))?;
        self.writer.write_all(data).map_err(FormatError::Io)?;
        self.last_path = Some(path.to_string());
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>, FormatError> {
        let cursor = self
            .writer
            .finish()
            .map_err(|e| parse_err(&format!("finalize zip: {e}")))?;
        Ok(cursor.into_inner())
    }
}

impl Default for DeterministicZipWriter {
    fn default() -> Self {
        Self::new()
    }
}

fn fixed_timestamp() -> DateTime {
    // 1980-01-01 00:00:00 — DOS epoch, the minimum valid ZIP timestamp.
    DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).expect("1980-01-01 is a valid ZIP datetime")
}

fn parse_err(msg: &str) -> FormatError {
    FormatError::Parse {
        format: "KEPUB".into(),
        message: msg.into(),
    }
}
