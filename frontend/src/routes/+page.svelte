<script lang="ts">
	import { SvelteSet, SvelteURLSearchParams } from 'svelte/reactivity';
	import { api, type PaginatedBooks } from '$lib/api/index.js';
	import type { SortField, SortOrder } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
	import { filters } from '$lib/stores/filters.svelte.js';
	import BookCard from '$lib/components/library/BookCard.svelte';
	import BookListView from '$lib/components/library/BookListView.svelte';
	import Pagination from '$lib/components/library/Pagination.svelte';
	import BatchEditPanel from '$lib/components/library/BatchEditPanel.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import type { BookFormat, MetadataStatus } from '$lib/api/types.js';

	const PER_PAGE = 24;
	const DEBOUNCE_MS = 300;
	const VIEW_STORAGE_KEY = 'archivis-library-view';

	type ViewMode = 'grid' | 'list';
	type SortOption = { label: string; field: SortField; order: SortOrder };

	const sortOptions: SortOption[] = [
		{ label: 'Recently Added', field: 'added_at', order: 'desc' },
		{ label: 'Title A\u2013Z', field: 'title', order: 'asc' },
		{ label: 'Title Z\u2013A', field: 'title', order: 'desc' },
		{ label: 'Highest Rated', field: 'rating', order: 'desc' }
	];

	function loadViewPreference(): ViewMode {
		if (typeof localStorage === 'undefined') return 'grid';
		const stored = localStorage.getItem(VIEW_STORAGE_KEY);
		return stored === 'list' ? 'list' : 'grid';
	}

	// Restore state from URL search params (supports browser back-navigation)
	const _params = page.url.searchParams;
	const _initPage = Math.max(1, parseInt(_params.get('page') || '1', 10) || 1);
	const _initQuery = _params.get('q') || '';
	const _initSort = _params.get('sort') as SortField | null;
	const _initOrder = _params.get('order') as SortOrder | null;
	const _initSortIdx =
		_initSort && _initOrder
			? sortOptions.findIndex((o) => o.field === _initSort && o.order === _initOrder)
			: -1;
	const _initFormat = _params.get('format') as BookFormat | null;
	const _initStatus = _params.get('status') as MetadataStatus | null;

	filters.clearFilters();
	if (_initFormat) filters.setFormat(_initFormat);
	if (_initStatus) filters.setStatus(_initStatus);

	let viewMode = $state<ViewMode>(loadViewPreference());
	let searchInput = $state(_initQuery);
	let activeQuery = $state(_initQuery);
	let sortIndex = $state(_initSortIdx >= 0 ? _initSortIdx : 0);
	let currentPage = $state(_initPage);
	let loading = $state(true);
	let data = $state<PaginatedBooks | null>(null);
	let error = $state<string | null>(null);
	let debounceTimer: ReturnType<typeof setTimeout> | undefined;

	let activeSortBy = $state<SortField>(_initSort || sortOptions[0].field);
	let activeSortOrder = $state<SortOrder>(_initOrder || sortOptions[0].order);

	// --- Identify All state ---
	let identifyAllDialogOpen = $state(false);
	let identifyingAll = $state(false);
	let identifyAllError = $state<string | null>(null);
	let identifyAllCompleted = $state(0);
	let identifyAllTotal = $state(0);
	let identifyAllEventSources: EventSource[] = [];

	// --- Selection mode state ---
	let selectionMode = $state(false);
	let selectedIds = new SvelteSet<string>();
	let lastClickedId = $state<string | null>(null);
	let batchEditOpen = $state(false);

	const selectedCount = $derived(selectedIds.size);
	const selectedArray = $derived(Array.from(selectedIds));

	const includeParam = $derived(viewMode === 'list' ? 'authors,series,files' : 'authors,files');

	const showIdentifyAll = $derived(
		filters.activeStatus === 'needs_review' || filters.activeStatus === 'unidentified'
	);

	function setViewMode(mode: ViewMode) {
		viewMode = mode;
		localStorage.setItem(VIEW_STORAGE_KEY, mode);
	}

	function handleSearchInput(e: Event) {
		const value = (e.target as HTMLInputElement).value;
		searchInput = value;
		clearTimeout(debounceTimer);
		debounceTimer = setTimeout(() => {
			activeQuery = value.trim();
		}, DEBOUNCE_MS);
	}

	function handleSearchKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			clearTimeout(debounceTimer);
			activeQuery = searchInput.trim();
		}
	}

	function handleSortChange(e: Event) {
		sortIndex = Number((e.target as HTMLSelectElement).value);
		activeSortBy = sortOptions[sortIndex].field;
		activeSortOrder = sortOptions[sortIndex].order;
	}

	function handlePageChange(page: number) {
		currentPage = page;
	}

	function handleListSort(field: SortField, order: SortOrder) {
		activeSortBy = field;
		activeSortOrder = order;
		// Sync dropdown if it matches a preset
		const idx = sortOptions.findIndex((o) => o.field === field && o.order === order);
		if (idx >= 0) sortIndex = idx;
	}

	// --- Selection mode ---

	function toggleSelectionMode() {
		selectionMode = !selectionMode;
		if (!selectionMode) {
			selectedIds.clear();
			lastClickedId = null;
		}
	}

	function exitSelectionMode() {
		selectionMode = false;
		selectedIds.clear();
		lastClickedId = null;
	}

	function handleBookSelect(bookId: string, event?: MouseEvent) {
		if (event?.shiftKey && lastClickedId && data) {
			// Range selection: select all books between lastClickedId and bookId
			const items = data.items;
			const lastIdx = items.findIndex((b) => b.id === lastClickedId);
			const currentIdx = items.findIndex((b) => b.id === bookId);
			if (lastIdx >= 0 && currentIdx >= 0) {
				const start = Math.min(lastIdx, currentIdx);
				const end = Math.max(lastIdx, currentIdx);
				for (let i = start; i <= end; i++) {
					selectedIds.add(items[i].id);
				}
			}
		} else {
			// Toggle single selection
			if (selectedIds.has(bookId)) {
				selectedIds.delete(bookId);
			} else {
				selectedIds.add(bookId);
			}
		}

		lastClickedId = bookId;
	}

	function handleCardSelect(bookId: string, event: MouseEvent) {
		handleBookSelect(bookId, event);
	}

	function selectAll() {
		if (!data) return;
		for (const book of data.items) {
			selectedIds.add(book.id);
		}
	}

	function deselectAll() {
		selectedIds.clear();
	}

	function handleBatchApply() {
		// Refresh the book list and exit selection mode
		exitSelectionMode();
		// Trigger refresh by reassigning currentPage
		currentPage = currentPage;
	}

	// Reset to page 1 when search, sort, or filters change (but not on initial mount)
	let _prevQuery = _initQuery;
	let _prevSortBy: SortField = _initSort || sortOptions[0].field;
	let _prevSortOrder: SortOrder = _initOrder || sortOptions[0].order;
	let _prevFormat: BookFormat | null = _initFormat;
	let _prevStatus: MetadataStatus | null = _initStatus;

	$effect(() => {
		const q = activeQuery;
		const sb = activeSortBy;
		const so = activeSortOrder;
		const fmt = filters.activeFormat;
		const st = filters.activeStatus;

		const changed =
			q !== _prevQuery ||
			sb !== _prevSortBy ||
			so !== _prevSortOrder ||
			fmt !== _prevFormat ||
			st !== _prevStatus;

		_prevQuery = q;
		_prevSortBy = sb;
		_prevSortOrder = so;
		_prevFormat = fmt;
		_prevStatus = st;

		if (changed) {
			currentPage = 1;
		}
	});

	// Fetch books when query params change
	$effect(() => {
		const page = currentPage;
		const field = activeSortBy;
		const order = activeSortOrder;
		const q = activeQuery;
		const include = includeParam;
		const format = filters.activeFormat;
		const status = filters.activeStatus;

		loading = true;
		error = null;

		api.books
			.list({
				page,
				per_page: PER_PAGE,
				sort_by: field,
				sort_order: order,
				q: q || undefined,
				format: format ?? undefined,
				status: status ?? undefined,
				include
			})
			.then((result) => {
				data = result;
			})
			.catch((err) => {
				error = err instanceof Error ? err.message : 'Failed to load books';
			})
			.finally(() => {
				loading = false;
			});
	});

	// Sync state to URL for back-navigation support
	$effect(() => {
		const params = new SvelteURLSearchParams();
		if (currentPage > 1) params.set('page', String(currentPage));
		if (activeQuery) params.set('q', activeQuery);
		if (activeSortBy !== sortOptions[0].field || activeSortOrder !== sortOptions[0].order) {
			params.set('sort', activeSortBy);
			params.set('order', activeSortOrder);
		}
		if (filters.activeFormat) params.set('format', filters.activeFormat);
		if (filters.activeStatus) params.set('status', filters.activeStatus);

		const search = params.toString();
		const url = search ? `/?${search}` : '/';
		goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	});

	// Cleanup SSE on unmount
	$effect(() => {
		return () => {
			for (const es of identifyAllEventSources) {
				es.close();
			}
			identifyAllEventSources = [];
		};
	});

	// --- Identify All ---

	async function handleIdentifyAll() {
		identifyingAll = true;
		identifyAllError = null;
		identifyAllCompleted = 0;
		identifyAllDialogOpen = false;

		try {
			const response = await api.identify.all();
			identifyAllTotal = response.count;

			if (response.count === 0) {
				identifyingAll = false;
				identifyAllError = 'No books found needing identification.';
				return;
			}

			// Subscribe to SSE for each task
			for (const taskId of response.task_ids) {
				subscribeToIdentifyTask(taskId);
			}
		} catch (err) {
			identifyAllError = err instanceof Error ? err.message : 'Failed to start identification';
			identifyingAll = false;
		}
	}

	function subscribeToIdentifyTask(taskId: string) {
		const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);
		identifyAllEventSources.push(es);

		es.addEventListener('task:complete', () => {
			identifyAllCompleted += 1;
			es.close();
			removeIdentifyEventSource(es);
			checkIdentifyAllDone();
		});

		es.addEventListener('task:error', () => {
			identifyAllCompleted += 1;
			es.close();
			removeIdentifyEventSource(es);
			checkIdentifyAllDone();
		});

		es.onerror = () => {
			identifyAllCompleted += 1;
			es.close();
			removeIdentifyEventSource(es);
			checkIdentifyAllDone();
		};
	}

	function removeIdentifyEventSource(es: EventSource) {
		identifyAllEventSources = identifyAllEventSources.filter((e) => e !== es);
	}

	function checkIdentifyAllDone() {
		if (identifyAllCompleted >= identifyAllTotal) {
			identifyingAll = false;
			// Refresh the book list
			currentPage = currentPage;
		}
	}

	function dismissIdentifyAll() {
		identifyAllCompleted = 0;
		identifyAllTotal = 0;
		identifyAllError = null;
	}

	const skeletonIds = Array.from({ length: 12 }, (_, i) => i);
</script>

<div class="space-y-6">
	<div>
		<h1 class="text-3xl font-bold tracking-tight">Library</h1>
		<p class="text-muted-foreground">Your e-book collection</p>
	</div>

	<!-- Controls bar -->
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<div class="relative w-full sm:max-w-xs">
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
			<Input
				type="search"
				placeholder="Search books..."
				value={searchInput}
				oninput={handleSearchInput}
				onkeydown={handleSearchKeydown}
				class="pl-9"
			/>
		</div>

		<div class="flex items-center gap-2">
			{#if showIdentifyAll && data && data.total > 0}
				<Button
					size="sm"
					variant="outline"
					onclick={() => (identifyAllDialogOpen = true)}
					disabled={identifyingAll}
				>
					{#if identifyingAll}
						<svg
							class="size-4 animate-spin"
							xmlns="http://www.w3.org/2000/svg"
							fill="none"
							viewBox="0 0 24 24"
						>
							<circle
								class="opacity-25"
								cx="12"
								cy="12"
								r="10"
								stroke="currentColor"
								stroke-width="4"
							></circle>
							<path
								class="opacity-75"
								fill="currentColor"
								d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
							></path>
						</svg>
						Identifying...
					{:else}
						<svg
							class="size-4"
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
						Identify All
					{/if}
				</Button>
			{/if}

			<!-- Selection mode toggle -->
			<Button
				size="sm"
				variant={selectionMode ? 'default' : 'outline'}
				onclick={toggleSelectionMode}
			>
				{#if selectionMode}
					<svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M18 6 6 18" />
						<path d="m6 6 12 12" />
					</svg>
					Exit Select
				{:else}
					<svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<polyline points="9 11 12 14 22 4" />
						<path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
					</svg>
					Select
				{/if}
			</Button>

			<!-- View toggle -->
			<div class="flex rounded-md border border-input shadow-xs">
				<Button
					variant={viewMode === 'grid' ? 'default' : 'ghost'}
					size="icon-sm"
					onclick={() => setViewMode('grid')}
					aria-label="Grid view"
					class="rounded-r-none border-0 shadow-none"
				>
					<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4">
						<rect x="3" y="3" width="7" height="7" />
						<rect x="14" y="3" width="7" height="7" />
						<rect x="3" y="14" width="7" height="7" />
						<rect x="14" y="14" width="7" height="7" />
					</svg>
				</Button>
				<Button
					variant={viewMode === 'list' ? 'default' : 'ghost'}
					size="icon-sm"
					onclick={() => setViewMode('list')}
					aria-label="List view"
					class="rounded-l-none border-0 shadow-none"
				>
					<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4">
						<line x1="3" y1="6" x2="21" y2="6" />
						<line x1="3" y1="12" x2="21" y2="12" />
						<line x1="3" y1="18" x2="21" y2="18" />
					</svg>
				</Button>
			</div>

			<select
				class="h-9 rounded-md border border-input bg-background px-3 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
				value={sortIndex}
				onchange={handleSortChange}
			>
				{#each sortOptions as option, i (option.field + option.order)}
					<option value={i}>{option.label}</option>
				{/each}
			</select>
		</div>
	</div>

	<!-- Selection toolbar -->
	{#if selectionMode}
		<div class="flex items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-2">
			<span class="text-sm font-medium">
				{selectedCount} selected
			</span>
			<div class="flex items-center gap-1.5">
				<Button size="sm" variant="ghost" class="h-7 text-xs" onclick={selectAll}>
					Select All
				</Button>
				<Button size="sm" variant="ghost" class="h-7 text-xs" onclick={deselectAll} disabled={selectedCount === 0}>
					Deselect All
				</Button>
			</div>
			<div class="flex-1"></div>
			<Button
				size="sm"
				onclick={() => (batchEditOpen = true)}
				disabled={selectedCount === 0}
			>
				<svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
					<path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
				</svg>
				Edit Selected
			</Button>
		</div>
	{/if}

	<!-- Identify All progress -->
	{#if identifyingAll || (identifyAllTotal > 0 && identifyAllCompleted > 0)}
		<div class="rounded-lg border border-border p-3">
			<div class="flex items-center justify-between text-sm">
				<span class="font-medium">
					{#if identifyingAll}
						Identifying books...
					{:else}
						Identification complete
					{/if}
				</span>
				<div class="flex items-center gap-2">
					<span class="text-xs text-muted-foreground">
						{identifyAllCompleted} / {identifyAllTotal}
					</span>
					{#if !identifyingAll}
						<Button
							variant="ghost"
							size="icon-sm"
							class="size-6"
							onclick={dismissIdentifyAll}
							aria-label="Dismiss"
						>
							<svg
								class="size-3"
								xmlns="http://www.w3.org/2000/svg"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<path d="M18 6 6 18" />
								<path d="m6 6 12 12" />
							</svg>
						</Button>
					{/if}
				</div>
			</div>
			<div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
				<div
					class="h-full rounded-full bg-primary transition-all duration-300"
					style="width: {identifyAllTotal > 0 ? (identifyAllCompleted / identifyAllTotal) * 100 : 0}%"
				></div>
			</div>
		</div>
	{/if}

	{#if identifyAllError}
		<div class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
			{identifyAllError}
		</div>
	{/if}

	<!-- Active filter chips -->
	{#if filters.hasActiveFilters}
		<div class="flex flex-wrap items-center gap-2">
			{#if filters.activeFormat}
				<span class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary">
					{filters.activeFormat.toUpperCase()}
					<button
						onclick={() => filters.setFormat(filters.activeFormat!)}
						class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
						aria-label="Remove format filter"
					>
						<svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M18 6 6 18" /><path d="m6 6 12 12" />
						</svg>
					</button>
				</span>
			{/if}
			{#if filters.activeStatus}
				<span class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary">
					{filters.activeStatus === 'needs_review' ? 'Needs Review' : filters.activeStatus === 'identified' ? 'Identified' : 'Unidentified'}
					<button
						onclick={() => filters.setStatus(filters.activeStatus!)}
						class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
						aria-label="Remove status filter"
					>
						<svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M18 6 6 18" /><path d="m6 6 12 12" />
						</svg>
					</button>
				</span>
			{/if}
			<button
				onclick={() => filters.clearFilters()}
				class="text-xs text-muted-foreground transition-colors hover:text-foreground"
			>
				Clear all
			</button>
		</div>
	{/if}

	<!-- Content area -->
	{#if loading}
		{#if viewMode === 'grid'}
			<!-- Skeleton grid -->
			<div class="grid grid-cols-[repeat(auto-fill,minmax(150px,200px))] justify-center gap-4">
				{#each skeletonIds as id (id)}
					<div>
						<div class="aspect-[2/3] w-full animate-pulse rounded-lg bg-muted"></div>
						<div class="mt-1.5 space-y-1 px-0.5">
							<div class="h-4 w-3/4 animate-pulse rounded bg-muted"></div>
							<div class="h-3 w-1/2 animate-pulse rounded bg-muted"></div>
						</div>
					</div>
				{/each}
			</div>
		{:else}
			<!-- Skeleton list -->
			<div class="space-y-0 overflow-hidden rounded-lg border border-border">
				{#each skeletonIds.slice(0, 8) as id (id)}
					<div class="flex items-center gap-3 border-b border-border px-3 py-2 last:border-b-0">
						<div class="h-10 w-7 animate-pulse rounded bg-muted"></div>
						<div class="h-4 w-48 animate-pulse rounded bg-muted"></div>
						<div class="h-4 w-32 animate-pulse rounded bg-muted"></div>
						<div class="h-4 w-24 animate-pulse rounded bg-muted"></div>
					</div>
				{/each}
			</div>
		{/if}
	{:else if error}
		<div class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12">
			<div class="text-center">
				<p class="text-destructive">{error}</p>
				<Button variant="outline" class="mt-4" onclick={() => (currentPage = currentPage)}>
					Retry
				</Button>
			</div>
		</div>
	{:else if data && data.items.length > 0}
		{#if viewMode === 'grid'}
			<div class="grid grid-cols-[repeat(auto-fill,minmax(150px,200px))] justify-center gap-4">
				{#each data.items as book (book.id)}
					<BookCard
						{book}
						{selectionMode}
						selected={selectedIds.has(book.id)}
						onselect={handleCardSelect}
					/>
				{/each}
			</div>
		{:else}
			<BookListView
				books={data.items}
				sortBy={activeSortBy}
				sortOrder={activeSortOrder}
				onSort={handleListSort}
				{selectionMode}
				{selectedIds}
				onselect={(bookId, event) => handleBookSelect(bookId, event)}
			/>
		{/if}

		<Pagination page={data.page} totalPages={data.total_pages} onPageChange={handlePageChange} />
	{:else}
		<!-- Empty state -->
		<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-12">
			<div class="text-center">
				{#if activeQuery || filters.hasActiveFilters}
					<svg class="mx-auto mb-3 size-10 text-muted-foreground/50" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
						<circle cx="11" cy="11" r="8" />
						<path d="m21 21-4.3-4.3" />
					</svg>
					<p class="font-medium text-foreground">No books found</p>
					<p class="mt-1 text-sm text-muted-foreground">No books match your current {activeQuery && filters.hasActiveFilters ? 'search and filters' : activeQuery ? 'search' : 'filters'}.</p>
					<Button variant="outline" class="mt-4" onclick={() => { searchInput = ''; activeQuery = ''; filters.clearFilters(); }}>
						Clear {activeQuery && filters.hasActiveFilters ? 'search & filters' : activeQuery ? 'search' : 'filters'}
					</Button>
				{:else}
					<svg class="mx-auto mb-3 size-10 text-muted-foreground/50" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
						<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20" />
					</svg>
					<p class="font-medium text-foreground">Welcome to Archivis</p>
					<p class="mt-1 text-sm text-muted-foreground">Your library is empty. Import your first e-books to get started.</p>
					<Button class="mt-4" href="/import">Import books</Button>
				{/if}
			</div>
		</div>
	{/if}
</div>

<!-- Identify All confirmation dialog -->
<AlertDialog.Root bind:open={identifyAllDialogOpen}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>Identify Books</AlertDialog.Title>
			<AlertDialog.Description>
				Identify all books that need metadata using configured providers? This will
				search for matching metadata from external sources like Open Library and Hardcover.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action onclick={handleIdentifyAll}>
				Identify All
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<!-- Batch edit panel -->
{#if selectionMode && selectedArray.length > 0}
	<BatchEditPanel
		bookIds={selectedArray}
		bind:open={batchEditOpen}
		onclose={() => (batchEditOpen = false)}
		onapply={handleBatchApply}
	/>
{/if}
