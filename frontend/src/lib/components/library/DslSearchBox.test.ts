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
	tokenize
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
		// "author:asimov title:dune format:epub"
		// cursor at position 21 → inside "title:dune"
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

	it('resolves olid: alias to open_library_id freetext', () => {
		const result = parseTokenAtCursor('olid:OL123', 10);
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
	type OnWarningPickFn = (field: string, query: string, id: string, name: string) => void;

	let onchangeFn: ReturnType<typeof vi.fn<OnChangeFn>>;
	let onsubmitFn: ReturnType<typeof vi.fn<OnSubmitFn>>;
	let onWarningPickFn: ReturnType<typeof vi.fn<OnWarningPickFn>>;

	beforeEach(() => {
		onchangeFn = vi.fn<OnChangeFn>();
		onsubmitFn = vi.fn<OnSubmitFn>();
		onWarningPickFn = vi.fn<OnWarningPickFn>();
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
		// Search icon is an SVG with a circle element
		expect(document.querySelector('svg circle')).toBeInTheDocument();
	});

	it('shows operator suggestions when typing a prefix', async () => {
		const user = userEvent.setup();
		renderBox();

		const input = screen.getByPlaceholderText('Search books...');
		await user.type(input, 'au');

		// Should show `author:` in the dropdown
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
		// The input value should end with `author:`
		const lastCallValue = onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0];
		expect(lastCallValue).toContain('author:');
	});

	it('selects a suggestion via ArrowDown+Enter', async () => {
		const user = userEvent.setup();
		renderBox();

		const input = screen.getByPlaceholderText('Search books...');
		await user.type(input, 'format:ep');

		// ArrowDown to highlight first item, Enter to select
		await user.keyboard('{ArrowDown}');
		await user.keyboard('{Enter}');

		const lastCallValue = onchangeFn.mock.calls[onchangeFn.mock.calls.length - 1][0];
		expect(lastCallValue).toContain('format:epub');
	});

	it('calls onsubmit on Enter with no highlighted suggestion', async () => {
		const user = userEvent.setup();
		renderBox();

		const input = screen.getByPlaceholderText('Search books...');
		// Type something that won't match any operator (so dropdown is closed or no highlight)
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
					query: 'nonexistent'
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
					query: 'nonexistent'
				}
			],
			showWarnings: false
		});

		expect(screen.queryByText(/No author found matching/)).not.toBeInTheDocument();
	});

	it('fires warning pick callback with field, query, id, and name', async () => {
		const user = userEvent.setup();
		renderBox({
			warnings: [
				{
					type: 'ambiguous_relation' as const,
					field: 'author',
					query: 'smith',
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

		expect(onWarningPickFn).toHaveBeenCalledWith('author', 'smith', 'a1', 'John Smith');
	});

	it('distinguishes repeated same-field warnings', async () => {
		const user = userEvent.setup();
		renderBox({
			warnings: [
				{
					type: 'ambiguous_relation' as const,
					field: 'tag',
					query: 'sci',
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
					match_count: 2,
					matches: [
						{ id: 't3', name: 'Fantasy' },
						{ id: 't4', name: 'Fan Fiction' }
					]
				}
			],
			showWarnings: true
		});

		// Pick from the first warning
		const scienceBtn = screen.getByText('Science');
		await user.click(scienceBtn);
		expect(onWarningPickFn).toHaveBeenCalledWith('tag', 'sci', 't1', 'Science');

		// Pick from the second warning
		const fantasyBtn = screen.getByText('Fantasy');
		await user.click(fantasyBtn);
		expect(onWarningPickFn).toHaveBeenCalledWith('tag', 'fan', 't3', 'Fantasy');
	});

	it('fetches relation suggestions with stale-request protection (relation→relation)', async () => {
		vi.useFakeTimers();
		const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
		const { api } = await import('$lib/api/index.js');

		// First call resolves slowly, second resolves quickly
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

		// Type `author:a` (triggers relation mode)
		await user.type(input, 'author:a');
		await vi.advanceTimersByTimeAsync(200); // relation debounce

		// Now type more: `author:asi`
		await user.type(input, 'si');
		await vi.advanceTimersByTimeAsync(200); // second debounce fires

		// Let the second resolve first
		await vi.advanceTimersByTimeAsync(0); // microtask

		// Now resolve the first (stale) response
		resolveFirst!({
			items: [
				{ id: 'stale', name: 'Stale Result', sort_name: 'Stale', book_count: 0 }
			],
			total: 1,
			page: 1,
			per_page: 10,
			total_pages: 1
		});
		await vi.advanceTimersByTimeAsync(0);

		// The stale result should NOT appear
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

		// Type `author:a` → relation mode, starts debounced API call
		await user.type(input, 'author:a');
		await vi.advanceTimersByTimeAsync(200); // debounce fires, API call in flight

		// User clears and switches to `format:` → enum mode
		await user.clear(input);
		await user.type(input, 'format:');

		// Enum suggestions should be showing
		expect(screen.getByText('epub')).toBeInTheDocument();

		// Now the stale author response arrives
		resolveAuthor!({
			items: [{ id: 'a1', name: 'Isaac Asimov', sort_name: 'Asimov', book_count: 5 }],
			total: 1,
			page: 1,
			per_page: 10,
			total_pages: 1
		});
		await vi.advanceTimersByTimeAsync(0);

		// Stale author result must NOT appear; enum suggestions should still be showing
		expect(screen.queryByText('Isaac Asimov')).not.toBeInTheDocument();
		expect(screen.getByText('epub')).toBeInTheDocument();

		vi.useRealTimers();
	});

	it('discards stale relation response when cursor moves to a different token', async () => {
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

		renderBox({ value: 'author:a hello' });
		const input = screen.getByPlaceholderText('Search books...');

		// Focus and place cursor at end of `author:a` (position 8)
		await user.click(input);
		// The input starts with `author:a hello`, cursor at end by default.
		// Move cursor to position 8 (end of `author:a`) to trigger relation mode.
		(input as HTMLInputElement).setSelectionRange(8, 8);
		input.dispatchEvent(new Event('select'));
		await vi.advanceTimersByTimeAsync(200); // debounce fires

		// Now move cursor to the `hello` token (position 14)
		(input as HTMLInputElement).setSelectionRange(14, 14);
		input.dispatchEvent(new Event('select'));

		// `hello` is a bare word → operators mode now; relation is invalidated

		// Stale author response arrives
		resolveAuthor!({
			items: [{ id: 'a1', name: 'Isaac Asimov', sort_name: 'Asimov', book_count: 5 }],
			total: 1,
			page: 1,
			per_page: 10,
			total_pages: 1
		});
		await vi.advanceTimersByTimeAsync(0);

		// Stale author result must not appear
		expect(screen.queryByText('Isaac Asimov')).not.toBeInTheDocument();

		vi.useRealTimers();
	});

	it('hides warnings when showWarnings changes from true to false', async () => {
		const { rerender } = render(DslSearchBox, {
			props: {
				value: 'author:smith',
				warnings: [
					{
						type: 'unknown_relation' as const,
						field: 'author',
						query: 'smith'
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
					query: 'smith'
				}
			],
			showWarnings: false,
			onchange: onchangeFn,
			onsubmit: onsubmitFn
		});

		expect(screen.queryByText(/No author found matching/)).not.toBeInTheDocument();
	});
});
