//! Canonical provider name constants shared across the metadata crate.
//!
//! Centralises string literals so that `identifier_type_belongs_to_provider`,
//! cover-priority lists, and individual provider modules all reference the
//! same constants.

pub const OPEN_LIBRARY: &str = "open_library";
pub const HARDCOVER: &str = "hardcover";
pub const GOOGLE_BOOKS: &str = "google_books";
