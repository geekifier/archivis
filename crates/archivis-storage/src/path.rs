use sanitise_file_name::{sanitise_with_options, Options};

/// Sanitization options: Windows-safe, collapses replacement characters, falls
/// back to `_unknown` for empty results.
const SANITIZE_OPTS: Options<Option<char>> = Options {
    collapse_replacements: true,
    six_measures_of_barley: "_unknown",
    ..Options::DEFAULT
};

/// Generate a relative storage path for a book file.
///
/// Pattern: `{first_letter}/{author}/{title}/{filename}`
///
/// All components are sanitized for cross-platform filesystem safety
/// (Windows reserved names, illegal characters, length limits, control
/// characters, bidirectional reordering attacks) via the `sanitise-file-name` crate.
///
/// The first letter is derived from the sanitized author name (uppercased).
/// Falls back to `_` for names starting with non-alphabetic characters.
pub fn generate_book_path(author: &str, title: &str, filename: &str) -> String {
    let author = sanitize(author);
    let title = sanitize(title);
    let filename = sanitize(filename);

    let first_letter = author
        .chars()
        .next()
        .filter(|c| c.is_alphabetic())
        .map_or_else(|| "_".to_owned(), |c| c.to_uppercase().to_string());

    format!("{first_letter}/{author}/{title}/{filename}")
}

/// Sanitize a single path component for safe cross-platform filesystem use.
fn sanitize(s: &str) -> String {
    sanitise_with_options(s.trim(), &SANITIZE_OPTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_path_generation() {
        let path = generate_book_path("Frank Herbert", "Dune", "dune.epub");
        assert_eq!(path, "F/Frank Herbert/Dune/dune.epub");
    }

    #[test]
    fn numeric_author_name() {
        let path = generate_book_path("42 Authors", "Anthology", "anthology.epub");
        assert_eq!(path, "_/42 Authors/Anthology/anthology.epub");
    }

    #[test]
    fn empty_author_falls_back() {
        let path = generate_book_path("", "Some Title", "book.epub");
        assert_eq!(path, "_/_unknown/Some Title/book.epub");
    }

    #[test]
    fn empty_title_falls_back() {
        let path = generate_book_path("Author", "", "book.epub");
        assert_eq!(path, "A/Author/_unknown/book.epub");
    }

    #[test]
    fn unicode_author_name() {
        let path = generate_book_path("Харуки Мураками", "1Q84", "1q84.epub");
        assert_eq!(path, "Х/Харуки Мураками/1Q84/1q84.epub");
    }

    #[test]
    fn leading_trailing_whitespace_trimmed() {
        let path = generate_book_path("  Author  ", "  Title  ", "  file.epub  ");
        assert_eq!(path, "A/Author/Title/file.epub");
    }

    #[test]
    fn filename_preserves_extension() {
        let path = generate_book_path("Author", "Title", "my.book.name.epub");
        assert_eq!(path, "A/Author/Title/my.book.name.epub");
    }

    #[test]
    fn windows_reserved_names_get_underscore_suffix() {
        assert_eq!(sanitize("CON"), "CON_");
        assert_eq!(sanitize("NUL"), "NUL_");
        assert_eq!(sanitize("aux.h"), "aux_.h");
        assert_eq!(sanitize("LPT1"), "LPT1_");
        assert_eq!(sanitize("com9"), "com9_");
    }

    #[test]
    fn slashes_replaced() {
        assert_eq!(sanitize("Author/Name"), "Author_Name");
        assert_eq!(sanitize("Back\\Slash"), "Back_Slash");
    }

    #[test]
    fn colons_replaced() {
        assert_eq!(sanitize("Title: A Subtitle"), "Title_ A Subtitle");
    }

    #[test]
    fn consecutive_unsafe_chars_collapsed() {
        // Three slashes collapse to one underscore
        assert_eq!(sanitize("A///B"), "A_B");
        // Three colons collapse to one underscore
        assert_eq!(sanitize("C:::D"), "C_D");
    }

    #[test]
    fn null_bytes_removed() {
        let result = sanitize("hello\0world");
        assert!(!result.contains('\0'));
    }

    #[test]
    fn all_invalid_becomes_unknown() {
        assert_eq!(sanitize("\0\0\0"), "_unknown");
        assert_eq!(sanitize("///"), "_unknown");
    }

    #[test]
    fn control_characters_removed() {
        let result = sanitize("hello\x01\x02world");
        assert!(!result.contains('\x01'));
        assert!(!result.contains('\x02'));
    }

    #[test]
    fn leading_dots_handled() {
        // Single leading dot is preserved (extension cleverness interprets as extension)
        assert_eq!(sanitize(".hidden"), ".hidden");
        // Multiple leading dots are trimmed (all-dots names are forbidden by most_fs_safe)
        assert_eq!(sanitize(".."), "_unknown");
    }

    #[test]
    fn preserves_hyphens_and_parens() {
        let result = sanitize("Title - Part (2024)");
        assert_eq!(result, "Title - Part (2024)");
    }

    #[test]
    fn preserves_apostrophe() {
        assert_eq!(sanitize("O'Brien"), "O'Brien");
    }

    #[test]
    fn long_names_truncated_to_255() {
        let long = "A".repeat(300);
        let result = sanitize(&long);
        assert!(result.len() <= 255);
    }
}
