<script lang="ts">
	import { tick } from 'svelte';
	import { api } from '$lib/api/index.js';
	import type { QueryWarning } from '$lib/api/types.js';
	import SearchWarnings from './SearchWarnings.svelte';
	import {
		parseTokenAtCursor,
		replaceTokenInQuery,
		buildFieldValue,
		getOperatorSuggestions,
		getValueSuggestions,
		getBooleanSuggestions,
		getLanguageSuggestions,
		type SuggestionItem,
		type SuggestionMode,
		type TokenSpan
	} from './search-dsl.js';

	interface Props {
		value: string;
		placeholder?: string;
		warnings?: QueryWarning[];
		showWarnings?: boolean;
		onWarningPick?: (field: string, query: string, id: string, name: string) => void;
		onchange?: (value: string) => void;
		onsubmit?: () => void;
	}

	let {
		value = $bindable(),
		placeholder = 'Search books...',
		warnings = [],
		showWarnings = true,
		onWarningPick,
		onchange,
		onsubmit
	}: Props = $props();

	let inputRef = $state<HTMLInputElement | null>(null);
	let containerRef = $state<HTMLDivElement | null>(null);
	let dropdownOpen = $state(false);
	let highlightIndex = $state(-1);
	let suggestions = $state<SuggestionItem[]>([]);
	let activeToken = $state<TokenSpan | null>(null);
	let loading = $state(false);

	// Stale-request protection for async relation lookups.
	// Incremented both when a new relation request starts AND when leaving relation mode,
	// so any in-flight response from a previous context is always rejected.
	let relationRequestId = 0;

	const RELATION_DEBOUNCE_MS = 200;
	let relationTimer: ReturnType<typeof setTimeout> | undefined;

	/** Cancel any pending relation lookup and invalidate in-flight responses. */
	function invalidateRelation() {
		if (relationTimer) {
			clearTimeout(relationTimer);
			relationTimer = undefined;
		}
		relationRequestId++;
		loading = false;
	}

	function updateSuggestions() {
		const cursorPos = inputRef?.selectionStart ?? value.length;
		const result = parseTokenAtCursor(value, cursorPos);
		activeToken = result.token;
		applySuggestionMode(result.mode);
	}

	function applySuggestionMode(mode: SuggestionMode) {
		// Any non-relation mode must invalidate pending relation lookups so stale
		// responses cannot reopen the dropdown after the context changed.
		if (mode.kind !== 'relation') {
			invalidateRelation();
		}

		switch (mode.kind) {
			case 'operators': {
				const items = getOperatorSuggestions(mode.prefix);
				setSuggestions(items);
				break;
			}
			case 'enum': {
				const items = getValueSuggestions(mode.field, mode.value, mode.options);
				setSuggestions(items);
				break;
			}
			case 'boolean': {
				const items = getBooleanSuggestions(mode.field, mode.value);
				setSuggestions(items);
				break;
			}
			case 'presence': {
				const items = getValueSuggestions(mode.field, mode.value, mode.options);
				setSuggestions(items);
				break;
			}
			case 'language': {
				const items = getLanguageSuggestions(mode.value);
				setSuggestions(items);
				break;
			}
			case 'relation': {
				fetchRelationSuggestions(mode.field, mode.value);
				break;
			}
			case 'freetext':
			case 'none':
				setSuggestions([]);
				break;
		}
	}

	function setSuggestions(items: SuggestionItem[]) {
		suggestions = items;
		highlightIndex = -1;
		dropdownOpen = items.length > 0;
	}

	async function fetchRelationSuggestions(field: string, searchValue: string) {
		if (!searchValue) {
			invalidateRelation();
			setSuggestions([]);
			return;
		}

		// Cancel any previous debounce timer but keep loading true
		if (relationTimer) {
			clearTimeout(relationTimer);
			relationTimer = undefined;
		}

		loading = true;
		const thisId = ++relationRequestId;

		relationTimer = setTimeout(async () => {
			try {
				const items = await searchRelation(field, searchValue);
				if (thisId !== relationRequestId) return; // stale
				setSuggestions(items);
			} catch {
				if (thisId !== relationRequestId) return;
				setSuggestions([]);
			} finally {
				if (thisId === relationRequestId) {
					loading = false;
				}
			}
		}, RELATION_DEBOUNCE_MS);
	}

	async function searchRelation(field: string, q: string): Promise<SuggestionItem[]> {
		switch (field) {
			case 'author': {
				const result = await api.authors.search(q);
				return result.items.map((a) => ({
					id: `author:${a.id}`,
					label: a.name,
					sublabel: `${a.book_count} book${a.book_count === 1 ? '' : 's'}`,
					insertText: buildFieldValue('author', a.name)
				}));
			}
			case 'series': {
				const result = await api.series.search(q);
				return result.items.map((s) => ({
					id: `series:${s.id}`,
					label: s.name,
					sublabel: `${s.book_count} book${s.book_count === 1 ? '' : 's'}`,
					insertText: buildFieldValue('series', s.name)
				}));
			}
			case 'publisher': {
				const result = await api.publishers.search(q);
				return result.items.map((p) => ({
					id: `publisher:${p.id}`,
					label: p.name,
					insertText: buildFieldValue('publisher', p.name)
				}));
			}
			case 'tag': {
				const result = await api.tags.search(q);
				return result.items.map((t) => ({
					id: `tag:${t.id}`,
					label: t.name,
					sublabel: t.category ?? undefined,
					insertText: buildFieldValue('tag', t.name)
				}));
			}
			default:
				return [];
		}
	}

	function selectSuggestion(item: SuggestionItem) {
		if (!activeToken) {
			// No token context (e.g. empty input with operator selected)
			const isOperator = item.insertText.endsWith(':');
			value = item.insertText + (isOperator ? '' : ' ');
			const cursorPos = value.length;
			onchange?.(value);
			dropdownOpen = false;
			tick().then(() => {
				inputRef?.setSelectionRange(cursorPos, cursorPos);
				inputRef?.focus();
			});
			return;
		}

		const { newQuery, newCursorPos } = replaceTokenInQuery(
			value,
			activeToken,
			item.insertText
		);

		// If this is an operator (ends with `:`) don't add trailing space
		const isOperator = item.insertText.endsWith(':');
		if (isOperator) {
			value = newQuery;
		} else {
			// Ensure trailing space after value completion
			const beforeCursor = newQuery.slice(0, newCursorPos);
			const afterCursor = newQuery.slice(newCursorPos);
			if (!afterCursor.startsWith(' ') && afterCursor.length > 0) {
				value = beforeCursor + ' ' + afterCursor;
			} else if (afterCursor.length === 0 && !beforeCursor.endsWith(' ')) {
				value = beforeCursor + ' ';
			} else {
				value = newQuery;
			}
		}

		onchange?.(value);
		dropdownOpen = false;

		const finalCursorPos = isOperator ? newCursorPos : Math.min(newCursorPos + 1, value.length);
		tick().then(() => {
			inputRef?.setSelectionRange(finalCursorPos, finalCursorPos);
			inputRef?.focus();
			// After selecting an operator, immediately show value suggestions
			if (isOperator) {
				updateSuggestions();
			}
		});
	}

	function handleInput() {
		onchange?.(value);
		updateSuggestions();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (dropdownOpen && suggestions.length > 0) {
			if (e.key === 'ArrowDown') {
				e.preventDefault();
				highlightIndex = Math.min(highlightIndex + 1, suggestions.length - 1);
				return;
			}
			if (e.key === 'ArrowUp') {
				e.preventDefault();
				highlightIndex = Math.max(highlightIndex - 1, -1);
				return;
			}
			if (e.key === 'Enter' && highlightIndex >= 0) {
				e.preventDefault();
				selectSuggestion(suggestions[highlightIndex]);
				return;
			}
			if (e.key === 'Tab' && highlightIndex >= 0) {
				e.preventDefault();
				selectSuggestion(suggestions[highlightIndex]);
				return;
			}
			if (e.key === 'Escape') {
				e.preventDefault();
				dropdownOpen = false;
				return;
			}
		}

		// Enter with no highlighted suggestion → fire search
		if (e.key === 'Enter') {
			dropdownOpen = false;
			onsubmit?.();
		}
	}

	function handleFocus() {
		updateSuggestions();
	}

	function handleBlur(e: FocusEvent) {
		const related = e.relatedTarget as Node | null;
		if (containerRef && related && containerRef.contains(related)) return;
		setTimeout(() => {
			dropdownOpen = false;
		}, 150);
	}

	function handleCursorMove() {
		updateSuggestions();
	}

	function handleWarningPickInternal(field: string, query: string, id: string, name: string) {
		onWarningPick?.(field, query, id, name);
	}
</script>

<div class="relative" bind:this={containerRef}>
	<svg
		class="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
		xmlns="http://www.w3.org/2000/svg"
		viewBox="0 0 24 24"
		fill="none"
		stroke="currentColor"
		stroke-width="2"
		stroke-linecap="round"
		stroke-linejoin="round"
	>
		<circle cx="11" cy="11" r="8" />
		<path d="m21 21-4.3-4.3" />
	</svg>
	<input
		bind:this={inputRef}
		type="search"
		{placeholder}
		bind:value
		oninput={handleInput}
		onkeydown={handleKeydown}
		onfocus={handleFocus}
		onblur={handleBlur}
		onclick={handleCursorMove}
		onselect={handleCursorMove}
		role="combobox"
		aria-expanded={dropdownOpen}
		aria-controls="dsl-suggestions"
		aria-activedescendant={highlightIndex >= 0 ? `dsl-suggestion-${highlightIndex}` : undefined}
		autocomplete="off"
		class="border-input bg-background selection:bg-primary dark:bg-input/30 selection:text-primary-foreground ring-offset-background placeholder:text-muted-foreground flex h-9 w-full min-w-0 rounded-md border py-1 pl-9 pr-3 text-base shadow-xs transition-[color,box-shadow] outline-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]"
	/>

	{#if dropdownOpen && (suggestions.length > 0 || loading)}
		<div
			id="dsl-suggestions"
			role="listbox"
			class="absolute z-50 mt-1 max-h-56 w-full overflow-y-auto rounded-md border border-border bg-popover shadow-md"
		>
			{#if loading && suggestions.length === 0}
				<div class="px-3 py-2 text-sm text-muted-foreground">Searching...</div>
			{:else}
				{#each suggestions as item, i (item.id)}
					<button
						id="dsl-suggestion-{i}"
						type="button"
						role="option"
						aria-selected={i === highlightIndex}
						class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent {i ===
						highlightIndex
							? 'bg-accent'
							: ''}"
						onmousedown={(e) => {
							e.preventDefault();
							selectSuggestion(item);
						}}
					>
						<span class="font-medium">{item.label}</span>
						{#if item.sublabel}
							<span class="truncate text-xs text-muted-foreground">{item.sublabel}</span>
						{/if}
					</button>
				{/each}
			{/if}
		</div>
	{/if}

	{#if showWarnings && warnings.length > 0}
		<div class="mt-1.5">
			<SearchWarnings warnings={warnings} onpick={handleWarningPickInternal} />
		</div>
	{/if}
</div>
