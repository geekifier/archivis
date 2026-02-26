<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import { api } from '$lib/api/index.js';
	import type { BookDetail, ReadingProgressResponse, TocItem } from '$lib/api/types.js';
	import ReaderView from '$lib/components/reader/ReaderView.svelte';
	import { reader } from '$lib/stores/reader.svelte.js';

	const bookId = $derived(page.params.bookId ?? '');
	const fileId = $derived(page.params.fileId ?? '');

	let book = $state<BookDetail | null>(null);
	let bookBlob = $state<Blob | null>(null);
	let savedLocation = $state<string | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let readerView = $state<ReturnType<typeof ReaderView> | null>(null);

	// Theme-reactive background color
	const themes: Record<string, string> = {
		light: '#ffffff',
		dark: '#1a1a1a',
		sepia: '#f4ecd8'
	};
	const containerBg = $derived(themes[reader.preferences.theme] ?? '#ffffff');

	$effect(() => {
		void loadReader();
	});

	async function loadReader(): Promise<void> {
		loading = true;
		error = null;
		try {
			// 1. Fetch book metadata
			book = await api.books.get(bookId);

			// 2. Find the file entry to get the format
			const fileEntry = book.files.find((f) => f.id === fileId);
			const fmt = fileEntry?.format ?? 'epub';

			// 3. Initialize the reader store
			reader.init(bookId, fileId, book.title, fmt);
			reader.loadPreferences();

			// 4. Try to load saved location from localStorage first (instant)
			savedLocation = reader.loadSavedLocation();

			// 5. Then try server progress (may have newer data)
			let serverProgress: ReadingProgressResponse | null = null;
			try {
				serverProgress = await api.reader.getProgress(bookId);
			} catch {
				// No saved progress, that's fine
			}

			if (serverProgress?.location) {
				savedLocation = serverProgress.location;
			}

			// Merge server preferences if available
			if (serverProgress?.preferences) {
				const serverPrefs = serverProgress.preferences;
				for (const [key, value] of Object.entries(serverPrefs)) {
					if (value !== undefined && value !== null) {
						reader.updatePreference(
							key as keyof typeof reader.preferences,
							value as never
						);
					}
				}
			}

			// 6. Fetch file as Blob
			bookBlob = await api.reader.fetchFileBlob(bookId, fileId);
		} catch (err: unknown) {
			error = err instanceof Error ? err.message : 'Failed to load reader';
		} finally {
			loading = false;
		}
	}

	function handleRelocate(detail: Parameters<NonNullable<typeof reader.updateLocation>>[0]): void {
		reader.updateLocation(detail);
	}

	function handleTocLoaded(toc: TocItem[]): void {
		reader.setToc(toc);
	}

	function handleLoad(): void {
		// Book is ready
	}

	function handleKeydown(e: KeyboardEvent): void {
		// Don't capture when typing in an input
		if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;

		switch (e.key) {
			case 'ArrowRight':
			case 'PageDown':
			case ' ':
				e.preventDefault();
				readerView?.next();
				break;
			case 'ArrowLeft':
			case 'PageUp':
				e.preventDefault();
				readerView?.prev();
				break;
			case 'Escape':
				reader.toggleToolbar();
				break;
			case 'f':
				reader.toggleFullscreen();
				break;
		}
	}

	function handleBeforeUnload(): void {
		reader.saveProgressNow();
	}

	function handleFullscreenChange(): void {
		reader.setFullscreen(!!document.fullscreenElement);
	}

	onMount(() => {
		document.addEventListener('fullscreenchange', handleFullscreenChange);

		return () => {
			document.removeEventListener('fullscreenchange', handleFullscreenChange);
			reader.destroy();
		};
	});
</script>

<svelte:window onkeydown={handleKeydown} onbeforeunload={handleBeforeUnload} />

<svelte:head>
	<title>{book?.title ?? 'Reader'} — Archivis</title>
</svelte:head>

<div class="flex h-screen flex-col" style:background-color={containerBg}>
	<!-- Minimal header for now (toolbar will be added in Task 5) -->
	{#if reader.toolbarVisible}
		<div class="flex items-center gap-3 border-b border-border bg-background/90 px-4 py-2 backdrop-blur-sm">
			<button
				onclick={() => goto(`/books/${bookId}`)}
				class="text-sm text-muted-foreground hover:text-foreground"
			>
				&larr; Back
			</button>
			{#if book}
				<span class="truncate text-sm font-medium">{book.title}</span>
			{/if}
			{#if reader.currentChapter}
				<span class="hidden truncate text-xs text-muted-foreground md:inline">
					{reader.currentChapter}
				</span>
			{/if}
			<div class="ml-auto text-xs text-muted-foreground">
				{reader.progressPercent}%
			</div>
		</div>
	{/if}

	<!-- Reader viewport -->
	{#if loading}
		<div class="flex flex-1 items-center justify-center">
			<div class="text-muted-foreground">Loading book...</div>
		</div>
	{:else if error}
		<div class="flex flex-1 items-center justify-center">
			<div class="text-destructive">{error}</div>
		</div>
	{:else if bookBlob}
		<div class="flex-1 overflow-hidden">
			<ReaderView
				bind:this={readerView}
				{bookBlob}
				{savedLocation}
				onRelocate={handleRelocate}
				onTocLoaded={handleTocLoaded}
				onLoad={handleLoad}
			/>
		</div>
	{/if}
</div>
