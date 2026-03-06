/**
 * Client-side ISBN validation and conversion utilities.
 *
 * All validation runs entirely in the browser with no API calls,
 * providing instant feedback on every keystroke.
 */

export interface IsbnValidation {
  valid: boolean;
  normalized: string;
  isbnType: 'isbn10' | 'isbn13' | null;
  message: string;
  isbn13Equivalent: string | null;
  isbn10Equivalent: string | null;
}

/**
 * Validate and normalize an ISBN string entirely client-side.
 * Strips hyphens, spaces, and leading/trailing whitespace.
 * Validates length and checksum. Provides cross-format equivalents.
 */
export function validateIsbn(input: string): IsbnValidation {
  const normalized = input.replace(/[\s-]/g, '').toUpperCase();

  if (normalized.length === 0) {
    return {
      valid: false,
      normalized,
      isbnType: null,
      message: 'ISBN is empty',
      isbn13Equivalent: null,
      isbn10Equivalent: null
    };
  }

  if (normalized.length === 13) {
    // Validate ISBN-13: all digits
    if (!/^\d{13}$/.test(normalized)) {
      return {
        valid: false,
        normalized,
        isbnType: null,
        message: 'ISBN-13 must contain only digits',
        isbn13Equivalent: null,
        isbn10Equivalent: null
      };
    }

    if (!validateIsbn13Checksum(normalized)) {
      const expected = computeIsbn13CheckDigit(normalized.slice(0, 12));
      return {
        valid: false,
        normalized,
        isbnType: null,
        message: `ISBN-13 checksum failed: expected check digit ${expected}, got ${normalized[12]}`,
        isbn13Equivalent: null,
        isbn10Equivalent: null
      };
    }

    return {
      valid: true,
      normalized,
      isbnType: 'isbn13',
      message: 'Valid ISBN-13',
      isbn13Equivalent: normalized,
      isbn10Equivalent: isbn13ToIsbn10(normalized)
    };
  }

  if (normalized.length === 10) {
    // Validate ISBN-10: 9 digits + check digit (digit or X)
    if (!/^\d{9}[\dX]$/.test(normalized)) {
      return {
        valid: false,
        normalized,
        isbnType: null,
        message: 'ISBN-10 must contain 9 digits followed by a digit or X',
        isbn13Equivalent: null,
        isbn10Equivalent: null
      };
    }

    if (!validateIsbn10Checksum(normalized)) {
      const expected = computeIsbn10CheckDigit(normalized.slice(0, 9));
      return {
        valid: false,
        normalized,
        isbnType: null,
        message: `ISBN-10 checksum failed: expected check digit ${expected}, got ${normalized[9]}`,
        isbn13Equivalent: null,
        isbn10Equivalent: null
      };
    }

    return {
      valid: true,
      normalized,
      isbnType: 'isbn10',
      message: 'Valid ISBN-10',
      isbn13Equivalent: isbn10ToIsbn13(normalized),
      isbn10Equivalent: normalized
    };
  }

  return {
    valid: false,
    normalized,
    isbnType: null,
    message: `Invalid length: expected 10 or 13 characters, got ${normalized.length}`,
    isbn13Equivalent: null,
    isbn10Equivalent: null
  };
}

/** ISBN-13 checksum: alternating weights 1,3 -- sum mod 10 == 0. */
function validateIsbn13Checksum(digits: string): boolean {
  let sum = 0;
  for (let i = 0; i < 13; i++) {
    const digit = Number(digits[i]);
    sum += i % 2 === 0 ? digit : digit * 3;
  }
  return sum % 10 === 0;
}

/** ISBN-10 checksum: weights 10..1, 'X' = 10 -- sum mod 11 == 0. */
function validateIsbn10Checksum(digits: string): boolean {
  let sum = 0;
  for (let i = 0; i < 10; i++) {
    const char = digits[i];
    const value = char === 'X' ? 10 : Number(char);
    sum += value * (10 - i);
  }
  return sum % 11 === 0;
}

/** Compute the check digit for an ISBN-13 given the first 12 digits. */
function computeIsbn13CheckDigit(first12: string): string {
  let sum = 0;
  for (let i = 0; i < 12; i++) {
    const digit = Number(first12[i]);
    sum += i % 2 === 0 ? digit : digit * 3;
  }
  const check = (10 - (sum % 10)) % 10;
  return String(check);
}

/** Compute the check digit for an ISBN-10 given the first 9 digits. */
function computeIsbn10CheckDigit(first9: string): string {
  let sum = 0;
  for (let i = 0; i < 9; i++) {
    sum += Number(first9[i]) * (10 - i);
  }
  const check = (11 - (sum % 11)) % 11;
  return check === 10 ? 'X' : String(check);
}

/** Convert a valid ISBN-10 to ISBN-13 (prefix 978, recalculate check digit). */
export function isbn10ToIsbn13(isbn10: string): string | null {
  const normalized = isbn10.replace(/[\s-]/g, '').toUpperCase();
  if (normalized.length !== 10) return null;
  if (!/^\d{9}[\dX]$/.test(normalized)) return null;

  const prefix = '978' + normalized.slice(0, 9);
  const checkDigit = computeIsbn13CheckDigit(prefix);
  return prefix + checkDigit;
}

/** Convert a valid ISBN-13 with 978 prefix to ISBN-10 (strip prefix, recalculate check digit). */
export function isbn13ToIsbn10(isbn13: string): string | null {
  const normalized = isbn13.replace(/[\s-]/g, '');
  if (normalized.length !== 13) return null;
  if (!/^\d{13}$/.test(normalized)) return null;
  if (!normalized.startsWith('978')) return null;

  const body = normalized.slice(3, 12);
  const checkDigit = computeIsbn10CheckDigit(body);
  return body + checkDigit;
}
