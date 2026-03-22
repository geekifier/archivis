import { describe, it, expect, beforeEach } from 'vitest';
import { filters } from './filters.svelte.js';

describe('filters store', () => {
  beforeEach(() => {
    filters.clearFilters();
  });

  it('has correct initial state', () => {
    expect(filters.activeFormat).toBeNull();
    expect(filters.activeStatus).toBeNull();
    expect(filters.hasActiveFilters).toBe(false);
  });

  describe('setFormat', () => {
    it('sets activeFormat and activates filters', () => {
      filters.setFormat('epub');
      expect(filters.activeFormat).toBe('epub');
      expect(filters.hasActiveFilters).toBe(true);
    });

    it('toggles off when setting the same format again', () => {
      filters.setFormat('epub');
      filters.setFormat('epub');
      expect(filters.activeFormat).toBeNull();
      expect(filters.hasActiveFilters).toBe(false);
    });

    it('replaces format when setting a different one', () => {
      filters.setFormat('epub');
      filters.setFormat('pdf');
      expect(filters.activeFormat).toBe('pdf');
    });
  });

  describe('setStatus', () => {
    it('sets activeStatus and activates filters', () => {
      filters.setStatus('identified');
      expect(filters.activeStatus).toBe('identified');
      expect(filters.hasActiveFilters).toBe(true);
    });

    it('toggles off when setting the same status again', () => {
      filters.setStatus('identified');
      filters.setStatus('identified');
      expect(filters.activeStatus).toBeNull();
      expect(filters.hasActiveFilters).toBe(false);
    });
  });

  it('hasActiveFilters is true when both format and status are set', () => {
    filters.setFormat('epub');
    filters.setStatus('identified');
    expect(filters.hasActiveFilters).toBe(true);
  });

  describe('clearFilters', () => {
    it('resets both format and status to null', () => {
      filters.setFormat('epub');
      filters.setStatus('identified');
      filters.clearFilters();
      expect(filters.activeFormat).toBeNull();
      expect(filters.activeStatus).toBeNull();
      expect(filters.hasActiveFilters).toBe(false);
    });
  });

  describe('toFilterState', () => {
    it('returns empty LibraryFilterState when no filters are active', () => {
      const state = filters.toFilterState();
      expect(state.text_query).toBeNull();
      expect(state.format).toBeNull();
      expect(state.metadata_status).toBeNull();
      expect(state.author_id).toBeNull();
      expect(state.tag_ids).toEqual([]);
      expect(state.tag_match).toBe('any');
      expect(state.isbn).toBeNull();
    });

    it('includes text query when provided', () => {
      const state = filters.toFilterState('hello world');
      expect(state.text_query).toBe('hello world');
    });

    it('trims text query and returns null for whitespace-only', () => {
      expect(filters.toFilterState('   ').text_query).toBeNull();
      expect(filters.toFilterState('').text_query).toBeNull();
      expect(filters.toFilterState().text_query).toBeNull();
    });

    it('includes all active filters', () => {
      filters.setFormat('epub');
      filters.setStatus('identified');
      filters.setLanguage('en');
      filters.setTrusted(true);
      filters.setYearMin(2020);
      filters.setYearMax(2025);
      filters.setHasCover(true);

      const state = filters.toFilterState('search');
      expect(state.text_query).toBe('search');
      expect(state.format).toBe('epub');
      expect(state.metadata_status).toBe('identified');
      expect(state.language).toBe('en');
      expect(state.trusted).toBe(true);
      expect(state.year_min).toBe(2020);
      expect(state.year_max).toBe(2025);
      expect(state.has_cover).toBe(true);
    });

    it('maps relation filters to IDs', () => {
      filters.setAuthor({ id: 'author-1', name: 'Brandon Sanderson' });
      filters.setSeries({ id: 'series-1', name: 'Stormlight' });
      filters.setPublisher({ id: 'pub-1', name: 'Tor' });

      const state = filters.toFilterState();
      expect(state.author_id).toBe('author-1');
      expect(state.series_id).toBe('series-1');
      expect(state.publisher_id).toBe('pub-1');
    });

    it('maps tags to tag_ids array', () => {
      filters.addTag({ id: 'tag-a', name: 'Fiction', category: null });
      filters.addTag({ id: 'tag-b', name: 'Fantasy', category: null });
      filters.setTagMatch('all');

      const state = filters.toFilterState();
      expect(state.tag_ids).toEqual(['tag-a', 'tag-b']);
      expect(state.tag_match).toBe('all');
    });

    it('maps identifier type/value to the correct field', () => {
      filters.setIdentifier('isbn', '978-0-13-468599-1');
      const isbn = filters.toFilterState();
      expect(isbn.isbn).toBe('978-0-13-468599-1');
      expect(isbn.asin).toBeNull();

      filters.setIdentifier('asin', 'B08N5WRWNW');
      const asin = filters.toFilterState();
      expect(asin.asin).toBe('B08N5WRWNW');
      expect(asin.isbn).toBeNull();
    });

    it('snapshotKey changes when filters change', () => {
      const snap1 = filters.snapshotKey();
      filters.setFormat('pdf');
      const snap2 = filters.snapshotKey();
      expect(snap1).not.toBe(snap2);
    });
  });
});
