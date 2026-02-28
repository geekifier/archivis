<script lang="ts">
	import { SvelteURLSearchParams } from 'svelte/reactivity';
	import { api, type PaginatedAuthors } from '$lib/api/index.js';
	import type { SortOrder } from '$lib/api/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import Pagination from '$lib/components/library/Pagination.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';

	const PER_PAGE = 30;
	const DEBOUNCE_MS = 300;

	type SortOption = { label: string; field: 'sort_name' | 'name' | 'book_count'; order: SortOrder };

	const sortOptions: SortOption[] = [
		{ label: 'Name A\u2013Z', field: 'sort_name', order: 'asc' },
		{ label: 'Name Z\u2013A', field: 'sort_name', order: 'desc' },
		{ label: 'Most Books', field: 'book_count', order: 'desc' },
		{ label: 'Fewest Books', field: 'book_count', order: 'asc' }
	];

	// Restore state from URL search params
	const _params = page.url.searchParams;
	const _initPage = Math.max(1, parseInt(_params.get('page') || '1', 10) || 1);
	const _initQuery = _params.get('q') || '';
	const _initSort = _params.get('sort') as 'sort_name' | 'name' | 'book_count' | null;
	const _initOrder = _params.get('order') as SortOrder | null;
	const _initSortIdx =
		_initSort && _initOrder
			? sortOptions.findIndex((o) => o.field === _initSort && o.order === _initOrder)
			: -1;

	let searchInput = $state(_initQuery);
	let activeQuery = $state(_initQuery);
	let sortIndex = $state(_initSortIdx >= 0 ? _initSortIdx : 0);
	let currentPage = $state(_initPage);
	let loading = $state(true);
	let data = $state<PaginatedAuthors | null>(null);
	let error = $state<string | null>(null);
	let debounceTimer: ReturnType<typeof setTimeout> | undefined;

	let activeSortBy = $state<'sort_name' | 'name' | 'book_count'>(_initSort || sortOptions[0].field);
	let activeSortOrder = $state<SortOrder>(_initOrder || sortOptions[0].order);

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

	function handlePageChange(p: number) {
		currentPage = p;
	}

	// Reset to page 1 when search or sort changes
	let _prevQuery = _initQuery;
	let _prevSortBy = _initSort || sortOptions[0].field;
	let _prevSortOrder = _initOrder || sortOptions[0].order;

	$effect(() => {
		const q = activeQuery;
		const sb = activeSortBy;
		const so = activeSortOrder;

		const changed = q !== _prevQuery || sb !== _prevSortBy || so !== _prevSortOrder;

		_prevQuery = q;
		_prevSortBy = sb;
		_prevSortOrder = so;

		if (changed) {
			currentPage = 1;
		}
	});

	// Fetch authors when params change
	$effect(() => {
		const p = currentPage;
		const field = activeSortBy;
		const order = activeSortOrder;
		const q = activeQuery;

		loading = true;
		error = null;

		api.authors
			.list({
				page: p,
				per_page: PER_PAGE,
				sort_by: field,
				sort_order: order,
				q: q || undefined
			})
			.then((result) => {
				data = result;
			})
			.catch((err) => {
				error = err instanceof Error ? err.message : 'Failed to load authors';
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

		const search = params.toString();
		const url = search ? `/authors?${search}` : '/authors';
		goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	});

	const skeletonRows = Array.from({ length: 10 }, (_, i) => i);
</script>

<div class="space-y-6">
	<div>
		<h1 class="text-3xl font-bold tracking-tight">Authors</h1>
		<p class="text-muted-foreground">Browse all authors in your library</p>
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
				placeholder="Search authors..."
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
		<div class="overflow-hidden rounded-lg border border-border">
			{#each skeletonRows as id (id)}
				<div
					class="flex items-center gap-4 border-b border-border px-4 py-3 last:border-b-0"
				>
					<div class="h-4 w-48 animate-pulse rounded bg-muted"></div>
					<div class="h-4 w-32 animate-pulse rounded bg-muted"></div>
					<div class="ml-auto h-4 w-8 animate-pulse rounded bg-muted"></div>
				</div>
			{/each}
		</div>
	{:else if error}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
		>
			<div class="text-center">
				<p class="text-destructive">{error}</p>
				<Button variant="outline" class="mt-4" onclick={() => (currentPage = currentPage)}>
					Retry
				</Button>
			</div>
		</div>
	{:else if data && data.items.length > 0}
		<p class="text-sm text-muted-foreground">
			{data.total} {data.total === 1 ? 'author' : 'authors'}
		</p>

		<div class="overflow-hidden rounded-lg border border-border">
			<table class="w-full text-sm">
				<thead>
					<tr class="border-b border-border bg-muted/50">
						<th class="px-4 py-2.5 text-left font-medium text-muted-foreground">Name</th>
						<th class="hidden px-4 py-2.5 text-left font-medium text-muted-foreground sm:table-cell"
							>Sort Name</th
						>
						<th class="px-4 py-2.5 text-right font-medium text-muted-foreground">Books</th>
					</tr>
				</thead>
				<tbody>
					{#each data.items as author (author.id)}
						<tr
							class="border-b border-border transition-colors last:border-b-0 hover:bg-muted/30"
						>
							<td class="px-4 py-2.5">
								<a
									href="/authors/{author.id}"
									class="font-medium text-foreground transition-colors hover:text-primary"
								>
									{author.name}
								</a>
							</td>
							<td class="hidden px-4 py-2.5 text-muted-foreground sm:table-cell">
								{#if author.sort_name && author.sort_name !== author.name}
									{author.sort_name}
								{:else}
									<span class="text-muted-foreground/40">&mdash;</span>
								{/if}
							</td>
							<td class="px-4 py-2.5 text-right text-muted-foreground">{author.book_count}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		<Pagination page={data.page} totalPages={data.total_pages} onPageChange={handlePageChange} />
	{:else}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-border p-12"
		>
			<div class="text-center">
				{#if activeQuery}
					<svg
						class="mx-auto mb-3 size-10 text-muted-foreground/50"
						xmlns="http://www.w3.org/2000/svg"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="1.5"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<circle cx="11" cy="11" r="8" />
						<path d="m21 21-4.3-4.3" />
					</svg>
					<p class="font-medium text-foreground">No authors found</p>
					<p class="mt-1 text-sm text-muted-foreground">
						No authors match your search.
					</p>
					<Button
						variant="outline"
						class="mt-4"
						onclick={() => {
							searchInput = '';
							activeQuery = '';
						}}
					>
						Clear search
					</Button>
				{:else}
					<svg
						class="mx-auto mb-3 size-10 text-muted-foreground/50"
						xmlns="http://www.w3.org/2000/svg"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="1.5"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
						<circle cx="9" cy="7" r="4" />
						<path d="M22 21v-2a4 4 0 0 0-3-3.87" />
						<path d="M16 3.13a4 4 0 0 1 0 7.75" />
					</svg>
					<p class="font-medium text-foreground">No authors yet</p>
					<p class="mt-1 text-sm text-muted-foreground">
						Authors will appear here once books are imported.
					</p>
				{/if}
			</div>
		</div>
	{/if}
</div>
