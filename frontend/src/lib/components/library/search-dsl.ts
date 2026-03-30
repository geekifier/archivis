import { LANGUAGES } from '$lib/data/languages.js';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface TokenSpan {
  /** Start index in the full query string. */
  start: number;
  /** End index (exclusive). */
  end: number;
  /** Raw text of the token (slice of the query). */
  text: string;
}

export type OperatorCategory =
  | 'relation'
  | 'text'
  | 'enum'
  | 'boolean'
  | 'range'
  | 'presence'
  | 'identifier'
  | 'language';

export interface OperatorDef {
  name: string;
  aliases: string[];
  category: OperatorCategory;
  description: string;
  values?: string[];
}

export type SuggestionMode =
  | { kind: 'operators'; prefix: string }
  | { kind: 'relation'; field: string; value: string }
  | { kind: 'enum'; field: string; value: string; options: string[] }
  | { kind: 'boolean'; field: string; value: string }
  | { kind: 'presence'; field: string; value: string; options: string[] }
  | { kind: 'language'; value: string }
  | { kind: 'freetext' }
  | { kind: 'none' };

// ---------------------------------------------------------------------------
// Operator registry
// ---------------------------------------------------------------------------

const BOOLEAN_VALUES = ['true', 'false', 'yes', 'no', '1', '0'];
const PRESENCE_VALUES = ['cover', 'description', 'desc', 'identifiers', 'identifier', 'ids'];

export const OPERATORS: OperatorDef[] = [
  // Relation fields
  { name: 'author', aliases: [], category: 'relation', description: 'Search by author name' },
  { name: 'series', aliases: [], category: 'relation', description: 'Search by series name' },
  {
    name: 'publisher',
    aliases: ['pub'],
    category: 'relation',
    description: 'Search by publisher name'
  },
  { name: 'tag', aliases: [], category: 'relation', description: 'Search by tag name' },
  // Text fields
  { name: 'title', aliases: [], category: 'text', description: 'Full-text search in titles' },
  {
    name: 'description',
    aliases: ['desc'],
    category: 'text',
    description: 'Full-text search in descriptions'
  },
  // Enum fields
  {
    name: 'format',
    aliases: ['fmt'],
    category: 'enum',
    description: 'File format',
    values: ['epub', 'pdf', 'mobi', 'cbz', 'fb2', 'txt', 'djvu', 'azw3']
  },
  {
    name: 'status',
    aliases: [],
    category: 'enum',
    description: 'Metadata status',
    values: ['identified', 'needs_review', 'unidentified']
  },
  {
    name: 'resolution',
    aliases: [],
    category: 'enum',
    description: 'Resolution state',
    values: ['pending', 'running', 'done', 'failed']
  },
  {
    name: 'outcome',
    aliases: [],
    category: 'enum',
    description: 'Resolution outcome',
    values: ['confirmed', 'enriched', 'disputed', 'ambiguous', 'unmatched']
  },
  // Language
  {
    name: 'language',
    aliases: ['lang'],
    category: 'language',
    description: 'Book language (ISO 639-1)'
  },
  // Boolean fields
  {
    name: 'trusted',
    aliases: [],
    category: 'boolean',
    description: 'Metadata trusted by user',
    values: BOOLEAN_VALUES
  },
  {
    name: 'locked',
    aliases: [],
    category: 'boolean',
    description: 'Metadata locked',
    values: BOOLEAN_VALUES
  },
  // Range
  {
    name: 'year',
    aliases: [],
    category: 'range',
    description: 'Publication year or range (e.g. 1965, 1965..1970, >=1965)'
  },
  // Presence
  {
    name: 'has',
    aliases: [],
    category: 'presence',
    description: 'Has a specific field',
    values: PRESENCE_VALUES
  },
  {
    name: 'missing',
    aliases: [],
    category: 'presence',
    description: 'Missing a specific field',
    values: PRESENCE_VALUES
  },
  // Identifiers
  {
    name: 'identifier',
    aliases: ['id'],
    category: 'identifier',
    description: 'Identifier lookup (`identifier:value` or `identifier:type:value`)'
  }
];

/** Map from any recognized name/alias to its canonical `OperatorDef`. */
export const OPERATOR_LOOKUP: Map<string, OperatorDef> = new Map();
for (const op of OPERATORS) {
  OPERATOR_LOOKUP.set(op.name, op);
  for (const alias of op.aliases) {
    OPERATOR_LOOKUP.set(alias, op);
  }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/**
 * Tokenize a query string into `TokenSpan` objects, respecting:
 * - Whitespace-delimited tokens
 * - Quoted phrases (double quotes), including field-qualified (`author:"Foo"`)
 * - Incomplete/unclosed quotes (extend to end of input)
 * - Negation prefix (`-`) as part of the token
 */
export function tokenize(query: string): TokenSpan[] {
  const tokens: TokenSpan[] = [];
  let i = 0;
  const len = query.length;

  while (i < len) {
    // Skip whitespace
    if (query[i] === ' ' || query[i] === '\t') {
      i++;
      continue;
    }

    const start = i;

    // Check for a bare quoted phrase (starts with `"` or `-"`)
    if (query[i] === '"' || (query[i] === '-' && i + 1 < len && query[i + 1] === '"')) {
      // Could be a negated quote `-"..."`
      if (query[i] === '-') i++; // skip the `-`
      i++; // skip opening `"`
      // Scan to closing `"` or end of input
      while (i < len && query[i] !== '"') i++;
      if (i < len) i++; // skip closing `"`
      tokens.push({ start, end: i, text: query.slice(start, i) });
      continue;
    }

    // Regular token: scan until whitespace, but handle field:quoted-value
    while (i < len && query[i] !== ' ' && query[i] !== '\t') {
      if (query[i] === '"') {
        // Start of a quoted section within the token (e.g. `author:"Foo Bar"`)
        i++; // skip opening `"`
        while (i < len && query[i] !== '"') i++;
        if (i < len) i++; // skip closing `"`
      } else {
        i++;
      }
    }

    tokens.push({ start, end: i, text: query.slice(start, i) });
  }

  return tokens;
}

// ---------------------------------------------------------------------------
// Token analysis helpers
// ---------------------------------------------------------------------------

export interface ParsedToken {
  negated: boolean;
  field: string | null;
  /** The raw field text as written (before canonical lookup). */
  fieldRaw: string | null;
  value: string;
  /** The unquoted value (strips surrounding `"` if present, handles unclosed quotes). */
  valueUnquoted: string;
}

export function parseToken(text: string): ParsedToken {
  let rest = text;
  let negated = false;

  if (rest.startsWith('-')) {
    negated = true;
    rest = rest.slice(1);
  }

  // Find the first colon that isn't inside quotes
  const colonIdx = findFieldColon(rest);
  if (colonIdx === -1) {
    return { negated, field: null, fieldRaw: null, value: rest, valueUnquoted: rest };
  }

  const fieldRaw = rest.slice(0, colonIdx);
  const rawValue = rest.slice(colonIdx + 1);
  return {
    negated,
    field: fieldRaw.toLowerCase(),
    fieldRaw,
    value: rawValue,
    valueUnquoted: stripQuotes(rawValue)
  };
}

/** Find the index of the field-separating colon (before any quoted section). */
function findFieldColon(text: string): number {
  for (let i = 0; i < text.length; i++) {
    if (text[i] === ':') return i;
    if (text[i] === '"') return -1; // quote before colon → not a field
  }
  return -1;
}

export function stripQuotes(s: string): string {
  if (s.length >= 2 && s[0] === '"' && s[s.length - 1] === '"') {
    return s.slice(1, -1);
  }
  if (s.length >= 1 && s[0] === '"') {
    return s.slice(1); // unclosed quote
  }
  return s;
}

// ---------------------------------------------------------------------------
// parseTokenAtCursor
// ---------------------------------------------------------------------------

export function parseTokenAtCursor(
  query: string,
  cursorPos: number
): { token: TokenSpan | null; mode: SuggestionMode } {
  const tokens = tokenize(query);

  // Find the token the cursor is inside or immediately after
  let token: TokenSpan | null = null;
  for (const t of tokens) {
    if (cursorPos >= t.start && cursorPos <= t.end) {
      token = t;
      break;
    }
  }

  // Cursor is in whitespace or at the start of an empty query
  if (!token) {
    return { token: null, mode: { kind: 'operators', prefix: '' } };
  }

  const parsed = parseToken(token.text);

  // No field separator — could be an operator prefix or a bare quoted phrase
  if (parsed.field === null) {
    // Bare quoted phrase (starts with `"` or `-"`)
    const stripped = parsed.negated ? token.text.slice(1) : token.text;
    if (stripped.startsWith('"')) {
      return { token, mode: { kind: 'none' } };
    }
    // Bare word — check if it's an operator prefix
    return { token, mode: { kind: 'operators', prefix: parsed.value.toLowerCase() } };
  }

  // Has a field — look it up in the registry
  const op = OPERATOR_LOOKUP.get(parsed.field);
  if (!op) {
    return { token, mode: { kind: 'none' } };
  }

  switch (op.category) {
    case 'relation':
      return {
        token,
        mode: { kind: 'relation', field: op.name, value: parsed.valueUnquoted }
      };
    case 'enum':
      return {
        token,
        mode: { kind: 'enum', field: op.name, value: parsed.valueUnquoted, options: op.values! }
      };
    case 'boolean':
      return {
        token,
        mode: { kind: 'boolean', field: op.name, value: parsed.valueUnquoted }
      };
    case 'presence':
      return {
        token,
        mode: {
          kind: 'presence',
          field: op.name,
          value: parsed.valueUnquoted,
          options: op.values!
        }
      };
    case 'language':
      return { token, mode: { kind: 'language', value: parsed.valueUnquoted } };
    case 'text':
    case 'range':
    case 'identifier':
      return { token, mode: { kind: 'freetext' } };
  }
}

// ---------------------------------------------------------------------------
// replaceTokenInQuery
// ---------------------------------------------------------------------------

export function replaceTokenInQuery(
  query: string,
  token: TokenSpan,
  replacement: string
): { newQuery: string; newCursorPos: number } {
  const before = query.slice(0, token.start);
  const after = query.slice(token.end);

  // Empty replacement → remove the token and collapse extra whitespace
  if (replacement === '') {
    const joined = (before + after).replace(/  +/g, ' ');
    return { newQuery: joined, newCursorPos: Math.min(before.length, joined.length) };
  }

  // Preserve negation prefix if the original token was negated
  const parsed = parseToken(token.text);
  let prefix = '';
  if (parsed.negated && !replacement.startsWith('-')) {
    prefix = '-';
  }

  const full = prefix + replacement;
  const needsTrailingSpace = !full.endsWith(':') && !after.startsWith(' ') && after.length > 0;
  const newQuery = before + full + (needsTrailingSpace ? ' ' : '') + after;
  const newCursorPos = before.length + full.length + (needsTrailingSpace ? 1 : 0);

  return { newQuery, newCursorPos };
}

/**
 * Build the replacement string for a field:value selection.
 * Wraps the value in quotes if it contains spaces.
 */
export function buildFieldValue(field: string, value: string): string {
  if (value.includes(' ')) {
    return `${field}:"${value}"`;
  }
  return `${field}:${value}`;
}

// ---------------------------------------------------------------------------
// findTokenByFieldAndQuery
// ---------------------------------------------------------------------------

/**
 * Find the first token in the query whose field matches `field` (or alias)
 * AND whose unquoted value matches `queryText`.
 */
export function findTokenByFieldAndQuery(
  query: string,
  field: string,
  queryText: string
): TokenSpan | null {
  const tokens = tokenize(query);
  for (const t of tokens) {
    const parsed = parseToken(t.text);
    if (parsed.field === null) continue;
    const op = OPERATOR_LOOKUP.get(parsed.field);
    if (!op) continue;
    if (op.name === field && parsed.valueUnquoted === queryText) {
      return t;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Suggestion helpers
// ---------------------------------------------------------------------------

export interface SuggestionItem {
  id: string;
  label: string;
  sublabel?: string;
  /** The text to insert when this suggestion is selected. */
  insertText: string;
}

/** Filter operators by prefix and return suggestion items. */
export function getOperatorSuggestions(prefix: string): SuggestionItem[] {
  const lower = prefix.toLowerCase();
  const results: SuggestionItem[] = [];

  for (const op of OPERATORS) {
    const nameMatches = op.name.startsWith(lower);
    const aliasMatch = op.aliases.find((a) => a.startsWith(lower));

    if (nameMatches || aliasMatch) {
      const aliasHint = op.aliases.length > 0 ? ` (alias: ${op.aliases.join(', ')})` : '';
      results.push({
        id: `op:${op.name}`,
        label: `${op.name}:`,
        sublabel: op.description + aliasHint,
        insertText: `${op.name}:`
      });
    }
  }

  return results;
}

/** Filter enum/presence values and return suggestion items. */
export function getValueSuggestions(
  field: string,
  value: string,
  options: string[]
): SuggestionItem[] {
  const lower = value.toLowerCase();
  return options
    .filter((v) => v.toLowerCase().startsWith(lower))
    .map((v) => ({
      id: `val:${field}:${v}`,
      label: v,
      insertText: buildFieldValue(field, v)
    }));
}

/** Filter boolean values and return suggestion items. */
export function getBooleanSuggestions(field: string, value: string): SuggestionItem[] {
  return getValueSuggestions(field, value, BOOLEAN_VALUES);
}

/** Filter LANGUAGES list and return suggestion items. */
export function getLanguageSuggestions(value: string): SuggestionItem[] {
  const lower = value.toLowerCase();
  const results: SuggestionItem[] = [];

  for (const [code, label] of LANGUAGES) {
    if (code.toLowerCase().startsWith(lower) || label.toLowerCase().startsWith(lower)) {
      results.push({
        id: `lang:${code}`,
        label: `${label} (${code})`,
        insertText: `language:${code}`
      });
    }
  }

  return results;
}

// ---------------------------------------------------------------------------
// Chip / Draft types and helpers
// ---------------------------------------------------------------------------

export interface SearchChip {
  /** Unique ID for keyed rendering. */
  id: string;
  kind: 'field' | 'text';
  /** Original raw token text — used for serialization roundtrip. */
  raw: string;
  negated: boolean;
  /** Canonical field name (via `OPERATOR_LOOKUP`), null for text chips. */
  field: string | null;
  /** Field text as written (preserves aliases like `pub`, `fmt`). */
  fieldRaw: string | null;
  /** Unquoted display value. */
  displayValue: string;
}

export interface DraftState {
  negated: boolean;
  /** Canonical field name when in field mode, null for bare mode. */
  field: string | null;
  /** Field text as written (e.g., `pub` for publisher alias). */
  fieldRaw: string | null;
  /** Text being typed — entire input in bare mode, just the value in field mode. */
  valueText: string;
}

export const EMPTY_DRAFT: DraftState = {
  negated: false,
  field: null,
  fieldRaw: null,
  valueText: ''
};

let chipId = 0;

/** Reset the chip ID counter (for tests). */
export function resetChipIds(): void {
  chipId = 0;
}

// ---------------------------------------------------------------------------
// Token completeness check
// ---------------------------------------------------------------------------

/**
 * Determine whether a token is "complete" (should become a chip) or still
 * being edited (should remain as draft).
 *
 * Complete:
 *  - `field:value` with recognized field, non-empty value, and closed quotes
 *  - bare word that does NOT match any operator prefix (e.g., `dune`)
 *  - bare quoted phrase with closed quotes
 *  - negated variants of any of the above
 *
 * Incomplete:
 *  - `field:` with empty value
 *  - `field:"unclosed`
 *  - bare word matching an operator prefix (e.g., `au` → `author`)
 *  - bare quoted phrase with unclosed quote
 */
export function isTokenComplete(token: TokenSpan): boolean {
  const parsed = parseToken(token.text);

  if (parsed.field !== null) {
    // Has a colon — check if the field is recognized and value is non-empty
    const op = OPERATOR_LOOKUP.get(parsed.field);
    if (!op) {
      // Unrecognized field — treat as complete text (e.g., `http://...`)
      return true;
    }
    if (parsed.value === '') return false; // `field:` with no value
    // Check for unclosed quote
    if (parsed.value.startsWith('"') && !parsed.value.endsWith('"')) return false;
    return true;
  }

  // No field separator
  const text = parsed.negated ? token.text.slice(1) : token.text;

  // Bare quoted phrase
  if (text.startsWith('"')) {
    // Complete if closes with `"`
    return text.length >= 2 && text.endsWith('"');
  }

  // Bare word — complete only if it does NOT match any operator prefix
  return getOperatorSuggestions(text).length === 0;
}

// ---------------------------------------------------------------------------
// Token ↔ Chip / Draft conversions
// ---------------------------------------------------------------------------

/** Convert a complete token into a `SearchChip`. */
export function tokenToChip(token: TokenSpan): SearchChip {
  const parsed = parseToken(token.text);
  const op = parsed.field ? OPERATOR_LOOKUP.get(parsed.field) : null;
  return {
    id: `chip-${chipId++}`,
    kind: parsed.field && op ? 'field' : 'text',
    raw: token.text,
    negated: parsed.negated,
    field: op ? op.name : null,
    fieldRaw: parsed.fieldRaw,
    displayValue: parsed.field ? parsed.valueUnquoted : parsed.value
  };
}

/** Convert an incomplete token into a structured `DraftState`. */
export function tokenToDraft(token: TokenSpan): DraftState {
  const parsed = parseToken(token.text);
  const op = parsed.field ? OPERATOR_LOOKUP.get(parsed.field) : null;
  if (op) {
    return {
      negated: parsed.negated,
      field: op.name,
      fieldRaw: parsed.fieldRaw,
      valueText: parsed.valueUnquoted
    };
  }
  // Bare mode
  return {
    negated: parsed.negated,
    field: null,
    fieldRaw: null,
    valueText: parsed.value
  };
}

/**
 * Commit a draft into a finalized `SearchChip`.
 * Uses `buildFieldValue()` so multi-word values get properly quoted.
 */
export function commitDraft(draft: DraftState): SearchChip {
  const neg = draft.negated ? '-' : '';
  let raw: string;
  let displayValue: string;
  let kind: 'field' | 'text';

  if (draft.field) {
    raw = neg + buildFieldValue(draft.fieldRaw ?? draft.field, draft.valueText);
    displayValue = draft.valueText;
    kind = 'field';
  } else {
    raw = neg + draft.valueText;
    displayValue = draft.valueText;
    kind = 'text';
  }

  return {
    id: `chip-${chipId++}`,
    kind,
    raw,
    negated: draft.negated,
    field: draft.field,
    fieldRaw: draft.fieldRaw,
    displayValue
  };
}

// ---------------------------------------------------------------------------
// Query ↔ Chips+Draft serialization
// ---------------------------------------------------------------------------

/**
 * Parse a query string into committed chips plus an optional trailing draft.
 *
 * The last token becomes a draft if `isTokenComplete()` returns false.
 * All preceding tokens always become chips.
 */
export function splitQueryIntoChipsAndDraft(
  query: string
): { chips: SearchChip[]; draft: DraftState } {
  const tokens = tokenize(query);
  if (tokens.length === 0) {
    return { chips: [], draft: { ...EMPTY_DRAFT } };
  }

  const last = tokens[tokens.length - 1];
  if (!isTokenComplete(last)) {
    return {
      chips: tokens.slice(0, -1).map(tokenToChip),
      draft: tokenToDraft(last)
    };
  }

  return {
    chips: tokens.map(tokenToChip),
    draft: { ...EMPTY_DRAFT }
  };
}

/**
 * Serialize a draft to its in-progress text form.
 * Does NOT quote multi-word values — quoting only happens at commit time.
 */
export function serializeDraft(draft: DraftState): string {
  const neg = draft.negated ? '-' : '';
  if (draft.field) {
    const fieldText = draft.fieldRaw ?? draft.field;
    return draft.valueText
      ? `${neg}${fieldText}:${draft.valueText}`
      : `${neg}${fieldText}:`;
  }
  return draft.valueText ? neg + draft.valueText : '';
}

/** Join chip raw texts + serialized draft into a full query string. */
export function chipsToQuery(chips: SearchChip[], draft: DraftState): string {
  const parts = chips.map((c) => c.raw);
  const draftText = serializeDraft(draft);
  if (draftText) parts.push(draftText);
  return parts.join(' ');
}

/**
 * Build a `SuggestionMode` from a known field and current value text.
 * Used in field-mode draft where the field is already determined.
 */
export function fieldToSuggestionMode(op: OperatorDef, value: string): SuggestionMode {
  switch (op.category) {
    case 'relation':
      return { kind: 'relation', field: op.name, value };
    case 'enum':
      return { kind: 'enum', field: op.name, value, options: op.values! };
    case 'boolean':
      return { kind: 'boolean', field: op.name, value };
    case 'presence':
      return { kind: 'presence', field: op.name, value, options: op.values! };
    case 'language':
      return { kind: 'language', value };
    case 'text':
    case 'range':
    case 'identifier':
      return { kind: 'freetext' };
  }
}
