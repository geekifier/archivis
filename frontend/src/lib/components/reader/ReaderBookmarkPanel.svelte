<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { BookmarkResponse } from '$lib/api/types.js';

	interface Props {
		bookId: string;
		fileId: string;
		currentLocation: string | null;
		currentProgress: number;
		open: boolean;
		onClose: () => void;
		onNavigate: (location: string) => void;
	}

	let { bookId, fileId, currentLocation, currentProgress, open, onClose, onNavigate }: Props =
		$props();

	let bookmarks = $state<BookmarkResponse[]>([]);
	let loadingBookmarks = $state(false);
	let addingBookmark = $state(false);
	let deletingId = $state<string | null>(null);
	let errorMessage = $state<string | null>(null);

	// Fetch bookmarks when panel opens
	$effect(() => {
		if (open) {
			void fetchBookmarks();
		}
	});

	async function fetchBookmarks(): Promise<void> {
		loadingBookmarks = true;
		errorMessage = null;
		try {
			bookmarks = await api.reader.listBookmarks(bookId, fileId);
		} catch (err: unknown) {
			errorMessage = err instanceof Error ? err.message : 'Failed to load bookmarks';
		} finally {
			loadingBookmarks = false;
		}
	}

	async function addBookmark(): Promise<void> {
		if (!currentLocation) return;
		addingBookmark = true;
		errorMessage = null;
		try {
			const created = await api.reader.createBookmark(bookId, fileId, {
				location: currentLocation,
				position: currentProgress
			});
			bookmarks = [created, ...bookmarks];
		} catch (err: unknown) {
			errorMessage = err instanceof Error ? err.message : 'Failed to add bookmark';
		} finally {
			addingBookmark = false;
		}
	}

	async function deleteBookmark(id: string): Promise<void> {
		deletingId = id;
		errorMessage = null;
		try {
			await api.reader.deleteBookmark(id);
			bookmarks = bookmarks.filter((b) => b.id !== id);
		} catch (err: unknown) {
			errorMessage = err instanceof Error ? err.message : 'Failed to delete bookmark';
		} finally {
			deletingId = null;
		}
	}

	function handleNavigate(bookmark: BookmarkResponse): void {
		onNavigate(bookmark.location);
		onClose();
	}

	function formatBookmarkLabel(bookmark: BookmarkResponse): string {
		if (bookmark.label) return bookmark.label;
		const pct = Math.round(bookmark.position * 100);
		return `Position ${pct}%`;
	}

	function handleOverlayKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			onClose();
		}
	}
</script>

{#if open}
	<!-- Background overlay -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed inset-0 z-40 bg-black/40"
		onclick={onClose}
		onkeydown={handleOverlayKeydown}
	></div>

	<!-- Bookmarks panel (slides from right, same pattern as settings) -->
	<div
		class="fixed bottom-0 right-0 top-0 z-50 flex w-72 flex-col border-l border-border bg-background shadow-lg sm:w-80"
	>
		<!-- Header -->
		<div class="flex items-center justify-between border-b border-border px-4 py-3">
			<h2 class="text-sm font-semibold">Bookmarks</h2>
			<div class="flex items-center gap-1">
				<!-- Add bookmark button -->
				<button
					onclick={addBookmark}
					disabled={addingBookmark || !currentLocation}
					class="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50"
					aria-label="Add bookmark at current position"
					title="Add bookmark"
				>
					<svg
						class="size-4"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<path d="M12 5v14" />
						<path d="M5 12h14" />
					</svg>
				</button>
				<!-- Close button -->
				<button
					onclick={onClose}
					class="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
					aria-label="Close bookmarks"
				>
					<svg
						class="size-4"
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
				</button>
			</div>
		</div>

		<!-- Error message -->
		{#if errorMessage}
			<div class="border-b border-destructive/20 bg-destructive/10 px-4 py-2 text-xs text-destructive">
				{errorMessage}
			</div>
		{/if}

		<!-- Bookmark list -->
		<div class="flex-1 overflow-y-auto">
			{#if loadingBookmarks}
				<div class="px-4 py-8 text-center text-sm text-muted-foreground">
					Loading bookmarks...
				</div>
			{:else if bookmarks.length === 0}
				<div class="px-4 py-8 text-center text-sm text-muted-foreground">
					No bookmarks yet
				</div>
			{:else}
				{#each bookmarks as bookmark (bookmark.id)}
					<div
						class="group flex items-start gap-2 border-b border-border px-4 py-3 transition-colors hover:bg-accent/50"
					>
						<!-- Bookmark content (clickable for navigation) -->
						<button
							class="min-w-0 flex-1 text-left"
							onclick={() => handleNavigate(bookmark)}
						>
							<div class="text-sm font-medium text-foreground">
								{formatBookmarkLabel(bookmark)}
							</div>
							{#if bookmark.excerpt}
								<div class="mt-0.5 line-clamp-2 text-sm text-muted-foreground">
									{bookmark.excerpt}
								</div>
							{/if}
						</button>

						<!-- Delete button -->
						<button
							onclick={() => deleteBookmark(bookmark.id)}
							disabled={deletingId === bookmark.id}
							class="mt-0.5 inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-all hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100 disabled:pointer-events-none disabled:opacity-50"
							aria-label="Delete bookmark"
							title="Delete bookmark"
						>
							<svg
								class="size-3.5"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<path d="M3 6h18" />
								<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
								<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
							</svg>
						</button>
					</div>
				{/each}
			{/if}
		</div>
	</div>
{/if}
