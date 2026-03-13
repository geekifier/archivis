//! Language normalization utilities.
//!
//! Establishes ISO 639-1 (2-letter codes) as the single internal
//! representation for languages throughout the application.

use isolang::Language;

/// ISO 639-2/B (bibliographic) codes that differ from 639-3 (terminology).
///
/// `isolang` uses 639-3 codes internally, so these B-codes must be mapped
/// to their 639-3 equivalents before lookup.
const ISO_639_2B_TO_3: &[(&str, &str)] = &[
    ("alb", "sqi"),
    ("arm", "hye"),
    ("baq", "eus"),
    ("bur", "mya"),
    ("chi", "zho"),
    ("cze", "ces"),
    ("dut", "nld"),
    ("fre", "fra"),
    ("geo", "kat"),
    ("ger", "deu"),
    ("gre", "ell"),
    ("ice", "isl"),
    ("iri", "gle"),
    ("mac", "mkd"),
    ("mao", "mri"),
    ("may", "msa"),
    ("per", "fas"),
    ("rum", "ron"),
    ("slo", "slk"),
    ("tib", "bod"),
    ("wel", "cym"),
];

/// Native-language names that `isolang::from_name_lowercase` does not cover.
///
/// Sourced from the Hardcover provider's existing mapping.
const NATIVE_NAMES: &[(&str, &str)] = &[
    ("bahasa indonesia", "id"),
    ("bahasa melayu", "ms"),
    ("català", "ca"),
    ("dansk", "da"),
    ("deutsch", "de"),
    ("eesti", "et"),
    ("español", "es"),
    ("français", "fr"),
    ("hrvatski", "hr"),
    ("italiano", "it"),
    ("latviešu", "lv"),
    ("lietuvių", "lt"),
    ("magyar", "hu"),
    ("nederlands", "nl"),
    ("norsk", "no"),
    ("polski", "pl"),
    ("português", "pt"),
    ("română", "ro"),
    ("slovenščina", "sl"),
    ("srpski", "sr"),
    ("suomi", "fi"),
    ("svenska", "sv"),
    ("türkçe", "tr"),
    ("čeština", "cs"),
    ("ελληνικά", "el"),
    ("български", "bg"),
    ("русский", "ru"),
    ("українська", "uk"),
    ("עברית", "he"),
    ("العربية", "ar"),
    ("فارسی", "fa"),
    ("اردو", "ur"),
    ("हिन्दी", "hi"),
    ("ไทย", "th"),
    ("中文", "zh"),
    ("日本語", "ja"),
    ("한국어", "ko"),
    ("tiếng việt", "vi"),
];

/// Normalize any language identifier to an ISO 639-1 (2-letter) code.
///
/// Accepts ISO 639-1/2/3 codes, BCP 47 tags (e.g. `"en-US"`), English
/// names, and common native names. Returns `None` for empty, unknown,
/// or undetermined inputs.
pub fn normalize_language(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("und")
        || trimmed.eq_ignore_ascii_case("undetermined")
    {
        return None;
    }

    // BCP 47 / locale tags: extract primary subtag
    if let Some(pos) = trimmed.find(['-', '_']) {
        return normalize_language(&trimmed[..pos]);
    }

    let lower = trimmed.to_lowercase();

    // 2-letter code → validate via isolang
    if lower.len() == 2 && lower.is_ascii() {
        return Language::from_639_1(&lower).and_then(|l| l.to_639_1());
    }

    // 3-letter code → try 639-3 first, then 639-2/B bridge
    if lower.len() == 3 && lower.is_ascii() {
        if let Some(code) = Language::from_639_3(&lower).and_then(|l| l.to_639_1()) {
            return Some(code);
        }
        // Bridge 639-2/B → 639-3
        for &(bib, term) in ISO_639_2B_TO_3 {
            if lower == bib {
                return Language::from_639_3(term).and_then(|l| l.to_639_1());
            }
        }
        return None;
    }

    // English name via isolang
    if let Some(code) = Language::from_name_lowercase(&lower).and_then(|l| l.to_639_1()) {
        return Some(code);
    }

    // Native name fallback
    for &(name, code) in NATIVE_NAMES {
        if lower == name {
            return Some(code);
        }
    }

    None
}

/// Return the English display name for an ISO 639-1 code.
///
/// Example: `"en"` → `Some("English")`.
pub fn language_label(code: &str) -> Option<&'static str> {
    Language::from_639_1(code).map(|l| l.to_name())
}

/// Check whether `code` is a known ISO 639-1 (2-letter) code.
pub fn is_valid_iso639_1(code: &str) -> bool {
    Language::from_639_1(code).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_language ──────────────────────────────────────────

    #[test]
    fn two_letter_codes() {
        assert_eq!(normalize_language("en"), Some("en"));
        assert_eq!(normalize_language("fr"), Some("fr"));
        assert_eq!(normalize_language("de"), Some("de"));
        assert_eq!(normalize_language("pl"), Some("pl"));
        assert_eq!(normalize_language("ja"), Some("ja"));
    }

    #[test]
    fn two_letter_mixed_case() {
        assert_eq!(normalize_language("EN"), Some("en"));
        assert_eq!(normalize_language("Fr"), Some("fr"));
        assert_eq!(normalize_language("dE"), Some("de"));
    }

    #[test]
    fn three_letter_639_3_codes() {
        assert_eq!(normalize_language("eng"), Some("en"));
        assert_eq!(normalize_language("fra"), Some("fr"));
        assert_eq!(normalize_language("deu"), Some("de"));
        assert_eq!(normalize_language("spa"), Some("es"));
        assert_eq!(normalize_language("jpn"), Some("ja"));
        assert_eq!(normalize_language("pol"), Some("pl"));
        assert_eq!(normalize_language("zho"), Some("zh"));
        assert_eq!(normalize_language("nld"), Some("nl"));
    }

    #[test]
    fn three_letter_639_2b_bibliographic() {
        assert_eq!(normalize_language("fre"), Some("fr"));
        assert_eq!(normalize_language("ger"), Some("de"));
        assert_eq!(normalize_language("chi"), Some("zh"));
        assert_eq!(normalize_language("dut"), Some("nl"));
        assert_eq!(normalize_language("cze"), Some("cs"));
        assert_eq!(normalize_language("rum"), Some("ro"));
        assert_eq!(normalize_language("gre"), Some("el"));
        assert_eq!(normalize_language("per"), Some("fa"));
        assert_eq!(normalize_language("slo"), Some("sk"));
        assert_eq!(normalize_language("wel"), Some("cy"));
        assert_eq!(normalize_language("iri"), Some("ga"));
        assert_eq!(normalize_language("may"), Some("ms"));
        // "mal" is ISO 639-3 for Malayalam, not a B-code for Malay
        assert_eq!(normalize_language("mal"), Some("ml"));
    }

    #[test]
    fn three_letter_case_insensitive() {
        assert_eq!(normalize_language("ENG"), Some("en"));
        assert_eq!(normalize_language("FRE"), Some("fr"));
        assert_eq!(normalize_language("Deu"), Some("de"));
    }

    #[test]
    fn english_names() {
        assert_eq!(normalize_language("English"), Some("en"));
        assert_eq!(normalize_language("French"), Some("fr"));
        assert_eq!(normalize_language("German"), Some("de"));
        assert_eq!(normalize_language("Spanish"), Some("es"));
        assert_eq!(normalize_language("Japanese"), Some("ja"));
        assert_eq!(normalize_language("Chinese"), Some("zh"));
        assert_eq!(normalize_language("Polish"), Some("pl"));
    }

    #[test]
    fn english_names_case_insensitive() {
        assert_eq!(normalize_language("english"), Some("en"));
        assert_eq!(normalize_language("ENGLISH"), Some("en"));
        assert_eq!(normalize_language("English"), Some("en"));
    }

    #[test]
    fn native_names() {
        assert_eq!(normalize_language("français"), Some("fr"));
        assert_eq!(normalize_language("deutsch"), Some("de"));
        assert_eq!(normalize_language("español"), Some("es"));
        assert_eq!(normalize_language("italiano"), Some("it"));
        assert_eq!(normalize_language("русский"), Some("ru"));
        assert_eq!(normalize_language("日本語"), Some("ja"));
        assert_eq!(normalize_language("中文"), Some("zh"));
        assert_eq!(normalize_language("한국어"), Some("ko"));
        assert_eq!(normalize_language("العربية"), Some("ar"));
    }

    #[test]
    fn bcp47_tags() {
        assert_eq!(normalize_language("en-US"), Some("en"));
        assert_eq!(normalize_language("en-GB"), Some("en"));
        assert_eq!(normalize_language("pt-BR"), Some("pt"));
        assert_eq!(normalize_language("zh-Hans"), Some("zh"));
        assert_eq!(normalize_language("zh-TW"), Some("zh"));
    }

    #[test]
    fn locale_underscore() {
        assert_eq!(normalize_language("en_US"), Some("en"));
        assert_eq!(normalize_language("pt_BR"), Some("pt"));
    }

    #[test]
    fn empty_and_undetermined() {
        assert_eq!(normalize_language(""), None);
        assert_eq!(normalize_language("   "), None);
        assert_eq!(normalize_language("und"), None);
        assert_eq!(normalize_language("UND"), None);
        assert_eq!(normalize_language("undetermined"), None);
        assert_eq!(normalize_language("Undetermined"), None);
    }

    #[test]
    fn unknown_values() {
        assert_eq!(normalize_language("Klingon"), None);
        assert_eq!(normalize_language("zzz"), None);
        assert_eq!(normalize_language("xxx"), None);
        assert_eq!(normalize_language("xx"), None);
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(normalize_language("  en  "), Some("en"));
        assert_eq!(normalize_language(" English "), Some("en"));
    }

    // ── language_label ──────────────────────────────────────────────

    #[test]
    fn label_known_codes() {
        assert_eq!(language_label("en"), Some("English"));
        assert_eq!(language_label("fr"), Some("French"));
        assert_eq!(language_label("de"), Some("German"));
        assert_eq!(language_label("ja"), Some("Japanese"));
    }

    #[test]
    fn label_unknown_code() {
        assert_eq!(language_label("xx"), None);
        assert_eq!(language_label(""), None);
    }

    // ── is_valid_iso639_1 ───────────────────────────────────────────

    #[test]
    fn valid_codes() {
        assert!(is_valid_iso639_1("en"));
        assert!(is_valid_iso639_1("fr"));
        assert!(is_valid_iso639_1("de"));
    }

    #[test]
    fn invalid_codes() {
        assert!(!is_valid_iso639_1("xx"));
        assert!(!is_valid_iso639_1("eng"));
        assert!(!is_valid_iso639_1(""));
    }
}
