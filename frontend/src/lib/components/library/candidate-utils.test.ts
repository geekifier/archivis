import { describe, it, expect } from 'vitest';
import {
  scoreColor,
  formatScore,
  providerColorClass,
  hasChange,
  getExcludedFields
} from './candidate-utils.js';

describe('scoreColor', () => {
  it('returns green for 1.0', () => {
    expect(scoreColor(1.0)).toBe('bg-green-500');
  });

  it('returns green for 0.8 (boundary)', () => {
    expect(scoreColor(0.8)).toBe('bg-green-500');
  });

  it('returns amber for 0.79', () => {
    expect(scoreColor(0.79)).toBe('bg-amber-500');
  });

  it('returns amber for 0.5 (boundary)', () => {
    expect(scoreColor(0.5)).toBe('bg-amber-500');
  });

  it('returns red for 0.49', () => {
    expect(scoreColor(0.49)).toBe('bg-red-500');
  });

  it('returns red for 0', () => {
    expect(scoreColor(0)).toBe('bg-red-500');
  });
});

describe('formatScore', () => {
  it('formats 1.0 as 100%', () => {
    expect(formatScore(1.0)).toBe('100%');
  });

  it('formats 0.856 as 86%', () => {
    expect(formatScore(0.856)).toBe('86%');
  });

  it('formats 0 as 0%', () => {
    expect(formatScore(0)).toBe('0%');
  });

  it('formats 0.5 as 50%', () => {
    expect(formatScore(0.5)).toBe('50%');
  });
});

describe('providerColorClass', () => {
  it('returns blue class for Open Library', () => {
    expect(providerColorClass('Open Library')).toContain('bg-blue-100');
  });

  it('is case-insensitive for Open Library', () => {
    expect(providerColorClass('open library')).toContain('bg-blue-100');
  });

  it('returns purple class for Hardcover', () => {
    expect(providerColorClass('Hardcover')).toContain('bg-purple-100');
  });

  it('is case-insensitive for Hardcover', () => {
    expect(providerColorClass('hardcover')).toContain('bg-purple-100');
  });

  it('returns muted class for unknown providers', () => {
    expect(providerColorClass('Unknown Provider')).toBe('bg-muted text-muted-foreground');
  });
});

describe('hasChange', () => {
  it('returns true when values differ', () => {
    expect(hasChange('new value', 'old value')).toBe(true);
  });

  it('returns false when values are the same', () => {
    expect(hasChange('same', 'same')).toBe(false);
  });

  it('returns false when candidate is null', () => {
    expect(hasChange(null, 'book value')).toBe(false);
  });

  it('returns false when candidate is undefined', () => {
    expect(hasChange(undefined, 'book value')).toBe(false);
  });

  it('returns false when candidate is empty string', () => {
    expect(hasChange('', 'book value')).toBe(false);
  });

  it('returns false when both are null', () => {
    expect(hasChange(null, null)).toBe(false);
  });

  it('returns true when book is null but candidate has value', () => {
    expect(hasChange('new value', null)).toBe(true);
  });
});

describe('getExcludedFields', () => {
  it('returns empty array when no selections exist', () => {
    expect(getExcludedFields({}, 'cand-1')).toEqual([]);
  });

  it('returns empty array when all fields are selected', () => {
    const selections = {
      'cand-1': { title: true, authors: true, description: true }
    };
    expect(getExcludedFields(selections, 'cand-1')).toEqual([]);
  });

  it('returns deselected field names', () => {
    const selections = {
      'cand-1': { title: true, authors: false, description: false }
    };
    const result = getExcludedFields(selections, 'cand-1');
    expect(result).toContain('authors');
    expect(result).toContain('description');
    expect(result).not.toContain('title');
    expect(result).toHaveLength(2);
  });

  it('returns empty array for unknown candidateId', () => {
    const selections = {
      'cand-1': { title: true, authors: false }
    };
    expect(getExcludedFields(selections, 'cand-999')).toEqual([]);
  });
});
