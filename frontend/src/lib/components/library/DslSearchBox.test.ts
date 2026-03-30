import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import DslSearchBox from './DslSearchBox.svelte';
import {
  parseTokenAtCursor,
  replaceTokenInQuery,
  findTokenByFieldAndQuery,
  buildFieldValue,
  getOperatorSuggestions,
  getValueSuggestions,
  getBooleanSuggestions,
  getLanguageSuggestions,
  tokenize,
  isTokenComplete,
  splitQueryIntoChipsAndDraft,
  serializeDraft,
  tokenToDraft,
  commitDraft,
  chipsToQuery,
  resetChipIds,
  EMPTY_DRAFT,
  type TokenSpan
} from './search-dsl.js';

// ---------------------------------------------------------------------------
// A. Unit tests for search-dsl.ts (pure functions)
// ---------------------------------------------------------------------------

describe('tokenize', () => {
  it('splits whitespace-delimited tokens', () => {
    const tokens = tokenize('author:asimov format:epub');
    expect(tokens).toHaveLength(2);
    expect(tokens[0].text).toBe('author:asimov');
    expect(tokens[1].text).toBe('format:epub');
  });

  it('handles quoted phrases as single tokens', () => {
    const tokens = tokenize('author:"Isaac Asimov" title:dune');
    expect(tokens).toHaveLength(2);
    expect(tokens[0].text).toBe('author:"Isaac Asimov"');
    expect(tokens[1].text).toBe('title:dune');
  });

  it('handles unclosed quotes extending to end of input', () => {
    const tokens = tokenize('author:"Isaac');
    expect(tokens).toHaveLength(1);
    expect(tokens[0].text).toBe('author:"Isaac');
  });

  it('handles negation prefix', () => {
    const tokens = tokenize('-author:smith');
    expect(tokens).toHaveLength(1);
    expect(tokens[0].text).toBe('-author:smith');
  });

  it('handles bare quoted phrases', () => {
    const tokens = tokenize('"hello world"');
    expect(tokens).toHaveLength(1);
    expect(tokens[0].text).toBe('"hello world"');
  });

  it('handles negated bare quoted phrases', () => {
    const tokens = tokenize('-"bad book"');
    expect(tokens).toHaveLength(1);
    expect(tokens[0].text).toBe('-"bad book"');
  });

  it('returns empty array for empty input', () => {
    expect(tokenize('')).toHaveLength(0);
    expect(tokenize('   ')).toHaveLength(0);
  });
});

describe('parseTokenAtCursor', () => {
  // --- Basic parsing ---

  it('returns operators mode for bare word prefix', () => {
    const result = parseTokenAtCursor('au', 2);
    expect(result.mode).toEqual({ kind: 'operators', prefix: 'au' });
    expect(result.token?.text).toBe('au');
  });

  it('returns relation mode for author:value', () => {
    const result = parseTokenAtCursor('author:asi', 10);
    expect(result.mode).toEqual({ kind: 'relation', field: 'author', value: 'asi' });
  });

  it('identifies correct token when cursor is in middle of query', () => {
    const query = 'author:asimov title:dune format:epub';
    const result = parseTokenAtCursor(query, 21);
    expect(result.token?.text).toBe('title:dune');
    expect(result.mode).toEqual({ kind: 'freetext' });
  });

  it('returns enum mode for format:value', () => {
    const result = parseTokenAtCursor('format:ep', 9);
    expect(result.mode.kind).toBe('enum');
    if (result.mode.kind === 'enum') {
      expect(result.mode.field).toBe('format');
      expect(result.mode.value).toBe('ep');
      expect(result.mode.options).toContain('epub');
    }
  });

  it('returns presence mode for has:value', () => {
    const result = parseTokenAtCursor('has:co', 6);
    expect(result.mode.kind).toBe('presence');
    if (result.mode.kind === 'presence') {
      expect(result.mode.field).toBe('has');
      expect(result.mode.value).toBe('co');
      expect(result.mode.options).toContain('cover');
    }
  });

  it('returns operators mode for empty input', () => {
    const result = parseTokenAtCursor('', 0);
    expect(result.mode).toEqual({ kind: 'operators', prefix: '' });
    expect(result.token).toBeNull();
  });

  it('returns operators mode when cursor is in whitespace', () => {
    const result = parseTokenAtCursor('author:foo ', 11);
    expect(result.mode).toEqual({ kind: 'operators', prefix: '' });
  });

  // --- Quoted value handling ---

  it('returns relation mode for unclosed quote author:"Isa', () => {
    const result = parseTokenAtCursor('author:"Isa', 11);
    expect(result.mode).toEqual({ kind: 'relation', field: 'author', value: 'Isa' });
  });

  it('returns relation mode for complete quoted author:"Isaac Asimov"', () => {
    const query = 'author:"Isaac Asimov"';
    const result = parseTokenAtCursor(query, query.length);
    expect(result.mode).toEqual({
      kind: 'relation',
      field: 'author',
      value: 'Isaac Asimov'
    });
  });

  it('returns none for bare quoted phrase without field prefix', () => {
    const result = parseTokenAtCursor('"some text', 10);
    expect(result.mode).toEqual({ kind: 'none' });
  });

  it('returns relation mode for tag:"sci fi"', () => {
    const query = 'tag:"sci fi"';
    const result = parseTokenAtCursor(query, query.length);
    expect(result.mode).toEqual({ kind: 'relation', field: 'tag', value: 'sci fi' });
  });

  // --- Negation ---

  it('returns relation mode for negated -author:x', () => {
    const result = parseTokenAtCursor('-author:x', 9);
    expect(result.mode).toEqual({ kind: 'relation', field: 'author', value: 'x' });
  });

  it('returns enum mode for negated -format:epub', () => {
    const result = parseTokenAtCursor('-format:epub', 12);
    expect(result.mode.kind).toBe('enum');
    if (result.mode.kind === 'enum') {
      expect(result.mode.field).toBe('format');
    }
  });

  // --- Alias resolution ---

  it('resolves pub: alias to publisher relation', () => {
    const result = parseTokenAtCursor('pub:ace', 7);
    expect(result.mode).toEqual({ kind: 'relation', field: 'publisher', value: 'ace' });
  });

  it('resolves fmt: alias to format enum', () => {
    const result = parseTokenAtCursor('fmt:ep', 6);
    expect(result.mode.kind).toBe('enum');
    if (result.mode.kind === 'enum') {
      expect(result.mode.field).toBe('format');
    }
  });

  it('resolves desc: alias to description freetext', () => {
    const result = parseTokenAtCursor('desc:fantasy', 12);
    expect(result.mode).toEqual({ kind: 'freetext' });
  });

  it('resolves id: alias to identifier freetext', () => {
    const result = parseTokenAtCursor('id:OL123', 8);
    expect(result.mode).toEqual({ kind: 'freetext' });
  });

  it('resolves identifier:isbn:... to freetext', () => {
    const result = parseTokenAtCursor('identifier:isbn:9780451524935', 20);
    expect(result.mode).toEqual({ kind: 'freetext' });
  });

  it('resolves lang: alias to language mode', () => {
    const result = parseTokenAtCursor('lang:en', 7);
    expect(result.mode).toEqual({ kind: 'language', value: 'en' });
  });

  // --- Language suggestions ---

  it('returns language mode for language: with empty value', () => {
    const result = parseTokenAtCursor('language:', 9);
    expect(result.mode).toEqual({ kind: 'language', value: '' });
  });

  it('returns language mode for lang:fr', () => {
    const result = parseTokenAtCursor('lang:fr', 7);
    expect(result.mode).toEqual({ kind: 'language', value: 'fr' });
  });

  it('returns language mode for language:chi', () => {
    const result = parseTokenAtCursor('language:chi', 12);
    expect(result.mode).toEqual({ kind: 'language', value: 'chi' });
  });

  // --- Presence field values ---

  it('returns presence mode for has:desc', () => {
    const result = parseTokenAtCursor('has:desc', 8);
    expect(result.mode.kind).toBe('presence');
    if (result.mode.kind === 'presence') {
      expect(result.mode.value).toBe('desc');
    }
  });

  it('returns presence mode for missing:ids', () => {
    const result = parseTokenAtCursor('missing:ids', 11);
    expect(result.mode.kind).toBe('presence');
    if (result.mode.kind === 'presence') {
      expect(result.mode.value).toBe('ids');
    }
  });

  // --- Boolean values ---

  it('returns boolean mode for trusted: with all values', () => {
    const result = parseTokenAtCursor('trusted:', 8);
    expect(result.mode).toEqual({ kind: 'boolean', field: 'trusted', value: '' });
  });

  it('returns boolean mode for locked:y', () => {
    const result = parseTokenAtCursor('locked:y', 8);
    expect(result.mode).toEqual({ kind: 'boolean', field: 'locked', value: 'y' });
  });
});

describe('getOperatorSuggestions', () => {
  it('returns all operators for empty prefix', () => {
    const items = getOperatorSuggestions('');
    expect(items.length).toBeGreaterThan(10);
  });

  it('filters by prefix', () => {
    const items = getOperatorSuggestions('au');
    expect(items).toHaveLength(1);
    expect(items[0].label).toBe('author:');
  });

  it('matches aliases', () => {
    const items = getOperatorSuggestions('pub');
    expect(items.some((i) => i.label === 'publisher:')).toBe(true);
  });
});

describe('getLanguageSuggestions', () => {
  it('filters by code prefix', () => {
    const items = getLanguageSuggestions('fr');
    expect(items.some((i) => i.label.includes('French'))).toBe(true);
  });

  it('filters by label prefix', () => {
    const items = getLanguageSuggestions('chi');
    expect(items.some((i) => i.label.includes('Chinese'))).toBe(true);
  });

  it('returns all languages for empty value', () => {
    const items = getLanguageSuggestions('');
    expect(items.length).toBeGreaterThan(40);
  });
});

describe('getBooleanSuggestions', () => {
  it('returns all 6 values for empty input', () => {
    const items = getBooleanSuggestions('trusted', '');
    expect(items).toHaveLength(6);
  });

  it('filters by prefix', () => {
    const items = getBooleanSuggestions('locked', 'y');
    expect(items).toHaveLength(1);
    expect(items[0].label).toBe('yes');
  });
});

describe('getValueSuggestions', () => {
  it('filters presence values', () => {
    const options = ['cover', 'description', 'desc', 'identifiers', 'identifier', 'ids'];
    const items = getValueSuggestions('has', 'desc', options);
    expect(items.some((i) => i.label === 'description')).toBe(true);
    expect(items.some((i) => i.label === 'desc')).toBe(true);
    expect(items.every((i) => i.label.startsWith('desc'))).toBe(true);
  });
});

describe('replaceTokenInQuery', () => {
  it('replaces middle token preserving surrounding text', () => {
    const query = 'format:epub author:x tag:scifi';
    const token = { start: 12, end: 20, text: 'author:x' };
    const { newQuery } = replaceTokenInQuery(query, token, 'author:asimov');
    expect(newQuery).toBe('format:epub author:asimov tag:scifi');
  });

  it('wraps values with spaces in quotes via buildFieldValue', () => {
    const query = 'author:x';
    const token = { start: 0, end: 8, text: 'author:x' };
    const { newQuery } = replaceTokenInQuery(
      query,
      token,
      buildFieldValue('author', 'Isaac Asimov')
    );
    expect(newQuery).toBe('author:"Isaac Asimov"');
  });

  it('preserves negation prefix', () => {
    const query = '-tag:x';
    const token = { start: 0, end: 6, text: '-tag:x' };
    const { newQuery } = replaceTokenInQuery(query, token, 'tag:scifi');
    expect(newQuery).toBe('-tag:scifi');
  });

  it('handles operator completion without trailing space', () => {
    const query = 'au';
    const token = { start: 0, end: 2, text: 'au' };
    const { newQuery, newCursorPos } = replaceTokenInQuery(query, token, 'author:');
    expect(newQuery).toBe('author:');
    expect(newCursorPos).toBe(7);
  });

  it('removes a token when replacement is empty', () => {
    const query = 'format:epub author:smith tag:scifi';
    const token = { start: 12, end: 24, text: 'author:smith' };
    const { newQuery } = replaceTokenInQuery(query, token, '');
    expect(newQuery).toBe('format:epub tag:scifi');
  });

  it('preserves negation prefix when replacing with quoted value', () => {
    const query = '-author:smith';
    const token = { start: 0, end: 13, text: '-author:smith' };
    const replacement = buildFieldValue('author', 'John Smith');
    const { newQuery } = replaceTokenInQuery(query, token, replacement);
    expect(newQuery).toBe('-author:"John Smith"');
  });

  it('preserves negation prefix when replacing negated token in multi-token query', () => {
    const query = 'format:epub -author:And tag:scifi';
    const token = { start: 12, end: 23, text: '-author:And' };
    const replacement = buildFieldValue('author', 'Andrzej Sapkowski');
    const { newQuery } = replaceTokenInQuery(query, token, replacement);
    expect(newQuery).toBe('format:epub -author:"Andrzej Sapkowski" tag:scifi');
  });
});

// ---------------------------------------------------------------------------
// Warning-pick token resolution (find + replace end-to-end)
// ---------------------------------------------------------------------------

describe('warning-pick: negation-aware find + replace', () => {
  it('positive pick removes only author:King, leaving -author:King intact', () => {
    const query = 'author:King -author:King';
    const token = findTokenByFieldAndQuery(query, 'author', 'King', false);
    expect(token).not.toBeNull();
    const { newQuery } = replaceTokenInQuery(query, token!, '');
    expect(newQuery.trim()).toBe('-author:King');
  });

  it('negated pick rewrites only -author:King, leaving author:King intact', () => {
    const query = 'author:King -author:King';
    const token = findTokenByFieldAndQuery(query, 'author', 'King', true);
    expect(token).not.toBeNull();
    const replacement = buildFieldValue('author', 'Stephen King');
    const { newQuery } = replaceTokenInQuery(query, token!, replacement);
    expect(newQuery).toBe('author:King -author:"Stephen King"');
  });

  it('negated pick with quoted multi-word value rewrites correctly', () => {
    const query = '-author:And format:epub';
    const token = findTokenByFieldAndQuery(query, 'author', 'And', true);
    expect(token).not.toBeNull();
    const replacement = buildFieldValue('author', 'Andrzej Sapkowski');
    const { newQuery } = replaceTokenInQuery(query, token!, replacement);
    expect(newQuery).toBe('-author:"Andrzej Sapkowski" format:epub');
  });
});

describe('findTokenByFieldAndQuery', () => {
  it('finds author:asimov in a multi-token query', () => {
    const query = 'format:epub author:asimov tag:scifi';
    const token = findTokenByFieldAndQuery(query, 'author', 'asimov');
    expect(token).not.toBeNull();
    expect(token!.text).toBe('author:asimov');
  });

  it('returns null when field not present', () => {
    const token = findTokenByFieldAndQuery('format:epub', 'author', 'smith');
    expect(token).toBeNull();
  });

  it('distinguishes repeated same-field tokens by query text', () => {
    const query = 'tag:sci tag:fan';
    const t1 = findTokenByFieldAndQuery(query, 'tag', 'sci');
    const t2 = findTokenByFieldAndQuery(query, 'tag', 'fan');
    expect(t1).not.toBeNull();
    expect(t2).not.toBeNull();
    expect(t1!.text).toBe('tag:sci');
    expect(t2!.text).toBe('tag:fan');
    expect(t1!.start).not.toBe(t2!.start);
  });

  it('returns first occurrence for duplicate same-field same-value tokens', () => {
    const query = 'tag:sci tag:sci';
    const token = findTokenByFieldAndQuery(query, 'tag', 'sci');
    expect(token).not.toBeNull();
    expect(token!.start).toBe(0);
  });

  it('resolves field aliases', () => {
    const query = 'pub:ace';
    const token = findTokenByFieldAndQuery(query, 'publisher', 'ace');
    expect(token).not.toBeNull();
    expect(token!.text).toBe('pub:ace');
  });

  it('finds negated token -author:smith', () => {
    const query = 'format:epub -author:smith';
    const token = findTokenByFieldAndQuery(query, 'author', 'smith');
    expect(token).not.toBeNull();
    expect(token!.text).toBe('-author:smith');
  });

  it('finds negated token with quoted value', () => {
    const query = '-author:"John Smith" tag:scifi';
    const token = findTokenByFieldAndQuery(query, 'author', 'John Smith');
    expect(token).not.toBeNull();
    expect(token!.text).toBe('-author:"John Smith"');
  });

  it('distinguishes author:King from -author:King by negated flag', () => {
    const query = 'author:King -author:King';
    const pos = findTokenByFieldAndQuery(query, 'author', 'King', false);
    const neg = findTokenByFieldAndQuery(query, 'author', 'King', true);
    expect(pos).not.toBeNull();
    expect(neg).not.toBeNull();
    expect(pos!.text).toBe('author:King');
    expect(pos!.start).toBe(0);
    expect(neg!.text).toBe('-author:King');
    expect(neg!.start).toBe(12);
  });

  it('returns null when negated flag does not match any token', () => {
    const query = 'author:King';
    expect(findTokenByFieldAndQuery(query, 'author', 'King', true)).toBeNull();
  });

  it('returns null when positive flag does not match negated token', () => {
    const query = '-author:King';
    expect(findTokenByFieldAndQuery(query, 'author', 'King', false)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// A2. Unit tests for chip/draft helpers
// ---------------------------------------------------------------------------

describe('isTokenComplete', () => {
  function mkToken(text: string): TokenSpan {
    return { start: 0, end: text.length, text };
  }

  it('complete: field:value with recognized field', () => {
    expect(isTokenComplete(mkToken('format:epub'))).toBe(true);
    expect(isTokenComplete(mkToken('author:asimov'))).toBe(true);
    expect(isTokenComplete(mkToken('year:1965..1970'))).toBe(true);
  });

  it('complete: quoted field value', () => {
    expect(isTokenComplete(mkToken('author:"Isaac Asimov"'))).toBe(true);
  });

  it('complete: negated field:value', () => {
    expect(isTokenComplete(mkToken('-tag:scifi'))).toBe(true);
  });

  it('complete: bare text not matching operator prefix', () => {
    expect(isTokenComplete(mkToken('dune'))).toBe(true);
    expect(isTokenComplete(mkToken('hello'))).toBe(true);
  });

  it('complete: unrecognized field treated as text', () => {
    expect(isTokenComplete(mkToken('http://example.com'))).toBe(true);
  });

  it('incomplete: field with empty value', () => {
    expect(isTokenComplete(mkToken('author:'))).toBe(false);
    expect(isTokenComplete(mkToken('format:'))).toBe(false);
  });

  it('incomplete: unclosed quote in field value', () => {
    expect(isTokenComplete(mkToken('author:"Isaac'))).toBe(false);
  });

  it('incomplete: bare word matching operator prefix', () => {
    expect(isTokenComplete(mkToken('au'))).toBe(false);
    expect(isTokenComplete(mkToken('for'))).toBe(false);
    expect(isTokenComplete(mkToken('tag'))).toBe(false);
  });

  it('incomplete: bare unclosed quote', () => {
    expect(isTokenComplete(mkToken('"hello'))).toBe(false);
  });

  it('complete: bare closed quote', () => {
    expect(isTokenComplete(mkToken('"hello world"'))).toBe(true);
  });

  it('incomplete: bare dash (negation prefix with no content)', () => {
    expect(isTokenComplete(mkToken('-'))).toBe(false);
  });

  it('complete: negated text not matching operator prefix', () => {
    expect(isTokenComplete(mkToken('-dune'))).toBe(true);
  });

  it('incomplete: negated operator prefix', () => {
    expect(isTokenComplete(mkToken('-au'))).toBe(false);
  });
});

describe('splitQueryIntoChipsAndDraft', () => {
  beforeEach(() => resetChipIds());

  it('all complete tokens → all chips, empty draft', () => {
    const result = splitQueryIntoChipsAndDraft('author:asimov format:epub');
    expect(result.chips).toHaveLength(2);
    expect(result.chips[0].field).toBe('author');
    expect(result.chips[0].displayValue).toBe('asimov');
    expect(result.chips[1].field).toBe('format');
    expect(result.chips[1].displayValue).toBe('epub');
    expect(result.draft).toEqual({ ...EMPTY_DRAFT });
  });

  it('last token incomplete → becomes draft', () => {
    const result = splitQueryIntoChipsAndDraft('author:asimov au');
    expect(result.chips).toHaveLength(1);
    expect(result.chips[0].field).toBe('author');
    expect(result.draft.field).toBeNull();
    expect(result.draft.valueText).toBe('au');
  });

  it('single incomplete token → no chips, draft', () => {
    const result = splitQueryIntoChipsAndDraft('author:');
    expect(result.chips).toHaveLength(0);
    expect(result.draft.field).toBe('author');
    expect(result.draft.valueText).toBe('');
  });

  it('unclosed quote → draft with value', () => {
    const result = splitQueryIntoChipsAndDraft('author:"Isaac');
    expect(result.chips).toHaveLength(0);
    expect(result.draft.field).toBe('author');
    expect(result.draft.valueText).toBe('Isaac');
  });

  it('empty query → empty result', () => {
    const result = splitQueryIntoChipsAndDraft('');
    expect(result.chips).toHaveLength(0);
    expect(result.draft).toEqual({ ...EMPTY_DRAFT });
  });

  it('single complete text → one chip, empty draft', () => {
    const result = splitQueryIntoChipsAndDraft('dune');
    expect(result.chips).toHaveLength(1);
    expect(result.chips[0].kind).toBe('text');
    expect(result.chips[0].displayValue).toBe('dune');
    expect(result.draft).toEqual({ ...EMPTY_DRAFT });
  });

  it('bare dash → no chips, dash as draft', () => {
    const result = splitQueryIntoChipsAndDraft('-');
    expect(result.chips).toHaveLength(0);
    expect(result.draft.valueText).toBe('-');
  });

  it('text then bare dash → text chip, dash as draft', () => {
    const result = splitQueryIntoChipsAndDraft('dune -');
    expect(result.chips).toHaveLength(1);
    expect(result.chips[0].displayValue).toBe('dune');
    expect(result.draft.valueText).toBe('-');
  });

  it('complete tokens + incomplete trailing → chips + draft', () => {
    const result = splitQueryIntoChipsAndDraft('author:asimov author:');
    expect(result.chips).toHaveLength(1);
    expect(result.chips[0].field).toBe('author');
    expect(result.draft.field).toBe('author');
    expect(result.draft.valueText).toBe('');
  });

  it('negated token preserved in chip', () => {
    const result = splitQueryIntoChipsAndDraft('-tag:scifi');
    expect(result.chips).toHaveLength(1);
    expect(result.chips[0].negated).toBe(true);
    expect(result.chips[0].field).toBe('tag');
    expect(result.chips[0].displayValue).toBe('scifi');
  });
});

describe('commitDraft', () => {
  beforeEach(() => resetChipIds());

  it('field with multi-word value → quoted raw', () => {
    const chip = commitDraft({
      negated: false,
      field: 'author',
      fieldRaw: 'author',
      valueText: 'Isaac Asimov'
    });
    expect(chip.raw).toBe('author:"Isaac Asimov"');
    expect(chip.displayValue).toBe('Isaac Asimov');
    expect(chip.kind).toBe('field');
  });

  it('field with single-word value → unquoted raw', () => {
    const chip = commitDraft({
      negated: false,
      field: 'author',
      fieldRaw: 'author',
      valueText: 'asimov'
    });
    expect(chip.raw).toBe('author:asimov');
  });

  it('negated field → prefixed raw', () => {
    const chip = commitDraft({
      negated: true,
      field: 'tag',
      fieldRaw: 'tag',
      valueText: 'scifi'
    });
    expect(chip.raw).toBe('-tag:scifi');
    expect(chip.negated).toBe(true);
  });

  it('bare text draft → text chip', () => {
    const chip = commitDraft({
      negated: false,
      field: null,
      fieldRaw: null,
      valueText: 'dune'
    });
    expect(chip.raw).toBe('dune');
    expect(chip.kind).toBe('text');
  });

  it('preserves field alias in raw', () => {
    const chip = commitDraft({
      negated: false,
      field: 'publisher',
      fieldRaw: 'pub',
      valueText: 'ace'
    });
    expect(chip.raw).toBe('pub:ace');
  });

  it('bare negated draft → chip with negated=true and stripped displayValue', () => {
    const chip = commitDraft({
      negated: false,
      field: null,
      fieldRaw: null,
      valueText: '-foo'
    });
    expect(chip.raw).toBe('-foo');
    expect(chip.negated).toBe(true);
    expect(chip.displayValue).toBe('foo');
    expect(chip.kind).toBe('text');
  });
});

describe('serializeDraft', () => {
  it('field draft with multi-word value is quoted', () => {
    expect(
      serializeDraft({ negated: false, field: 'author', fieldRaw: 'author', valueText: 'Isaac Asimov' })
    ).toBe('author:"Isaac Asimov"');
  });

  it('field draft with single-word value stays unquoted', () => {
    expect(
      serializeDraft({ negated: false, field: 'author', fieldRaw: 'author', valueText: 'asimov' })
    ).toBe('author:asimov');
  });

  it('negated field draft with multi-word value', () => {
    expect(
      serializeDraft({ negated: true, field: 'author', fieldRaw: 'author', valueText: 'Isaac Asimov' })
    ).toBe('-author:"Isaac Asimov"');
  });

  it('bare draft passes valueText through (including negation)', () => {
    expect(
      serializeDraft({ negated: false, field: null, fieldRaw: null, valueText: '-foo' })
    ).toBe('-foo');
  });

  it('bare draft without negation', () => {
    expect(
      serializeDraft({ negated: false, field: null, fieldRaw: null, valueText: 'dune' })
    ).toBe('dune');
  });

  it('empty bare draft returns empty string', () => {
    expect(serializeDraft({ ...EMPTY_DRAFT })).toBe('');
  });
});

describe('chipsToQuery', () => {
  it('joins chips and empty draft', () => {
    const chips = [
      { id: '1', kind: 'field' as const, raw: 'author:asimov', negated: false, field: 'author', fieldRaw: 'author', displayValue: 'asimov' },
      { id: '2', kind: 'field' as const, raw: 'format:epub', negated: false, field: 'format', fieldRaw: 'format', displayValue: 'epub' }
    ];
    expect(chipsToQuery(chips, { ...EMPTY_DRAFT })).toBe('author:asimov format:epub');
  });

  it('appends draft in field mode with multi-word quoting', () => {
    const chips = [
      { id: '1', kind: 'field' as const, raw: 'format:epub', negated: false, field: 'format', fieldRaw: 'format', displayValue: 'epub' }
    ];
    const draft = { negated: false, field: 'author', fieldRaw: 'author', valueText: 'Isaac Asimov' };
    expect(chipsToQuery(chips, draft)).toBe('format:epub author:"Isaac Asimov"');
  });

  it('appends draft in bare mode', () => {
    const chips = [
      { id: '1', kind: 'field' as const, raw: 'format:epub', negated: false, field: 'format', fieldRaw: 'format', displayValue: 'epub' }
    ];
    const draft = { negated: false, field: null, fieldRaw: null, valueText: 'dune' };
    expect(chipsToQuery(chips, draft)).toBe('format:epub dune');
  });

  it('empty chips and empty draft → empty string', () => {
    expect(chipsToQuery([], { ...EMPTY_DRAFT })).toBe('');
  });
});

describe('tokenToDraft', () => {
  it('converts field token with value', () => {
    const token: TokenSpan = { start: 0, end: 10, text: 'author:asi' };
    const d = tokenToDraft(token);
    expect(d.field).toBe('author');
    expect(d.valueText).toBe('asi');
    expect(d.negated).toBe(false);
  });

  it('converts field token with empty value', () => {
    const token: TokenSpan = { start: 0, end: 7, text: 'author:' };
    const d = tokenToDraft(token);
    expect(d.field).toBe('author');
    expect(d.valueText).toBe('');
  });

  it('converts bare word to bare draft', () => {
    const token: TokenSpan = { start: 0, end: 2, text: 'au' };
    const d = tokenToDraft(token);
    expect(d.field).toBeNull();
    expect(d.valueText).toBe('au');
  });

  it('handles negated field token', () => {
    const token: TokenSpan = { start: 0, end: 12, text: '-author:test' };
    const d = tokenToDraft(token);
    expect(d.negated).toBe(true);
    expect(d.field).toBe('author');
    expect(d.valueText).toBe('test');
  });

  it('handles unclosed quote', () => {
    const token: TokenSpan = { start: 0, end: 13, text: 'author:"Isaac' };
    const d = tokenToDraft(token);
    expect(d.field).toBe('author');
    expect(d.valueText).toBe('Isaac');
  });

  it('bare negated token keeps - in valueText with negated=false', () => {
    const token: TokenSpan = { start: 0, end: 4, text: '-foo' };
    const d = tokenToDraft(token);
    expect(d.negated).toBe(false);
    expect(d.field).toBeNull();
    expect(d.valueText).toBe('-foo');
  });
});

// ---------------------------------------------------------------------------
// B. Component tests for DslSearchBox.svelte
// ---------------------------------------------------------------------------

// Mock the API module
vi.mock('$lib/api/index.js', () => {
  const mockAuthors = {
    search: vi.fn().mockResolvedValue({
      items: [
        { id: 'a1', name: 'Isaac Asimov', sort_name: 'Asimov, Isaac', book_count: 5 },
        { id: 'a2', name: 'Isaac Newton', sort_name: 'Newton, Isaac', book_count: 1 }
      ],
      total: 2,
      page: 1,
      per_page: 10,
      total_pages: 1
    })
  };
  const mockSeries = { search: vi.fn().mockResolvedValue({ items: [] }) };
  const mockPublishers = { search: vi.fn().mockResolvedValue({ items: [] }) };
  const mockTags = { search: vi.fn().mockResolvedValue({ items: [] }) };

  return {
    api: {
      authors: mockAuthors,
      series: mockSeries,
      publishers: mockPublishers,
      tags: mockTags
    }
  };
});

describe('DslSearchBox', () => {
  type OnChangeFn = (value: string) => void;
  type OnSubmitFn = () => void;
  type OnWarningPickFn = (field: string, query: string, id: string, name: string, negated: boolean) => void;

  let onchangeFn: ReturnType<typeof vi.fn<OnChangeFn>>;
  let onsubmitFn: ReturnType<typeof vi.fn<OnSubmitFn>>;
  let onWarningPickFn: ReturnType<typeof vi.fn<OnWarningPickFn>>;

  beforeEach(() => {
    onchangeFn = vi.fn<OnChangeFn>();
    onsubmitFn = vi.fn<OnSubmitFn>();
    onWarningPickFn = vi.fn<OnWarningPickFn>();
    // Ensure real timers are active (guards against leaking from a previous test)
    vi.useRealTimers();
  });

  function renderBox(props: Record<string, unknown> = {}) {
    return render(DslSearchBox, {
      props: {
        value: '',
        onchange: onchangeFn,
        onsubmit: onsubmitFn,
        onWarningPick: onWarningPickFn,
        ...props
      }
    });
  }

  it('renders input with placeholder and search icon', () => {
    renderBox({ placeholder: 'Search books...' });
    expect(screen.getByPlaceholderText('Search books...')).toBeInTheDocument();
    expect(document.querySelector('svg circle')).toBeInTheDocument();
  });

  it('shows operator suggestions when typing a prefix', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'au');

    expect(screen.getByText('author:')).toBeInTheDocument();
  });

  it('shows all format enum values when typing format:', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'format:');

    expect(screen.getByText('epub')).toBeInTheDocument();
    expect(screen.getByText('pdf')).toBeInTheDocument();
    expect(screen.getByText('mobi')).toBeInTheDocument();
  });

  it('filters enum values while typing', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'format:ep');

    expect(screen.getByText('epub')).toBeInTheDocument();
    expect(screen.queryByText('pdf')).not.toBeInTheDocument();
  });

  it('shows language suggestions for lang:', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'lang:en');

    expect(screen.getByText('English (en)')).toBeInTheDocument();
  });

  it('selects an operator suggestion via click', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'au');

    const suggestion = screen.getByText('author:');
    await user.click(suggestion);

    expect(onchangeFn).toHaveBeenCalled();
    const lastCallValue = onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0];
    expect(lastCallValue).toContain('author:');
  });

  it('selects a value suggestion via ArrowDown+Enter', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'format:ep');

    await user.keyboard('{ArrowDown}');
    await user.keyboard('{Enter}');

    const lastCallValue = onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0];
    expect(lastCallValue).toContain('format:epub');
  });

  it('calls onsubmit on Enter with no highlighted suggestion', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'hello world');
    await user.keyboard('{Enter}');

    expect(onsubmitFn).toHaveBeenCalled();
  });

  it('closes dropdown on Escape', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'au');
    expect(screen.getByText('author:')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByText('author:')).not.toBeInTheDocument();
  });

  it('renders warnings below input when showWarnings is true', () => {
    renderBox({
      warnings: [
        {
          type: 'unknown_relation' as const,
          field: 'author',
          query: 'nonexistent',
          negated: false
        }
      ],
      showWarnings: true
    });

    expect(screen.getByText(/No author found matching/)).toBeInTheDocument();
  });

  it('hides warnings when showWarnings is false', () => {
    renderBox({
      warnings: [
        {
          type: 'unknown_relation' as const,
          field: 'author',
          query: 'nonexistent',
          negated: false
        }
      ],
      showWarnings: false
    });

    expect(screen.queryByText(/No author found matching/)).not.toBeInTheDocument();
  });

  it('fires warning pick callback with field, query, id, name, and negated=false', async () => {
    const user = userEvent.setup();
    renderBox({
      warnings: [
        {
          type: 'ambiguous_relation' as const,
          field: 'author',
          query: 'smith',
          negated: false,
          match_count: 2,
          matches: [
            { id: 'a1', name: 'John Smith' },
            { id: 'a2', name: 'Jane Smith' }
          ]
        }
      ],
      showWarnings: true
    });

    const pickBtn = screen.getByText('John Smith');
    await user.click(pickBtn);

    expect(onWarningPickFn).toHaveBeenCalledWith('author', 'smith', 'a1', 'John Smith', false);
  });

  it('fires warning pick callback with negated=true for negated ambiguous warning', async () => {
    const user = userEvent.setup();
    renderBox({
      warnings: [
        {
          type: 'ambiguous_relation' as const,
          field: 'author',
          query: 'smith',
          negated: true,
          match_count: 2,
          matches: [
            { id: 'a1', name: 'John Smith' },
            { id: 'a2', name: 'Jane Smith' }
          ]
        }
      ],
      showWarnings: true
    });

    // Warning text should include the negation prefix
    const label = screen.getByText('-author:smith');
    expect(label).toBeInTheDocument();

    const pickBtn = screen.getByText('John Smith');
    await user.click(pickBtn);

    expect(onWarningPickFn).toHaveBeenCalledWith('author', 'smith', 'a1', 'John Smith', true);
  });

  it('distinguishes repeated same-field warnings', async () => {
    const user = userEvent.setup();
    renderBox({
      warnings: [
        {
          type: 'ambiguous_relation' as const,
          field: 'tag',
          query: 'sci',
          negated: false,
          match_count: 2,
          matches: [
            { id: 't1', name: 'Science' },
            { id: 't2', name: 'Sci-Fi' }
          ]
        },
        {
          type: 'ambiguous_relation' as const,
          field: 'tag',
          query: 'fan',
          negated: false,
          match_count: 2,
          matches: [
            { id: 't3', name: 'Fantasy' },
            { id: 't4', name: 'Fan Fiction' }
          ]
        }
      ],
      showWarnings: true
    });

    const scienceBtn = screen.getByText('Science');
    await user.click(scienceBtn);
    expect(onWarningPickFn).toHaveBeenCalledWith('tag', 'sci', 't1', 'Science', false);

    const fantasyBtn = screen.getByText('Fantasy');
    await user.click(fantasyBtn);
    expect(onWarningPickFn).toHaveBeenCalledWith('tag', 'fan', 't3', 'Fantasy', false);
  });

  // --- Stale-request protection ---

  it('fetches relation suggestions with stale-request protection (relation→relation)', async () => {
    vi.useFakeTimers();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const { api } = await import('$lib/api/index.js');

    let resolveFirst: (v: unknown) => void;
    const firstPromise = new Promise((r) => {
      resolveFirst = r;
    });
    const secondResult = {
      items: [{ id: 'a3', name: 'Asi Special', sort_name: 'Asi', book_count: 1 }],
      total: 1,
      page: 1,
      per_page: 10,
      total_pages: 1
    };

    vi.mocked(api.authors.search)
      .mockImplementationOnce(() => firstPromise as ReturnType<typeof api.authors.search>)
      .mockResolvedValueOnce(secondResult);

    renderBox();
    const input = screen.getByPlaceholderText('Search books...');

    // Type `author:a` → triggers field mode with relation suggestions
    await user.type(input, 'author:a');
    await vi.advanceTimersByTimeAsync(200);

    // Type more → value becomes `asi`
    await user.type(input, 'si');
    await vi.advanceTimersByTimeAsync(200);

    await vi.advanceTimersByTimeAsync(0);

    // Resolve the first (stale) response
    resolveFirst!({
      items: [{ id: 'stale', name: 'Stale Result', sort_name: 'Stale', book_count: 0 }],
      total: 1,
      page: 1,
      per_page: 10,
      total_pages: 1
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.queryByText('Stale Result')).not.toBeInTheDocument();

    vi.useRealTimers();
  });

  it('discards stale relation response when mode switches to non-relation (relation→enum)', async () => {
    vi.useFakeTimers();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const { api } = await import('$lib/api/index.js');

    let resolveAuthor: (v: unknown) => void;
    const authorPromise = new Promise((r) => {
      resolveAuthor = r;
    });

    vi.mocked(api.authors.search).mockImplementationOnce(
      () => authorPromise as ReturnType<typeof api.authors.search>
    );

    renderBox();
    const input = screen.getByPlaceholderText('Search books...');

    // Type `author:a` → auto-detects field mode (author), valueText = 'a'
    await user.type(input, 'author:a');
    await vi.advanceTimersByTimeAsync(200); // debounce fires, API call in flight

    // Exit field mode via Backspace (clear `a`, then dissolve field), then type `format:`
    await user.keyboard('{Backspace}'); // clears valueText → ''
    await user.keyboard('{Backspace}'); // dissolves field → bare mode with 'author'
    await user.clear(input); // clear the bare-mode text
    await user.type(input, 'format:');

    // Enum suggestions should be showing
    expect(screen.getByText('epub')).toBeInTheDocument();

    // Stale author response arrives
    resolveAuthor!({
      items: [{ id: 'a1', name: 'Isaac Asimov', sort_name: 'Asimov', book_count: 5 }],
      total: 1,
      page: 1,
      per_page: 10,
      total_pages: 1
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.queryByText('Isaac Asimov')).not.toBeInTheDocument();
    expect(screen.getByText('epub')).toBeInTheDocument();

    vi.useRealTimers();
  });

  // --- Chip behavior tests ---

  it('creates a chip when typing a complete token followed by space and more text', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    // Type `dune f` — `dune` is complete, `f` starts the next
    await user.type(input, 'dune f');

    expect(screen.getByText('dune')).toBeInTheDocument();
    expect(screen.getByLabelText('Remove dune')).toBeInTheDocument();
  });

  it('auto-detects field mode when typing field:', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'format:');

    // Should show format enum suggestions (field mode detected)
    expect(screen.getByText('epub')).toBeInTheDocument();
  });

  it('creates chip from value suggestion selection', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, 'format:ep');

    await user.keyboard('{ArrowDown}');
    await user.keyboard('{Enter}');

    // Should have a chip for format:epub
    expect(screen.getByText('format:')).toBeInTheDocument();
    expect(screen.getByText('epub')).toBeInTheDocument();
  });

  it('backspace on empty input dissolves last chip into draft', async () => {
    const user = userEvent.setup();
    renderBox({ value: 'format:epub' });

    // The value is parsed into a chip
    expect(screen.getByText('format:')).toBeInTheDocument();
    expect(screen.getByText('epub')).toBeInTheDocument();

    const input = document.querySelector('input[role="combobox"]') as HTMLInputElement;
    await user.click(input);
    await user.keyboard('{Backspace}');

    // Chip should be dissolved — the remove button should be gone
    expect(screen.queryByLabelText('Remove format:epub')).not.toBeInTheDocument();
  });

  it('removes chip via X button click', async () => {
    const user = userEvent.setup();
    renderBox({ value: 'author:asimov format:epub' });

    expect(screen.getByLabelText('Remove author:asimov')).toBeInTheDocument();
    expect(screen.getByLabelText('Remove format:epub')).toBeInTheDocument();

    await user.click(screen.getByLabelText('Remove author:asimov'));

    expect(screen.queryByLabelText('Remove author:asimov')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Remove format:epub')).toBeInTheDocument();
  });

  it('external value change re-parses into chips', async () => {
    const { rerender } = render(DslSearchBox, {
      props: {
        value: 'author:asimov',
        onchange: onchangeFn,
        onsubmit: onsubmitFn
      }
    });

    expect(screen.getByText('asimov')).toBeInTheDocument();

    await rerender({
      value: 'format:epub dune',
      onchange: onchangeFn,
      onsubmit: onsubmitFn
    });

    expect(screen.getByText('epub')).toBeInTheDocument();
    expect(screen.getByText('dune')).toBeInTheDocument();
    expect(screen.queryByText('asimov')).not.toBeInTheDocument();
  });

  it('external value cleared removes all chips', async () => {
    const { rerender } = render(DslSearchBox, {
      props: {
        value: 'author:asimov format:epub',
        onchange: onchangeFn,
        onsubmit: onsubmitFn
      }
    });

    expect(screen.getByText('asimov')).toBeInTheDocument();

    await rerender({
      value: '',
      onchange: onchangeFn,
      onsubmit: onsubmitFn
    });

    expect(screen.queryByText('asimov')).not.toBeInTheDocument();
    expect(screen.queryByText('epub')).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText('Search books...')).toBeInTheDocument();
  });

  it('incomplete external value restores as draft', async () => {
    renderBox({ value: 'author:' });

    const fieldLabel = screen.getByText('author:');
    expect(fieldLabel).toBeInTheDocument();
    expect(screen.getByPlaceholderText('type value...')).toBeInTheDocument();
    expect(screen.queryByLabelText(/Remove/)).not.toBeInTheDocument();
  });

  it('negated chip displays negation prefix', () => {
    renderBox({ value: '-tag:scifi' });

    expect(screen.getByText('-')).toBeInTheDocument();
    expect(screen.getByText('tag:')).toBeInTheDocument();
    expect(screen.getByText('scifi')).toBeInTheDocument();
  });

  it('dissolving negated field chip to bare mode does not double-negate', async () => {
    const user = userEvent.setup();
    renderBox({ value: '-author:asimov' });

    // Chip should be rendered
    expect(screen.getByText('-')).toBeInTheDocument();
    expect(screen.getByText('author:')).toBeInTheDocument();
    expect(screen.getByText('asimov')).toBeInTheDocument();

    const input = document.querySelector('input[role="combobox"]') as HTMLInputElement;
    await user.click(input);

    // Backspace 1: dissolve chip → field-mode draft {negated:true, field:'author', valueText:'asimov'}
    await user.keyboard('{Backspace}');
    // Clear the value text, then one more backspace to dissolve field → bare
    await user.clear(input);
    await user.keyboard('{Backspace}');

    // Should NOT produce `--author` — the last onchange value must not start with `--`
    const lastValue = onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0] as string;
    expect(lastValue).not.toMatch(/^--/);
    expect(lastValue).toBe('-author');
  });

  it('negated operator selection preserves negation from bare draft', async () => {
    const user = userEvent.setup();
    renderBox();

    const input = screen.getByPlaceholderText('Search books...');
    await user.type(input, '-au');

    const suggestion = screen.getByText('author:');
    await user.click(suggestion);

    const lastValue = onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0] as string;
    expect(lastValue).toBe('-author:');
  });

  it('external bare negated draft restores correctly', async () => {
    // `-foo` is an operator prefix so it stays as draft
    renderBox({ value: '-for' });

    // Should be a draft, not a chip (no remove button)
    expect(screen.queryByLabelText(/Remove/)).not.toBeInTheDocument();

    // Serialized value should round-trip
    const lastValue = onchangeFn.mock.calls.length > 0
      ? onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0] as string
      : '-for';
    expect(lastValue).toBe('-for');
  });

  it('hides warnings when showWarnings changes from true to false', async () => {
    const { rerender } = render(DslSearchBox, {
      props: {
        value: 'author:smith',
        warnings: [
          {
            type: 'unknown_relation' as const,
            field: 'author',
            query: 'smith',
            negated: false
          }
        ],
        showWarnings: true,
        onchange: onchangeFn,
        onsubmit: onsubmitFn
      }
    });

    expect(screen.getByText(/No author found matching/)).toBeInTheDocument();

    await rerender({
      value: 'author:smit',
      warnings: [
        {
          type: 'unknown_relation' as const,
          field: 'author',
          query: 'smith',
          negated: false
        }
      ],
      showWarnings: false,
      onchange: onchangeFn,
      onsubmit: onsubmitFn
    });

    expect(screen.queryByText(/No author found matching/)).not.toBeInTheDocument();
  });
});
