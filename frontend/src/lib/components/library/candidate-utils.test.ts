import { describe, it, expect } from 'vitest';
import {
  scoreColor,
  formatScore,
  hasChange,
  namesMatch,
  getExcludedFields,
  tierColorClass,
  tierLabel
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

describe('namesMatch', () => {
  it('returns true for identical lists', () => {
    expect(namesMatch(['Alice', 'Bob'], ['Alice', 'Bob'])).toBe(true);
  });

  it('returns true for same names in different order', () => {
    expect(namesMatch(['Bob', 'Alice'], ['Alice', 'Bob'])).toBe(true);
  });

  it('is case-insensitive', () => {
    expect(namesMatch(['alice', 'BOB'], ['Alice', 'Bob'])).toBe(true);
  });

  it('trims whitespace', () => {
    expect(namesMatch(['  Alice ', 'Bob'], ['Alice', ' Bob '])).toBe(true);
  });

  it('returns false for different sets', () => {
    expect(namesMatch(['Alice', 'Bob'], ['Alice', 'Charlie'])).toBe(false);
  });

  it('returns false for different sizes', () => {
    expect(namesMatch(['Alice'], ['Alice', 'Bob'])).toBe(false);
  });

  it('returns true for two empty lists', () => {
    expect(namesMatch([], [])).toBe(true);
  });

  it('returns false when one list is empty and the other is not', () => {
    expect(namesMatch([], ['Alice'])).toBe(false);
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

describe('tierColorClass', () => {
    it('returns green class for strong_id_match', () => {
        expect(tierColorClass('strong_id_match')).toContain('bg-green-100');
    });

    it('returns amber class for probable_match', () => {
        expect(tierColorClass('probable_match')).toContain('bg-amber-100');
    });

    it('returns muted class for weak_match', () => {
        expect(tierColorClass('weak_match')).toBe('bg-muted text-muted-foreground');
    });

    it('returns muted class for undefined', () => {
        expect(tierColorClass(undefined)).toBe('bg-muted text-muted-foreground');
    });
});

describe('tierLabel', () => {
    it('returns "Strong ID match" for strong_id_match', () => {
        expect(tierLabel('strong_id_match')).toBe('Strong ID match');
    });

    it('returns "Probable match" for probable_match', () => {
        expect(tierLabel('probable_match')).toBe('Probable match');
    });

    it('returns "Weak match" for weak_match', () => {
        expect(tierLabel('weak_match')).toBe('Weak match');
    });

    it('returns empty string for undefined', () => {
        expect(tierLabel(undefined)).toBe('');
    });

    it('returns raw value for unknown tier', () => {
        expect(tierLabel('custom_tier')).toBe('custom_tier');
    });
});
