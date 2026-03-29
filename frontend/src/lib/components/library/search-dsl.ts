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
const OPERATOR_LOOKUP: Map<string, OperatorDef> = new Map();
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

interface ParsedToken {
  negated: boolean;
  field: string | null;
  /** The raw field text as written (before canonical lookup). */
  fieldRaw: string | null;
  value: string;
  /** The unquoted value (strips surrounding `"` if present, handles unclosed quotes). */
  valueUnquoted: string;
}

function parseToken(text: string): ParsedToken {
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

function stripQuotes(s: string): string {
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
