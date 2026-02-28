<script lang="ts">
	import { page } from '$app/state';
	import { afterNavigate } from '$app/navigation';
	import { api, ApiError } from '$lib/api/index.js';
	import type { SeriesResponse, PaginatedBooks, BookSummary } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import BookCard from '$lib/components/library/BookCard.svelte';
	import Pagination from '$lib/components/library/Pagination.svelte';

	const PER_PAGE = 24;

	let backHref = $state('/series');
	let backLabel = $state('Back to Series');

	afterNavigate(({ from }) => {
		if (!from?.url) return;
		const p = from.url.pathname;
		if (p === '/' || p.startsWith('/books')) {
			backHref = from.url.pathname + from.url.search;
			backLabel = 'Back to Library';
		} else if (p === '/series') {
			backHref = from.url.pathname + from.url.search;
			backLabel = 'Back to Series';
		}
	});

	let series = $state<SeriesResponse | null>(null);
	let books = $state<PaginatedBooks | null>(null);
	let loading = $state(true);
	let booksLoading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
	let currentPage = $state(1);

	const seriesId = $derived(page.params.id ?? '');

	function fetchSeries() {
		loading = true;
		error = null;
		notFound = false;

		api.series
			.get(seriesId)
			.then((result) => {
				series = result;
			})
			.catch((err) => {
				if (err instanceof ApiError && err.isNotFound) {
					notFound = true;
				} else {
					error = err instanceof Error ? err.message : 'Failed to load series';
				}
			})
			.finally(() => {
				loading = false;
			});
	}

	function fetchBooks() {
		booksLoading = true;

		api.series
			.listBooks(seriesId, {
				page: currentPage,
				per_page: PER_PAGE,
				include: 'authors,series'
			})
			.then((result) => {
				books = result;
			})
			.catch((err) => {
				if (!error && !notFound) {
					error = err instanceof Error ? err.message : 'Failed to load books';
				}
			})
			.finally(() => {
				booksLoading = false;
			});
	}

	$effect(() => {
		void seriesId;
		fetchSeries();
	});

	$effect(() => {
		void seriesId;
		void currentPage;
		fetchBooks();
	});

	function handlePageChange(p: number) {
		currentPage = p;
	}

	/** Sort books by their series position, falling back to title for unpositioned books. */
	function sortedByPosition(items: BookSummary[]): BookSummary[] {
		return [...items].sort((a, b) => {
			const aPosEntry = a.series?.find((s) => s.id === seriesId);
			const bPosEntry = b.series?.find((s) => s.id === seriesId);
			const aPos = aPosEntry?.position;
			const bPos = bPosEntry?.position;

			// Both have positions — sort numerically
			if (aPos != null && bPos != null) return aPos - bPos;
			// Only one has position — positioned first
			if (aPos != null) return -1;
			if (bPos != null) return 1;
			// Neither has position — sort by title
			return a.title.localeCompare(b.title);
		});
	}

	function positionLabel(book: BookSummary): string | null {
		const entry = book.series?.find((s) => s.id === seriesId);
		if (entry?.position == null) return null;
		const pos = entry.position;
		return Number.isInteger(pos) ? pos.toString() : pos.toFixed(1);
	}

	const sortedBooks = $derived(books ? sortedByPosition(books.items) : []);

	const skeletonIds = Array.from({ length: 12 }, (_, i) => i);
</script>

<div class="mx-auto max-w-5xl space-y-6">
	<!-- Back navigation -->
	<a
		href={backHref}
		class="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
	>
		<svg
			class="size-4"
			xmlns="http://www.w3.org/2000/svg"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			stroke-width="2"
			stroke-linecap="round"
			stroke-linejoin="round"
		>
			<path d="m15 18-6-6 6-6" />
		</svg>
		{backLabel}
	</a>

	{#if loading}
		<!-- Loading skeleton -->
		<div class="space-y-4">
			<div class="h-8 w-1/2 animate-pulse rounded bg-muted"></div>
			<div class="h-5 w-2/3 animate-pulse rounded bg-muted"></div>
		</div>
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
	{:else if notFound}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
		>
			<div class="text-center">
				<p class="text-lg font-medium text-destructive">Series not found</p>
				<p class="mt-1 text-sm text-muted-foreground">
					The series you're looking for doesn't exist or has been removed.
				</p>
				<Button variant="outline" class="mt-4" href={backHref}>{backLabel}</Button>
			</div>
		</div>
	{:else if error}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
		>
			<div class="text-center">
				<p class="text-destructive">{error}</p>
				<Button
					variant="outline"
					class="mt-4"
					onclick={() => {
						fetchSeries();
						fetchBooks();
					}}>Retry</Button
				>
			</div>
		</div>
	{:else if series}
		<!-- Series header -->
		<div>
			<h1 class="text-3xl font-bold tracking-tight">{series.name}</h1>
			{#if series.description}
				<p class="mt-2 text-muted-foreground">{series.description}</p>
			{/if}
		</div>

		<!-- Books section -->
		{#if booksLoading}
			<div
				class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
			>
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
		{:else if sortedBooks.length > 0}
			<p class="text-sm text-muted-foreground">
				{books?.total ?? 0} {(books?.total ?? 0) === 1 ? 'book' : 'books'} in series
			</p>
			<div
				class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
			>
				{#each sortedBooks as book (book.id)}
					<div class="relative">
						{#if positionLabel(book)}
							<span
								class="absolute left-1.5 top-1.5 z-10 inline-flex size-7 items-center justify-center rounded-full bg-black/70 text-xs font-semibold text-white shadow"
							>
								{positionLabel(book)}
							</span>
						{/if}
						<BookCard {book} />
					</div>
				{/each}
			</div>

			{#if books}
				<Pagination
					page={books.page}
					totalPages={books.total_pages}
					onPageChange={handlePageChange}
				/>
			{/if}
		{:else}
			<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-12">
				<div class="text-center">
					<p class="text-muted-foreground">No books found in this series.</p>
				</div>
			</div>
		{/if}
	{/if}
</div>
