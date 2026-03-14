/// ISBN validation and conversion utilities.
///
/// Provides checksum validation for ISBN-10 and ISBN-13, normalization
/// (stripping hyphens/spaces), and bidirectional ISBN-10/ISBN-13 conversion.
use std::fmt;

use crate::models::IdentifierType;

/// The detected type of an ISBN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsbnType {
    Isbn10,
    Isbn13,
}

impl fmt::Display for IsbnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Isbn10 => write!(f, "ISBN-10"),
            Self::Isbn13 => write!(f, "ISBN-13"),
        }
    }
}

/// Result of validating an ISBN string.
#[derive(Debug, Clone)]
pub struct IsbnValidation {
    /// Whether the ISBN is valid.
    pub valid: bool,
    /// The normalized form (hyphens/spaces removed, uppercase).
    pub normalized: String,
    /// The detected type, if valid.
    pub isbn_type: Option<IsbnType>,
    /// Human-readable validation message.
    pub message: String,
}

/// Validate and normalize an ISBN string.
///
/// Strips hyphens and spaces, validates length, checks the checksum digit,
/// and returns a detailed result.
pub fn validate_isbn(input: &str) -> IsbnValidation {
    let normalized: String = input
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .collect::<String>()
        .to_uppercase();

    if normalized.is_empty() {
        return IsbnValidation {
            valid: false,
            normalized,
            isbn_type: None,
            message: "ISBN must not be empty".into(),
        };
    }

    match normalized.len() {
        13 => {
            // Must be all digits
            if !normalized.chars().all(|c| c.is_ascii_digit()) {
                return IsbnValidation {
                    valid: false,
                    normalized,
                    isbn_type: None,
                    message: "ISBN-13 must contain only digits".into(),
                };
            }
            if validate_isbn13_checksum(&normalized) {
                IsbnValidation {
                    valid: true,
                    normalized,
                    isbn_type: Some(IsbnType::Isbn13),
                    message: "Valid ISBN-13".into(),
                }
            } else {
                let expected = compute_isbn13_check_digit(&normalized[..12]);
                let got = normalized.chars().last().unwrap();
                IsbnValidation {
                    valid: false,
                    normalized,
                    isbn_type: None,
                    message: format!(
                        "ISBN-13 checksum failed: expected check digit {expected}, got {got}"
                    ),
                }
            }
        }
        10 => {
            // First 9 must be digits, last can be digit or X
            let chars: Vec<char> = normalized.chars().collect();
            if !chars[..9].iter().all(char::is_ascii_digit) {
                return IsbnValidation {
                    valid: false,
                    normalized,
                    isbn_type: None,
                    message: "ISBN-10 must contain only digits (with optional trailing X)".into(),
                };
            }
            let last = chars[9];
            if !last.is_ascii_digit() && last != 'X' {
                return IsbnValidation {
                    valid: false,
                    normalized,
                    isbn_type: None,
                    message: "ISBN-10 check digit must be a digit or X".into(),
                };
            }
            if validate_isbn10_checksum(&normalized) {
                IsbnValidation {
                    valid: true,
                    normalized,
                    isbn_type: Some(IsbnType::Isbn10),
                    message: "Valid ISBN-10".into(),
                }
            } else {
                let expected = compute_isbn10_check_digit(&normalized[..9]);
                let got = last;
                IsbnValidation {
                    valid: false,
                    normalized,
                    isbn_type: None,
                    message: format!(
                        "ISBN-10 checksum failed: expected check digit {expected}, got {got}"
                    ),
                }
            }
        }
        _ => {
            let msg = format!(
                "Invalid ISBN length: expected 10 or 13 characters, got {}",
                normalized.len()
            );
            IsbnValidation {
                valid: false,
                normalized,
                isbn_type: None,
                message: msg,
            }
        }
    }
}

/// Validate an ISBN-13 checksum.
/// Sum of (digit x alternating 1,3,1,3...) mod 10 == 0.
pub(crate) fn validate_isbn13_checksum(isbn: &str) -> bool {
    let chars: Vec<char> = isbn.chars().collect();
    if chars.len() != 13 {
        return false;
    }
    let mut sum = 0u32;
    for (i, &ch) in chars.iter().enumerate() {
        let Some(d) = ch.to_digit(10) else {
            return false;
        };
        sum += if i % 2 == 0 { d } else { d * 3 };
    }
    sum % 10 == 0
}

/// Compute the expected ISBN-13 check digit from the first 12 digits.
fn compute_isbn13_check_digit(first_12: &str) -> char {
    debug_assert!(first_12.len() == 12);
    let sum: u32 = first_12
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let d = c.to_digit(10).unwrap_or(0);
            if i % 2 == 0 {
                d
            } else {
                d * 3
            }
        })
        .sum();
    let check = (10 - (sum % 10)) % 10;
    char::from_digit(check, 10).unwrap()
}

/// Validate an ISBN-10 checksum.
/// Sum of (digit x position 10..1) mod 11 == 0, where 'X' = 10.
pub(crate) fn validate_isbn10_checksum(isbn: &str) -> bool {
    let chars: Vec<char> = isbn.chars().collect();
    if chars.len() != 10 {
        return false;
    }
    let mut sum = 0u32;
    for (i, &ch) in chars.iter().enumerate() {
        let val = if ch == 'X' || ch == 'x' {
            if i != 9 {
                return false;
            }
            10
        } else {
            let Some(d) = ch.to_digit(10) else {
                return false;
            };
            d
        };
        let weight = 10 - u32::try_from(i).expect("index <= 9");
        sum += val * weight;
    }
    sum % 11 == 0
}

/// Compute the expected ISBN-10 check digit from the first 9 digits.
#[allow(clippy::cast_possible_truncation)] // index max 8, always fits in u32
fn compute_isbn10_check_digit(first_9: &str) -> char {
    debug_assert!(first_9.len() == 9);
    let sum: u32 = first_9
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let d = c.to_digit(10).unwrap_or(0);
            d * (10 - i as u32)
        })
        .sum();
    let check = (11 - (sum % 11)) % 11;
    if check == 10 {
        'X'
    } else {
        char::from_digit(check, 10).unwrap()
    }
}

/// Attempt to convert an ISBN-10 to ISBN-13.
///
/// Validates the input first. Returns `None` if the input is not a valid ISBN-10.
pub fn isbn10_to_isbn13(isbn10: &str) -> Option<String> {
    let normalized: String = isbn10
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .collect::<String>()
        .to_uppercase();

    if normalized.len() != 10 || !validate_isbn10_checksum(&normalized) {
        return None;
    }

    // ISBN-13 = "978" + first 9 digits of ISBN-10 + new check digit
    let base = format!("978{}", &normalized[..9]);
    let check = compute_isbn13_check_digit(&base);
    Some(format!("{base}{check}"))
}

/// Attempt to convert an ISBN-13 (978 prefix) to ISBN-10.
///
/// Only ISBN-13s starting with "978" can be converted. Returns `None` if the
/// input is not a valid ISBN-13 or does not have a 978 prefix.
pub fn isbn13_to_isbn10(isbn13: &str) -> Option<String> {
    let normalized: String = isbn13
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .collect::<String>()
        .to_uppercase();

    if normalized.len() != 13 || !validate_isbn13_checksum(&normalized) {
        return None;
    }

    if !normalized.starts_with("978") {
        return None;
    }

    // ISBN-10 = digits 4-12 of ISBN-13 + new check digit
    let base = &normalized[3..12];
    let check = compute_isbn10_check_digit(base);
    Some(format!("{base}{check}"))
}

/// Normalize an ISBN value: strip whitespace and hyphens, uppercase.
pub fn normalize_isbn(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Normalize an ASIN value: strip whitespace, uppercase.
pub fn normalize_asin(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Convert an ISBN value to normalized ISBN-13 for cross-type comparison.
pub fn to_isbn13(value: &str, id_type: IdentifierType) -> Option<String> {
    match id_type {
        IdentifierType::Isbn13 => Some(normalize_isbn(value)),
        IdentifierType::Isbn10 => isbn10_to_isbn13(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_isbn ───────────────────────────────────────────

    #[test]
    fn valid_isbn13_with_hyphens() {
        let result = validate_isbn("978-0-441-17271-9");
        assert!(result.valid);
        assert_eq!(result.normalized, "9780441172719");
        assert_eq!(result.isbn_type, Some(IsbnType::Isbn13));
        assert_eq!(result.message, "Valid ISBN-13");
    }

    #[test]
    fn valid_isbn10() {
        let result = validate_isbn("0441172717");
        assert!(result.valid);
        assert_eq!(result.normalized, "0441172717");
        assert_eq!(result.isbn_type, Some(IsbnType::Isbn10));
        assert_eq!(result.message, "Valid ISBN-10");
    }

    #[test]
    fn invalid_isbn13_bad_checksum() {
        let result = validate_isbn("978-0-441-17271-0");
        assert!(!result.valid);
        assert!(result.message.contains("checksum failed"));
        assert!(result.message.contains("expected check digit 9"));
        assert!(result.message.contains("got 0"));
    }

    #[test]
    fn invalid_isbn10_bad_checksum() {
        let result = validate_isbn("1234567890");
        assert!(!result.valid);
        assert!(result.message.contains("checksum failed"));
    }

    #[test]
    fn invalid_not_an_isbn() {
        let result = validate_isbn("not-an-isbn");
        assert!(!result.valid);
    }

    #[test]
    fn invalid_empty() {
        let result = validate_isbn("");
        assert!(!result.valid);
        assert!(result.message.contains("empty"));
    }

    #[test]
    fn isbn10_with_x_check_digit() {
        // ISBN-10: 0-306-40615-2 is valid; let's test one with X
        // "080442957X" is a valid ISBN-10 (verify: 0*10 + 8*9 + 0*8 + 4*7 + 4*6 + 2*5 + 9*4 + 5*3 + 7*2 + 10*1)
        let result = validate_isbn("080442957X");
        assert!(result.valid);
        assert_eq!(result.isbn_type, Some(IsbnType::Isbn10));
    }

    #[test]
    fn isbn13_with_spaces() {
        let result = validate_isbn("978 0 441 17271 9");
        assert!(result.valid);
        assert_eq!(result.normalized, "9780441172719");
    }

    // ── isbn10_to_isbn13 ────────────────────────────────────────

    #[test]
    fn convert_isbn10_to_isbn13() {
        assert_eq!(isbn10_to_isbn13("0441172717"), Some("9780441172719".into()));
    }

    #[test]
    fn convert_isbn10_with_hyphens_to_isbn13() {
        assert_eq!(
            isbn10_to_isbn13("0-441-17271-7"),
            Some("9780441172719".into())
        );
    }

    #[test]
    fn convert_invalid_isbn10_returns_none() {
        assert_eq!(isbn10_to_isbn13("1234567890"), None);
    }

    // ── isbn13_to_isbn10 ────────────────────────────────────────

    #[test]
    fn convert_isbn13_to_isbn10() {
        assert_eq!(isbn13_to_isbn10("9780441172719"), Some("0441172717".into()));
    }

    #[test]
    fn convert_isbn13_with_hyphens_to_isbn10() {
        assert_eq!(
            isbn13_to_isbn10("978-0-441-17271-9"),
            Some("0441172717".into())
        );
    }

    #[test]
    fn isbn13_with_979_prefix_cannot_convert() {
        // 979-prefix ISBN-13s cannot be converted to ISBN-10
        // Use a valid 979-prefix: 9791032305690
        let result = validate_isbn("9791032305690");
        assert!(result.valid);
        assert_eq!(isbn13_to_isbn10("9791032305690"), None);
    }

    #[test]
    fn convert_invalid_isbn13_returns_none() {
        assert_eq!(isbn13_to_isbn10("9780441172710"), None);
    }
}
