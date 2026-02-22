<script lang="ts">
	import { api, type PaginatedBooks } from '$lib/api/index.js';
	import type { SortField, SortOrder } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import BookCard from '$lib/components/library/BookCard.svelte';
	import BookListView from '$lib/components/library/BookListView.svelte';
	import Pagination from '$lib/components/library/Pagination.svelte';

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

	let viewMode = $state<ViewMode>(loadViewPreference());
	let searchInput = $state('');
	let activeQuery = $state('');
	let sortIndex = $state(0);
	let currentPage = $state(1);
	let loading = $state(true);
	let data = $state<PaginatedBooks | null>(null);
	let error = $state<string | null>(null);
	let debounceTimer: ReturnType<typeof setTimeout> | undefined;

	let activeSortBy = $state<SortField>(sortOptions[0].field);
	let activeSortOrder = $state<SortOrder>(sortOptions[0].order);

	const sortBy = $derived(sortOptions[sortIndex].field);
	const sortOrder = $derived(sortOptions[sortIndex].order);

	// Keep activeSortBy/Order in sync with dropdown changes
	$effect(() => {
		activeSortBy = sortBy;
		activeSortOrder = sortOrder;
	});

	const includeParam = $derived(viewMode === 'list' ? 'authors,series,files' : 'authors');

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

	// Reset to page 1 when search or sort changes
	$effect(() => {
		void activeQuery;
		void activeSortBy;
		void activeSortOrder;
		currentPage = 1;
	});

	// Fetch books when query params change
	$effect(() => {
		const page = currentPage;
		const field = activeSortBy;
		const order = activeSortOrder;
		const q = activeQuery;
		const include = includeParam;

		loading = true;
		error = null;

		api.books
			.list({
				page,
				per_page: PER_PAGE,
				sort_by: field,
				sort_order: order,
				q: q || undefined,
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

	<!-- Content area -->
	{#if loading}
		{#if viewMode === 'grid'}
			<!-- Skeleton grid -->
			<div class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
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
			<div class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
				{#each data.items as book (book.id)}
					<BookCard {book} />
				{/each}
			</div>
		{:else}
			<BookListView
				books={data.items}
				sortBy={activeSortBy}
				sortOrder={activeSortOrder}
				onSort={handleListSort}
			/>
		{/if}

		<Pagination page={data.page} totalPages={data.total_pages} onPageChange={handlePageChange} />
	{:else}
		<!-- Empty state -->
		<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-12">
			<div class="text-center">
				{#if activeQuery}
					<p class="text-muted-foreground">No books match your search.</p>
					<Button variant="outline" class="mt-4" onclick={() => { searchInput = ''; activeQuery = ''; }}>
						Clear search
					</Button>
				{:else}
					<p class="text-muted-foreground">No books in your library yet.</p>
					<Button variant="outline" class="mt-4" href="/import">Import books</Button>
				{/if}
			</div>
		</div>
	{/if}
</div>
