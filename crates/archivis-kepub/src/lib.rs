//! KEPUB on-the-fly conversion.
//!
//! Implements the `archivis_formats::transform::FormatTransformer` trait
//! for `EPUB → KEPUB` conversion. The pipeline is deterministic so that
//! HTTP `ETag` values stay stable across calls.
//!
//! See `docs/2_.Design01.md` and the project's `kepubify`-derived plan for
//! the algorithmic specification.

#![allow(clippy::module_name_repetitions)]

pub mod assets;
pub mod container;
pub mod content;
pub mod opf;
pub mod pipeline;
pub mod spans;

use archivis_core::errors::FormatError;
use archivis_core::models::BookFormat;
use archivis_formats::transform::FormatTransformer;

/// EPUB → KEPUB transformer.
///
/// Bump [`Self::VERSION`] whenever the conversion output changes; clients
/// rely on it to invalidate cached `ETag`s.
#[derive(Debug, Default, Clone, Copy)]
pub struct KepubTransformer;

impl KepubTransformer {
    pub const ID: &'static str = "kepub";
    pub const VERSION: &'static str = "0.1.0";
    pub const TARGET_MIME: &'static str = "application/kepub+zip";
    pub const TARGET_EXTENSION: &'static str = "kepub.epub";
}

impl FormatTransformer for KepubTransformer {
    fn id(&self) -> &'static str {
        Self::ID
    }
    fn version(&self) -> &'static str {
        Self::VERSION
    }
    fn source_format(&self) -> BookFormat {
        BookFormat::Epub
    }
    fn target_mime(&self) -> &'static str {
        Self::TARGET_MIME
    }
    fn target_extension(&self) -> &'static str {
        Self::TARGET_EXTENSION
    }
    fn transform(&self, input: &[u8]) -> Result<Vec<u8>, FormatError> {
        pipeline::convert(input)
    }
}
