<script lang="ts">
	import { api, ApiError } from '$lib/api/index.js';
	import type { DuplicateLinkResponse, PaginatedDuplicates, BookDetail } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import Pagination from '$lib/components/library/Pagination.svelte';
	import MergeDialog from '$lib/components/library/MergeDialog.svelte';
	import CoverImage from '$lib/components/library/CoverImage.svelte';
	import { placeholderHue } from '$lib/utils.js';
	import { goto } from '$app/navigation';

	const PER_PAGE = 20;

	let currentPage = $state(1);
	let loading = $state(true);
	let data = $state<PaginatedDuplicates | null>(null);
	let error = $state<string | null>(null);

	let dismissingId = $state<string | null>(null);
	let dismissError = $state<string | null>(null);

	// Merge dialog state
	let mergeDialogOpen = $state(false);
	let selectedLink = $state<DuplicateLinkResponse | null>(null);

	function fetchDuplicates() {
		loading = true;
		error = null;

		api.duplicates
			.list({ page: currentPage, per_page: PER_PAGE })
			.then((result) => {
				data = result;
			})
			.catch((err) => {
				error = err instanceof Error ? err.message : 'Failed to load duplicates';
			})
			.finally(() => {
				loading = false;
			});
	}

	$effect(() => {
		void currentPage;
		fetchDuplicates();
	});

	function handlePageChange(page: number) {
		currentPage = page;
	}

	function openMergeDialog(link: DuplicateLinkResponse) {
		selectedLink = link;
		mergeDialogOpen = true;
	}

	async function handleDismiss(linkId: string) {
		dismissingId = linkId;
		dismissError = null;
		try {
			await api.duplicates.dismiss(linkId);
			// Remove from list
			if (data) {
				data = {
					...data,
					items: data.items.filter((item) => item.id !== linkId),
					total: data.total - 1
				};
			}
		} catch (err) {
			dismissError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to dismiss duplicate';
		} finally {
			dismissingId = null;
		}
	}

	function handleMergeComplete(merged: BookDetail) {
		mergeDialogOpen = false;
		selectedLink = null;
		goto(`/books/${merged.id}`);
	}

	function handleMergeCancel() {
		mergeDialogOpen = false;
		selectedLink = null;
	}

	function detectionMethodLabel(method: string): string {
		switch (method) {
			case 'fuzzy':
				return 'Fuzzy Match';
			case 'hash':
				return 'File Hash';
			case 'isbn':
				return 'ISBN Match';
			case 'user':
				return 'User Flagged';
			default:
				return method;
		}
	}

	function detectionMethodClass(method: string): string {
		switch (method) {
			case 'fuzzy':
				return 'bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400';
			case 'hash':
				return 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400';
			case 'isbn':
				return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400';
			case 'user':
				return 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400';
			default:
				return 'bg-muted text-muted-foreground';
		}
	}

	const skeletonIds = Array.from({ length: 5 }, (_, i) => i);
</script>

<div class="mx-auto max-w-5xl space-y-6">
	<div>
		<h1 class="text-3xl font-bold tracking-tight">Duplicates</h1>
		<p class="text-muted-foreground">
			Review and resolve potential duplicate books in your library.
		</p>
	</div>

	{#if dismissError}
		<div
			class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
		>
			{dismissError}
		</div>
	{/if}

	{#if loading}
		<!-- Loading skeleton -->
		<div class="space-y-4">
			{#each skeletonIds as id (id)}
				<div class="rounded-lg border border-border p-4">
					<div class="flex items-center gap-6">
						<div class="flex items-center gap-4">
							<div class="h-20 w-14 animate-pulse rounded bg-muted"></div>
							<div class="space-y-2">
								<div class="h-4 w-40 animate-pulse rounded bg-muted"></div>
								<div class="h-3 w-24 animate-pulse rounded bg-muted"></div>
							</div>
						</div>
						<div class="flex items-center">
							<div class="h-4 w-8 animate-pulse rounded bg-muted"></div>
						</div>
						<div class="flex items-center gap-4">
							<div class="h-20 w-14 animate-pulse rounded bg-muted"></div>
							<div class="space-y-2">
								<div class="h-4 w-40 animate-pulse rounded bg-muted"></div>
								<div class="h-3 w-24 animate-pulse rounded bg-muted"></div>
							</div>
						</div>
					</div>
				</div>
			{/each}
		</div>
	{:else if error}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
		>
			<div class="text-center">
				<p class="text-destructive">{error}</p>
				<Button variant="outline" class="mt-4" onclick={fetchDuplicates}>Retry</Button>
			</div>
		</div>
	{:else if data && data.items.length > 0}
		<div class="space-y-4">
			{#each data.items as link (link.id)}
				<div class="rounded-lg border border-border bg-card shadow-sm">
					<div class="p-4">
						<!-- Top row: detection badge + confidence + actions -->
						<div class="mb-3 flex items-center justify-between">
							<div class="flex items-center gap-3">
								<span
									class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {detectionMethodClass(link.detection_method)}"
								>
									{detectionMethodLabel(link.detection_method)}
								</span>
								<div class="flex items-center gap-1.5 text-xs text-muted-foreground">
									<span>{Math.round(link.confidence * 100)}% confidence</span>
									<div class="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
										<div
											class="h-full rounded-full bg-primary transition-all"
											style="width: {link.confidence * 100}%"
										></div>
									</div>
								</div>
							</div>
							<div class="flex items-center gap-2">
								<Button
									size="sm"
									variant="outline"
									class="h-7 px-2 text-xs"
									disabled={dismissingId === link.id}
									onclick={() => handleDismiss(link.id)}
								>
									{#if dismissingId === link.id}
										Dismissing...
									{:else}
										Dismiss
									{/if}
								</Button>
								<Button
									size="sm"
									class="h-7 px-2 text-xs"
									onclick={() => openMergeDialog(link)}
								>
									Review
								</Button>
							</div>
						</div>

						<!-- Side-by-side book thumbnails -->
						<div class="flex items-center gap-4">
							<!-- Book A -->
							<a
								href="/books/{link.book_a.id}"
								class="flex min-w-0 flex-1 items-center gap-3 rounded-md p-2 transition-colors hover:bg-muted"
							>
								<div
									class="relative h-20 w-14 flex-shrink-0 overflow-hidden rounded bg-muted shadow-sm"
								>
									{#if link.book_a.has_cover}
										<CoverImage src="/api/books/{link.book_a.id}/cover?size=sm" alt="Cover of {link.book_a.title}" />
									{:else}
										<div
											class="flex h-full w-full items-center justify-center p-1"
											style="background-color: hsl({placeholderHue(link.book_a.id)}, 30%, 25%);"
										>
											<span
												class="line-clamp-3 text-center text-[8px] font-medium text-white/80"
											>
												{link.book_a.title}
											</span>
										</div>
									{/if}
								</div>
								<div class="min-w-0">
									<p class="line-clamp-2 text-sm font-medium">{link.book_a.title}</p>
									{#if link.book_a.authors && link.book_a.authors.length > 0}
										<p class="line-clamp-1 text-xs text-muted-foreground">
											{link.book_a.authors.map((a) => a.name).join(', ')}
										</p>
									{/if}
								</div>
							</a>

							<!-- Separator -->
							<div class="flex flex-col items-center gap-1 text-muted-foreground">
								<svg
									class="size-5"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									stroke-width="2"
									stroke-linecap="round"
									stroke-linejoin="round"
								>
									<path d="M8 7h12" />
									<path d="m16 3 4 4-4 4" />
									<path d="M16 17H4" />
									<path d="m8 21-4-4 4-4" />
								</svg>
							</div>

							<!-- Book B -->
							<a
								href="/books/{link.book_b.id}"
								class="flex min-w-0 flex-1 items-center gap-3 rounded-md p-2 transition-colors hover:bg-muted"
							>
								<div
									class="relative h-20 w-14 flex-shrink-0 overflow-hidden rounded bg-muted shadow-sm"
								>
									{#if link.book_b.has_cover}
										<CoverImage src="/api/books/{link.book_b.id}/cover?size=sm" alt="Cover of {link.book_b.title}" />
									{:else}
										<div
											class="flex h-full w-full items-center justify-center p-1"
											style="background-color: hsl({placeholderHue(link.book_b.id)}, 30%, 25%);"
										>
											<span
												class="line-clamp-3 text-center text-[8px] font-medium text-white/80"
											>
												{link.book_b.title}
											</span>
										</div>
									{/if}
								</div>
								<div class="min-w-0">
									<p class="line-clamp-2 text-sm font-medium">{link.book_b.title}</p>
									{#if link.book_b.authors && link.book_b.authors.length > 0}
										<p class="line-clamp-1 text-xs text-muted-foreground">
											{link.book_b.authors.map((a) => a.name).join(', ')}
										</p>
									{/if}
								</div>
							</a>
						</div>
					</div>
				</div>
			{/each}
		</div>

		<Pagination page={data.page} totalPages={data.total_pages} onPageChange={handlePageChange} />
	{:else}
		<!-- Empty state -->
		<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-12">
			<div class="text-center">
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
					<path d="M20 6 9 17l-5-5" />
				</svg>
				<p class="font-medium text-foreground">No duplicates found</p>
				<p class="mt-1 text-sm text-muted-foreground">
					Your library has no pending duplicate pairs to review. Duplicates are
					detected automatically during import or can be flagged manually from book
					detail pages.
				</p>
			</div>
		</div>
	{/if}
</div>

{#if selectedLink}
	<MergeDialog
		link={selectedLink}
		bind:open={mergeDialogOpen}
		onmerge={handleMergeComplete}
		oncancel={handleMergeCancel}
	/>
{/if}
