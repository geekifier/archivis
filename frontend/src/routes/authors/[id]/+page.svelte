<script lang="ts">
	import { page } from '$app/state';
	import { api, ApiError } from '$lib/api/index.js';
	import type { AuthorResponse, PaginatedBooks } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import BookCard from '$lib/components/library/BookCard.svelte';
	import Pagination from '$lib/components/library/Pagination.svelte';

	const PER_PAGE = 24;

	let author = $state<AuthorResponse | null>(null);
	let books = $state<PaginatedBooks | null>(null);
	let loading = $state(true);
	let booksLoading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
	let currentPage = $state(1);

	const authorId = $derived(page.params.id ?? '');

	function fetchAuthor() {
		loading = true;
		error = null;
		notFound = false;

		api.authors
			.get(authorId)
			.then((result) => {
				author = result;
			})
			.catch((err) => {
				if (err instanceof ApiError && err.isNotFound) {
					notFound = true;
				} else {
					error = err instanceof Error ? err.message : 'Failed to load author';
				}
			})
			.finally(() => {
				loading = false;
			});
	}

	function fetchBooks() {
		booksLoading = true;

		api.authors
			.listBooks(authorId, {
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
		void authorId;
		fetchAuthor();
	});

	$effect(() => {
		void authorId;
		void currentPage;
		fetchBooks();
	});

	function handlePageChange(p: number) {
		currentPage = p;
	}

	const skeletonIds = Array.from({ length: 12 }, (_, i) => i);
</script>

<div class="mx-auto max-w-5xl space-y-6">
	<!-- Back navigation -->
	<a
		href="/"
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
		Back to Library
	</a>

	{#if loading}
		<!-- Loading skeleton -->
		<div class="space-y-4">
			<div class="h-8 w-1/2 animate-pulse rounded bg-muted"></div>
			<div class="h-5 w-1/3 animate-pulse rounded bg-muted"></div>
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
				<p class="text-lg font-medium text-destructive">Author not found</p>
				<p class="mt-1 text-sm text-muted-foreground">
					The author you're looking for doesn't exist or has been removed.
				</p>
				<Button variant="outline" class="mt-4" href="/">Back to Library</Button>
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
						fetchAuthor();
						fetchBooks();
					}}>Retry</Button
				>
			</div>
		</div>
	{:else if author}
		<!-- Author header -->
		<div>
			<h1 class="text-3xl font-bold tracking-tight">{author.name}</h1>
			{#if author.sort_name && author.sort_name !== author.name}
				<p class="mt-1 text-muted-foreground">Sort name: {author.sort_name}</p>
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
		{:else if books && books.items.length > 0}
			<p class="text-sm text-muted-foreground">
				{books.total} {books.total === 1 ? 'book' : 'books'}
			</p>
			<div
				class="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
			>
				{#each books.items as book (book.id)}
					<BookCard {book} />
				{/each}
			</div>

			<Pagination page={books.page} totalPages={books.total_pages} onPageChange={handlePageChange} />
		{:else}
			<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-12">
				<div class="text-center">
					<p class="text-muted-foreground">No books found for this author.</p>
				</div>
			</div>
		{/if}
	{/if}
</div>
