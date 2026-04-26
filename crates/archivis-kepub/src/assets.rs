//! Embedded assets injected into the converted KEPUB.

/// Placeholder Kobo reader stub. Vendored from kepubify (MIT) — see
/// `THIRD_PARTY_LICENSES.md` at the repo root.
pub const KOBO_JS: &[u8] = include_bytes!("assets/kobo.js");

/// Path used when injecting `kobo.js` into an EPUB. The kepubify reference
/// implementation places it under the root of the package, which keeps
/// references simple regardless of where OPF lives.
pub const KOBO_JS_PATH: &str = "kobo.js";
