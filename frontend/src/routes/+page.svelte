<script lang="ts">
	import { api, type PaginatedBooks } from '$lib/api/index.js';
	import type { SortField, SortOrder } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import BookCard from '$lib/components/library/BookCard.svelte';
	import Pagination from '$lib/components/library/Pagination.svelte';

	const PER_PAGE = 24;
	const DEBOUNCE_MS = 300;

	type SortOption = { label: string; field: SortField; order: SortOrder };

	const sortOptions: SortOption[] = [
		{ label: 'Recently Added', field: 'added_at', order: 'desc' },
		{ label: 'Title A\u2013Z', field: 'title', order: 'asc' },
		{ label: 'Title Z\u2013A', field: 'title', order: 'desc' },
		{ label: 'Highest Rated', field: 'rating', order: 'desc' }
	];

	let searchInput = $state('');
	let activeQuery = $state('');
	let sortIndex = $state(0);
	let currentPage = $state(1);
	let loading = $state(true);
	let data = $state<PaginatedBooks | null>(null);
	let error = $state<string | null>(null);
	let debounceTimer: ReturnType<typeof setTimeout> | undefined;

	const sortBy = $derived(sortOptions[sortIndex].field);
	const sortOrder = $derived(sortOptions[sortIndex].order);

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

	// Reset to page 1 when search or sort changes
	$effect(() => {
		// Touch reactive deps to track them
		void activeQuery;
		void sortBy;
		void sortOrder;
		currentPage = 1;
	});

	// Fetch books when query params change
	$effect(() => {
		const page = currentPage;
		const field = sortBy;
		const order = sortOrder;
		const q = activeQuery;

		loading = true;
		error = null;

		api.books
			.list({
				page,
				per_page: PER_PAGE,
				sort_by: field,
				sort_order: order,
				q: q || undefined,
				include: 'authors'
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

	<!-- Content area -->
	{#if loading}
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
		<div class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
			{#each data.items as book (book.id)}
				<BookCard {book} />
			{/each}
		</div>

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
