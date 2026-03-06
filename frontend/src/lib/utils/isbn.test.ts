import { describe, it, expect } from 'vitest';
import { validateIsbn, isbn10ToIsbn13, isbn13ToIsbn10 } from './isbn.js';

describe('validateIsbn', () => {
  it('validates a correct ISBN-13 with hyphens', () => {
    const result = validateIsbn('978-0-441-17271-9');
    expect(result.valid).toBe(true);
    expect(result.isbnType).toBe('isbn13');
    expect(result.normalized).toBe('9780441172719');
    expect(result.message).toBe('Valid ISBN-13');
    expect(result.isbn13Equivalent).toBe('9780441172719');
    expect(result.isbn10Equivalent).toBe('0441172717');
  });

  it('validates a correct ISBN-10', () => {
    const result = validateIsbn('0441172717');
    expect(result.valid).toBe(true);
    expect(result.isbnType).toBe('isbn10');
    expect(result.normalized).toBe('0441172717');
    expect(result.message).toBe('Valid ISBN-10');
    expect(result.isbn13Equivalent).toBe('9780441172719');
  });

  it('rejects ISBN-13 with bad checksum', () => {
    const result = validateIsbn('978-0-441-17271-0');
    expect(result.valid).toBe(false);
    expect(result.isbnType).toBeNull();
    expect(result.message).toContain('checksum failed');
    expect(result.message).toContain('expected check digit 9');
    expect(result.message).toContain('got 0');
  });

  it('rejects ISBN-10 with bad checksum', () => {
    const result = validateIsbn('1234567890');
    expect(result.valid).toBe(false);
    expect(result.isbnType).toBeNull();
    expect(result.message).toContain('checksum failed');
  });

  it('rejects non-ISBN string', () => {
    const result = validateIsbn('not-an-isbn');
    expect(result.valid).toBe(false);
    expect(result.isbnType).toBeNull();
  });

  it('rejects empty string', () => {
    const result = validateIsbn('');
    expect(result.valid).toBe(false);
    expect(result.message).toBe('ISBN is empty');
  });

  it('handles ISBN-10 with X check digit', () => {
    // 080442957X is a valid ISBN-10
    const result = validateIsbn('080442957X');
    expect(result.valid).toBe(true);
    expect(result.isbnType).toBe('isbn10');
    expect(result.normalized).toBe('080442957X');
  });

  it('handles lowercase x in ISBN-10', () => {
    const result = validateIsbn('080442957x');
    expect(result.valid).toBe(true);
    expect(result.isbnType).toBe('isbn10');
    expect(result.normalized).toBe('080442957X');
  });

  it('rejects ISBN-13 with non-digit characters', () => {
    const result = validateIsbn('978044117271X');
    expect(result.valid).toBe(false);
    expect(result.message).toContain('must contain only digits');
  });

  it('rejects wrong length', () => {
    const result = validateIsbn('12345');
    expect(result.valid).toBe(false);
    expect(result.message).toContain('Invalid length');
    expect(result.message).toContain('got 5');
  });

  it('strips spaces during normalization', () => {
    const result = validateIsbn('978 0 441 17271 9');
    expect(result.valid).toBe(true);
    expect(result.normalized).toBe('9780441172719');
  });

  it('provides isbn10 equivalent for valid ISBN-13 with 978 prefix', () => {
    const result = validateIsbn('9780441172719');
    expect(result.valid).toBe(true);
    expect(result.isbn10Equivalent).toBe('0441172717');
  });

  it('returns null isbn10 equivalent for ISBN-13 with 979 prefix', () => {
    // 9791034779857 is a valid ISBN-13 with 979 prefix
    const result = validateIsbn('9791034779857');
    expect(result.valid).toBe(true);
    expect(result.isbn10Equivalent).toBeNull();
  });
});

describe('isbn10ToIsbn13', () => {
  it('converts 0441172717 to 9780441172719', () => {
    expect(isbn10ToIsbn13('0441172717')).toBe('9780441172719');
  });

  it('converts ISBN-10 with X check digit', () => {
    expect(isbn10ToIsbn13('080442957X')).toBe('9780804429573');
  });

  it('returns null for invalid length', () => {
    expect(isbn10ToIsbn13('12345')).toBeNull();
  });

  it('returns null for non-ISBN string', () => {
    expect(isbn10ToIsbn13('abcdefghij')).toBeNull();
  });

  it('handles hyphens in input', () => {
    expect(isbn10ToIsbn13('0-441-17271-7')).toBe('9780441172719');
  });
});

describe('isbn13ToIsbn10', () => {
  it('converts 9780441172719 to 0441172717', () => {
    expect(isbn13ToIsbn10('9780441172719')).toBe('0441172717');
  });

  it('returns null for ISBN-13 with 979 prefix', () => {
    expect(isbn13ToIsbn10('9791034779857')).toBeNull();
  });

  it('returns null for invalid length', () => {
    expect(isbn13ToIsbn10('12345')).toBeNull();
  });

  it('returns null for non-digit string', () => {
    expect(isbn13ToIsbn10('978044117271X')).toBeNull();
  });

  it('handles hyphens in input', () => {
    expect(isbn13ToIsbn10('978-0-441-17271-9')).toBe('0441172717');
  });

  it('produces ISBN-10 with X check digit when needed', () => {
    // 9780804429573 -> 080442957X
    expect(isbn13ToIsbn10('9780804429573')).toBe('080442957X');
  });
});
