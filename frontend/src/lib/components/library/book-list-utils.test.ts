import { describe, it, expect } from 'vitest';
import {
  columnToSortField,
  statusConfig,
  formatDate,
  formatAuthors,
  formatSeries,
  formatFormats,
  isAuthorRole
} from './book-list-utils.js';
import {
  createBookSummary,
  createAuthorEntry,
  createSeriesEntry,
  createFileEntry
} from '$lib/test-utils/index.js';

describe('columnToSortField', () => {
  it('maps title to title', () => {
    expect(columnToSortField['title']).toBe('title');
  });

  it('maps added_at to added_at', () => {
    expect(columnToSortField['added_at']).toBe('added_at');
  });

  it('maps metadata_status to metadata_status', () => {
    expect(columnToSortField['metadata_status']).toBe('metadata_status');
  });

  it('maps authors to author', () => {
    expect(columnToSortField['authors']).toBe('author');
  });

  it('maps series to series', () => {
    expect(columnToSortField['series']).toBe('series');
  });

  it('has exactly 5 entries', () => {
    expect(Object.keys(columnToSortField)).toHaveLength(5);
  });
});

describe('statusConfig', () => {
  it('has entry for identified', () => {
    expect(statusConfig['identified']).toBeDefined();
    expect(statusConfig['identified'].label).toBe('Identified');
    expect(statusConfig['identified'].class).toBeTruthy();
  });

  it('has entry for needs_review', () => {
    expect(statusConfig['needs_review']).toBeDefined();
    expect(statusConfig['needs_review'].label).toBe('Needs Review');
    expect(statusConfig['needs_review'].class).toBeTruthy();
  });

  it('has entry for unidentified', () => {
    expect(statusConfig['unidentified']).toBeDefined();
    expect(statusConfig['unidentified'].label).toBe('Unidentified');
    expect(statusConfig['unidentified'].class).toBeTruthy();
  });

  it('each entry has label and class properties', () => {
    for (const key of Object.keys(statusConfig)) {
      expect(statusConfig[key]).toHaveProperty('label');
      expect(statusConfig[key]).toHaveProperty('class');
    }
  });
});

describe('formatDate', () => {
  it('formats an ISO string and returns a string containing the year', () => {
    const result = formatDate('2024-06-15T12:30:00Z');
    expect(typeof result).toBe('string');
    expect(result).toContain('2024');
  });
});

describe('isAuthorRole', () => {
  it('returns true for undefined role', () => {
    expect(isAuthorRole(undefined)).toBe(true);
  });

  it('returns true for null role', () => {
    expect(isAuthorRole(null)).toBe(true);
  });

  it('returns true for empty string role', () => {
    expect(isAuthorRole('')).toBe(true);
  });

  it('returns true for "author" role', () => {
    expect(isAuthorRole('author')).toBe(true);
  });

  it('returns false for "translator"', () => {
    expect(isAuthorRole('translator')).toBe(false);
  });

  it('returns false for "editor"', () => {
    expect(isAuthorRole('editor')).toBe(false);
  });

  it('returns false for "illustrator"', () => {
    expect(isAuthorRole('illustrator')).toBe(false);
  });
});

describe('formatAuthors', () => {
  it('returns comma-separated author names', () => {
    const book = createBookSummary({
      authors: [createAuthorEntry({ name: 'Alice' }), createAuthorEntry({ name: 'Bob' })]
    });
    expect(formatAuthors(book)).toBe('Alice, Bob');
  });

  it('returns empty string when authors is undefined', () => {
    const book = createBookSummary({ authors: undefined });
    expect(formatAuthors(book)).toBe('');
  });

  it('returns empty string when authors array is empty', () => {
    const book = createBookSummary({ authors: [] });
    expect(formatAuthors(book)).toBe('');
  });

  it('excludes non-author contributors', () => {
    const book = createBookSummary({
      authors: [
        createAuthorEntry({ name: 'Alice', role: 'author' }),
        createAuthorEntry({ name: 'Bob', role: 'translator' })
      ]
    });
    expect(formatAuthors(book)).toBe('Alice');
  });

  it('treats empty role as author', () => {
    const book = createBookSummary({
      authors: [createAuthorEntry({ name: 'Alice', role: '' })]
    });
    expect(formatAuthors(book)).toBe('Alice');
  });
});

describe('formatSeries', () => {
  it('returns empty string when no series', () => {
    const book = createBookSummary({ series: [] });
    expect(formatSeries(book)).toBe('');
  });

  it('returns empty string when series is undefined', () => {
    const book = createBookSummary({ series: undefined });
    expect(formatSeries(book)).toBe('');
  });

  it('formats series with position', () => {
    const book = createBookSummary({
      series: [createSeriesEntry({ name: 'Wheel of Time', position: 1 })]
    });
    expect(formatSeries(book)).toBe('Wheel of Time #1');
  });

  it('formats series without position', () => {
    const book = createBookSummary({
      series: [createSeriesEntry({ name: 'Standalone', position: null })]
    });
    expect(formatSeries(book)).toBe('Standalone');
  });
});

describe('formatFormats', () => {
  it('returns uppercase format strings from files', () => {
    const book = createBookSummary({
      files: [createFileEntry({ format: 'epub' }), createFileEntry({ format: 'pdf' })]
    });
    expect(formatFormats(book)).toEqual(['EPUB', 'PDF']);
  });

  it('returns empty array when files is undefined', () => {
    const book = createBookSummary({ files: undefined });
    expect(formatFormats(book)).toEqual([]);
  });

  it('returns empty array when files is empty', () => {
    const book = createBookSummary({ files: [] });
    expect(formatFormats(book)).toEqual([]);
  });

  it('deduplicates formats when multiple files share the same format', () => {
    const book = createBookSummary({
      files: [
        createFileEntry({ format: 'epub' }),
        createFileEntry({ format: 'epub' }),
        createFileEntry({ format: 'pdf' })
      ]
    });
    expect(formatFormats(book)).toEqual(['EPUB', 'PDF']);
  });
});
