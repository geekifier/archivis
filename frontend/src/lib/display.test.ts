import { describe, it, expect } from 'vitest';
import {
  providerLabel,
  providerColorClass,
  identifierLabel,
  identifierColorClass,
  identifierTypeOptions,
  sectionLabel
} from './display.js';

describe('providerLabel', () => {
  it('maps open_library to Open Library', () => {
    expect(providerLabel('open_library')).toBe('Open Library');
  });

  it('maps hardcover to Hardcover', () => {
    expect(providerLabel('hardcover')).toBe('Hardcover');
  });

  it('maps google_books to Google Books', () => {
    expect(providerLabel('google_books')).toBe('Google Books');
  });

  it('maps loc to Library of Congress', () => {
    expect(providerLabel('loc')).toBe('Library of Congress');
  });

  it('returns slug as fallback for unknown provider', () => {
    expect(providerLabel('unknown_provider')).toBe('unknown_provider');
  });
});

describe('providerColorClass', () => {
  it('returns blue class for open_library', () => {
    expect(providerColorClass('open_library')).toContain('bg-blue-100');
  });

  it('returns purple class for hardcover', () => {
    expect(providerColorClass('hardcover')).toContain('bg-purple-100');
  });

  it('returns green class for google_books', () => {
    expect(providerColorClass('google_books')).toContain('bg-green-100');
  });

  it('returns orange class for loc', () => {
    expect(providerColorClass('loc')).toContain('bg-orange-500');
  });

  it('returns muted class for unknown provider', () => {
    expect(providerColorClass('unknown')).toBe('bg-muted text-muted-foreground');
  });
});

describe('identifierLabel', () => {
  it('maps isbn13 to ISBN-13', () => {
    expect(identifierLabel('isbn13')).toBe('ISBN-13');
  });

  it('maps isbn10 to ISBN-10', () => {
    expect(identifierLabel('isbn10')).toBe('ISBN-10');
  });

  it('maps asin to ASIN', () => {
    expect(identifierLabel('asin')).toBe('ASIN');
  });

  it('maps google_books to Google Books', () => {
    expect(identifierLabel('google_books')).toBe('Google Books');
  });

  it('maps open_library to Open Library', () => {
    expect(identifierLabel('open_library')).toBe('Open Library');
  });

  it('maps hardcover to Hardcover', () => {
    expect(identifierLabel('hardcover')).toBe('Hardcover');
  });

  it('maps lccn to LCCN', () => {
    expect(identifierLabel('lccn')).toBe('LCCN');
  });

  it('returns slug as fallback for unknown type', () => {
    expect(identifierLabel('custom_id')).toBe('custom_id');
  });
});

describe('identifierColorClass', () => {
  it('returns blue class for isbn13', () => {
    expect(identifierColorClass('isbn13')).toContain('bg-blue-100');
  });

  it('returns orange class for asin', () => {
    expect(identifierColorClass('asin')).toContain('bg-orange-100');
  });

  it('returns green class for google_books', () => {
    expect(identifierColorClass('google_books')).toContain('bg-green-100');
  });

  it('returns indigo class for open_library', () => {
    expect(identifierColorClass('open_library')).toContain('bg-indigo-100');
  });

  it('returns purple class for hardcover', () => {
    expect(identifierColorClass('hardcover')).toContain('bg-purple-100');
  });

  it('returns orange class for lccn', () => {
    expect(identifierColorClass('lccn')).toContain('bg-orange-500');
  });

  it('returns muted class for unknown type', () => {
    expect(identifierColorClass('unknown')).toBe('bg-muted text-muted-foreground');
  });
});

describe('identifierTypeOptions', () => {
  it('returns all identifier types as value/label pairs', () => {
    const options = identifierTypeOptions();
    expect(options).toHaveLength(7);
    expect(options).toContainEqual({ value: 'isbn13', label: 'ISBN-13' });
    expect(options).toContainEqual({ value: 'lccn', label: 'LCCN' });
  });
});

describe('sectionLabel', () => {
  it('title-cases single-word sections', () => {
    expect(sectionLabel('server')).toBe('Server');
    expect(sectionLabel('auth')).toBe('Auth');
    expect(sectionLabel('import')).toBe('Import');
  });

  it('title-cases multi-word section keys', () => {
    expect(sectionLabel('isbn_scan')).toBe('Isbn Scan');
  });

  it('joins dotted sections with a separator', () => {
    expect(sectionLabel('auth.proxy')).toBe('Auth · Proxy');
  });

  it('derives metadata subsection labels via providerLabel', () => {
    expect(sectionLabel('metadata.open_library')).toBe('Open Library');
    expect(sectionLabel('metadata.hardcover')).toBe('Hardcover');
    expect(sectionLabel('metadata.google_books')).toBe('Google Books');
    expect(sectionLabel('metadata.loc')).toBe('Library of Congress');
  });

  it('falls back to a title-cased derivation for unknown sections', () => {
    expect(sectionLabel('unknown_section')).toBe('Unknown Section');
  });
});
