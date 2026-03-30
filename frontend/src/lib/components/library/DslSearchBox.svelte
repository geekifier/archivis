<script lang="ts">
	import { tick } from 'svelte';
	import { api } from '$lib/api/index.js';
	import type { QueryWarning } from '$lib/api/types.js';
	import SearchWarnings from './SearchWarnings.svelte';
	import {
		tokenize,
		parseToken,
		parseTokenAtCursor,
		buildFieldValue,
		getOperatorSuggestions,
		getValueSuggestions,
		getBooleanSuggestions,
		getLanguageSuggestions,
		fieldToSuggestionMode,
		splitQueryIntoChipsAndDraft,
		chipsToQuery,
		commitDraft,
		tokenToChip,
		tokenToDraft,
		OPERATOR_LOOKUP,
		EMPTY_DRAFT,
		type SearchChip,
		type DraftState,
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

	// --- Internal chip + draft state ---
	let chips = $state<SearchChip[]>([]);
	let draft = $state<DraftState>({ ...EMPTY_DRAFT });

	let inputRef = $state<HTMLInputElement | null>(null);
	let containerRef = $state<HTMLDivElement | null>(null);
	let dropdownOpen = $state(false);
	let highlightIndex = $state(-1);
	let suggestions = $state<SuggestionItem[]>([]);
	let loading = $state(false);

	// Tracks the last value we serialized ourselves, to distinguish self-updates
	// from external parent changes.
	let lastSelfSerializedValue = '';

	// Stale-request protection for async relation lookups.
	let relationRequestId = 0;
	const RELATION_DEBOUNCE_MS = 200;
	let relationTimer: ReturnType<typeof setTimeout> | undefined;

	// --- Value synchronization ---

	function syncValue() {
		const serialized = chipsToQuery(chips, draft);
		if (serialized !== value) {
			value = serialized;
			lastSelfSerializedValue = serialized;
			onchange?.(value);
		} else if (serialized !== lastSelfSerializedValue) {
			lastSelfSerializedValue = serialized;
		}
	}

	// Detect external value changes (parent sets value, URL restoration)
	$effect(() => {
		const v = value;
		if (v !== lastSelfSerializedValue) {
			const result = splitQueryIntoChipsAndDraft(v);
			chips = result.chips;
			draft = result.draft;
			lastSelfSerializedValue = v;
		}
	});

	// --- Relation debounce helpers ---

	function invalidateRelation() {
		if (relationTimer) {
			clearTimeout(relationTimer);
			relationTimer = undefined;
		}
		relationRequestId++;
		loading = false;
	}

	// --- Draft mode transitions ---

	/**
	 * In bare mode, check if `draft.valueText` contains a recognized `field:`
	 * prefix and promote to field mode.
	 */
	function tryPromoteToFieldMode(): boolean {
		if (draft.field !== null) return false;

		let text = draft.valueText;
		let negated = draft.negated;
		if (text.startsWith('-')) {
			negated = true;
			text = text.slice(1);
		}

		const colonIdx = text.indexOf(':');
		if (colonIdx <= 0) return false;

		const fieldCandidate = text.slice(0, colonIdx).toLowerCase();
		const op = OPERATOR_LOOKUP.get(fieldCandidate);
		if (!op) return false;

		draft = {
			negated,
			field: op.name,
			fieldRaw: text.slice(0, colonIdx),
			valueText: text.slice(colonIdx + 1)
		};
		return true;
	}

	/**
	 * In bare mode, check if the input contains multiple whitespace-delimited
	 * tokens and commit all but the last.
	 */
	function tryAutoCommitInBareMode() {
		if (draft.field !== null) return;

		const fullText = (draft.negated ? '-' : '') + draft.valueText;
		const tokens = tokenize(fullText);
		if (tokens.length < 2) return;

		const toCommit = tokens.slice(0, -1);
		const last = tokens[tokens.length - 1];

		for (const t of toCommit) {
			chips = [...chips, tokenToChip(t)];
		}

		// Last token: check if it triggers field mode
		const lastParsed = parseToken(last.text);
		const op = lastParsed.field ? OPERATOR_LOOKUP.get(lastParsed.field) : null;
		if (op) {
			draft = {
				negated: lastParsed.negated,
				field: op.name,
				fieldRaw: lastParsed.fieldRaw,
				valueText: lastParsed.valueUnquoted
			};
		} else {
			draft = {
				negated: lastParsed.negated,
				field: null,
				fieldRaw: null,
				valueText: lastParsed.value
			};
		}
	}

	function commitCurrentDraft() {
		const trimmed = draft.valueText.trim();
		if (!trimmed && !draft.field) return;
		if (draft.field && !trimmed) return;

		const d = { ...draft, valueText: trimmed };
		chips = [...chips, commitDraft(d)];
		draft = { ...EMPTY_DRAFT };
	}

	function removeChip(chipId: string) {
		chips = chips.filter((c) => c.id !== chipId);
		syncValue();
		tick().then(() => inputRef?.focus());
	}

	// --- Suggestion system ---

	function updateSuggestions() {
		if (draft.field) {
			const op = OPERATOR_LOOKUP.get(draft.field);
			if (!op) {
				setSuggestions([]);
				return;
			}
			applySuggestionMode(fieldToSuggestionMode(op, draft.valueText));
		} else {
			const cursorPos = inputRef?.selectionStart ?? draft.valueText.length;
			const result = parseTokenAtCursor(draft.valueText, cursorPos);
			applySuggestionMode(result.mode);
		}
	}

	function applySuggestionMode(mode: SuggestionMode) {
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

		if (relationTimer) {
			clearTimeout(relationTimer);
			relationTimer = undefined;
		}

		loading = true;
		const thisId = ++relationRequestId;

		relationTimer = setTimeout(async () => {
			try {
				const items = await searchRelation(field, searchValue);
				if (thisId !== relationRequestId) return;
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

	// --- Suggestion selection ---

	function selectSuggestion(item: SuggestionItem) {
		const isOperator = item.insertText.endsWith(':');

		if (isOperator) {
			// Transition to field mode — no chip created yet
			const fieldName = item.insertText.slice(0, -1);
			const op = OPERATOR_LOOKUP.get(fieldName);
			draft = {
				negated: draft.negated,
				field: op?.name ?? fieldName,
				fieldRaw: fieldName,
				valueText: ''
			};
			dropdownOpen = false;
			syncValue();
			tick().then(() => {
				inputRef?.focus();
				updateSuggestions();
			});
		} else {
			// Value selected — create chip directly from the suggestion
			const parsed = parseToken(item.insertText);
			const op = parsed.field ? OPERATOR_LOOKUP.get(parsed.field) : null;

			const chip: SearchChip = {
				id: `chip-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
				kind: op ? 'field' : 'text',
				raw: (draft.negated ? '-' : '') + item.insertText,
				negated: draft.negated,
				field: op?.name ?? null,
				fieldRaw: parsed.fieldRaw,
				displayValue: parsed.field ? parsed.valueUnquoted : parsed.value
			};

			chips = [...chips, chip];
			draft = { ...EMPTY_DRAFT };
			dropdownOpen = false;
			syncValue();
			tick().then(() => inputRef?.focus());
		}
	}

	// --- Event handlers ---

	function handleInput() {
		if (draft.field === null) {
			if (!tryPromoteToFieldMode()) {
				tryAutoCommitInBareMode();
			}
		}
		syncValue();
		updateSuggestions();
	}

	function handleKeydown(e: KeyboardEvent) {
		// Dropdown navigation
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

		// Backspace handling
		if (e.key === 'Backspace') {
			if (draft.field !== null && draft.valueText === '') {
				// Field mode with empty value → dissolve to bare mode
				e.preventDefault();
				draft = {
					negated: draft.negated,
					field: null,
					fieldRaw: null,
					valueText: (draft.negated ? '-' : '') + (draft.fieldRaw ?? draft.field)
				};
				syncValue();
				tick().then(() => {
					inputRef?.focus();
					const len = draft.valueText.length;
					inputRef?.setSelectionRange(len, len);
					updateSuggestions();
				});
				return;
			}

			if (draft.field === null && draft.valueText === '' && chips.length > 0) {
				// Bare mode, empty input → dissolve last chip into draft
				e.preventDefault();
				const last = chips[chips.length - 1];
				chips = chips.slice(0, -1);
				const token: TokenSpan = { start: 0, end: last.raw.length, text: last.raw };
				draft = tokenToDraft(token);
				syncValue();
				tick().then(() => {
					inputRef?.focus();
					const len = draft.valueText.length;
					inputRef?.setSelectionRange(len, len);
					updateSuggestions();
				});
				return;
			}
		}

		// Enter with no highlighted suggestion → commit + submit
		if (e.key === 'Enter') {
			commitCurrentDraft();
			dropdownOpen = false;
			syncValue();
			onsubmit?.();
		}
	}

	function handlePaste(e: ClipboardEvent) {
		if (draft.field !== null) {
			// Field mode: let the browser paste into `valueText` naturally.
			// Multi-word values just accumulate.
			return;
		}

		// Bare mode: intercept and split into chips + draft
		const pastedText = e.clipboardData?.getData('text');
		if (!pastedText) return;

		e.preventDefault();

		const combined = draft.valueText + pastedText;
		const result = splitQueryIntoChipsAndDraft(combined);
		chips = [...chips, ...result.chips];
		draft = result.draft;
		syncValue();
		tick().then(() => {
			inputRef?.focus();
			updateSuggestions();
		});
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

	function handleWarningPickInternal(field: string, query: string, id: string, name: string) {
		onWarningPick?.(field, query, id, name);
	}

	function handleContainerClick(e: MouseEvent) {
		const target = e.target as HTMLElement;
		if (!target.closest('[data-chip-remove]')) {
			inputRef?.focus();
		}
	}
</script>

<div class="relative" bind:this={containerRef}>
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="border-input bg-background dark:bg-input/30 relative flex min-h-9 w-full flex-wrap items-center gap-1 rounded-md border py-1 pl-9 pr-3 shadow-xs transition-[color,box-shadow] focus-within:border-ring focus-within:ring-ring/50 focus-within:ring-[3px]"
		onclick={handleContainerClick}
	>
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

		{#each chips as chip (chip.id)}
			<span
				class="inline-flex items-center gap-0.5 rounded-md bg-primary/10 py-0.5 pl-1.5 pr-0.5 text-xs font-medium text-primary"
			>
				{#if chip.negated}<span class="font-bold text-destructive">-</span>{/if}
				{#if chip.field}
					<span class="text-muted-foreground">{chip.field}:</span>
					<span>{chip.displayValue}</span>
				{:else}
					<span>{chip.displayValue}</span>
				{/if}
				<button
					type="button"
					data-chip-remove
					class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
					aria-label="Remove {chip.field ? chip.field + ':' + chip.displayValue : chip.displayValue}"
					onmousedown={(e) => {
						e.preventDefault();
						removeChip(chip.id);
					}}
				>
					<svg
						class="size-2.5"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<path d="M18 6 6 18" /><path d="m6 6 12 12" />
					</svg>
				</button>
			</span>
		{/each}

		<!-- Draft area: single input element, wrapped in a chip-like border when in field mode.
		     Uses `display: contents` in bare mode so the span is invisible to layout. -->
		<span
			class={draft.field
				? 'inline-flex items-center gap-0.5 rounded-md border border-ring/50 bg-primary/5 py-0.5 pl-1.5 text-xs'
				: 'contents'}
		>
			{#if draft.field}
				{#if draft.negated}<span class="font-bold text-destructive">-</span>{/if}
				<span class="font-medium text-muted-foreground">{draft.field}:</span>
			{/if}
			<input
				bind:this={inputRef}
				type="text"
				bind:value={draft.valueText}
				placeholder={draft.field
					? 'type value...'
					: chips.length === 0
						? placeholder
						: ''}
				oninput={handleInput}
				onkeydown={handleKeydown}
				onfocus={handleFocus}
				onblur={handleBlur}
				onpaste={handlePaste}
				class="min-w-[60px] flex-1 bg-transparent outline-none {draft.field
					? 'max-w-full pr-1 text-xs font-medium text-primary placeholder:text-muted-foreground/60'
					: 'text-base placeholder:text-muted-foreground md:text-sm'}"
				role="combobox"
				aria-expanded={dropdownOpen}
				aria-controls="dsl-suggestions"
				aria-activedescendant={highlightIndex >= 0
					? `dsl-suggestion-${highlightIndex}`
					: undefined}
				autocomplete="off"
			/>
		</span>
	</div>

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
