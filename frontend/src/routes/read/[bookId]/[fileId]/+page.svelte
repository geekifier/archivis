<script lang="ts">
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { api } from '$lib/api/index.js';
	import type { BookDetail, ReadingProgressResponse, TocItem } from '$lib/api/types.js';
	import ReaderView from '$lib/components/reader/ReaderView.svelte';
	import ReaderToolbar from '$lib/components/reader/ReaderToolbar.svelte';
	import ReaderTocPanel from '$lib/components/reader/ReaderTocPanel.svelte';
	import { reader } from '$lib/stores/reader.svelte.js';

	const bookId = $derived(page.params.bookId ?? '');
	const fileId = $derived(page.params.fileId ?? '');

	let book = $state<BookDetail | null>(null);
	let bookBlob = $state<Blob | null>(null);
	let savedLocation = $state<string | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let readerView = $state<ReturnType<typeof ReaderView> | null>(null);

	// Auto-hide timer for toolbar
	const TOOLBAR_HIDE_DELAY = 3000;
	let autoHideTimer: ReturnType<typeof setTimeout> | null = null;

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

	// Auto-hide toolbar after 3 seconds when visible
	$effect(() => {
		if (reader.toolbarVisible) {
			resetAutoHideTimer();
		} else {
			clearAutoHideTimer();
		}

		return () => {
			clearAutoHideTimer();
		};
	});

	function resetAutoHideTimer(): void {
		clearAutoHideTimer();
		// Don't auto-hide if a panel is open
		if (reader.tocPanelOpen || reader.settingsPanelOpen || reader.bookmarksPanelOpen) return;
		autoHideTimer = setTimeout(() => {
			reader.hideToolbar();
		}, TOOLBAR_HIDE_DELAY);
	}

	function clearAutoHideTimer(): void {
		if (autoHideTimer) {
			clearTimeout(autoHideTimer);
			autoHideTimer = null;
		}
	}

	function handleInteraction(): void {
		if (reader.toolbarVisible) {
			resetAutoHideTimer();
		}
	}

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

	function handleTocNavigate(href: string): void {
		readerView?.goTo(href);
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
				if (reader.tocPanelOpen) {
					reader.toggleTocPanel();
				} else {
					reader.toggleToolbar();
				}
				break;
			case 'f':
				reader.toggleFullscreen();
				break;
			case 't':
				reader.toggleTocPanel();
				break;
		}
	}

	function handleBeforeUnload(): void {
		reader.saveProgressNow();
	}

	function handleFullscreenChange(): void {
		reader.setFullscreen(!!document.fullscreenElement);
	}

	// Touch tap zone handling
	function handleViewportClick(e: MouseEvent): void {
		// Only handle direct clicks on the tap zone overlay, not bubbled events
		const target = e.currentTarget as HTMLElement;
		if (!target) return;

		const rect = target.getBoundingClientRect();
		const x = e.clientX - rect.left;
		const width = rect.width;
		const fraction = x / width;

		if (fraction < 0.25) {
			// Left 25%: previous page
			readerView?.prev();
		} else if (fraction > 0.75) {
			// Right 25%: next page
			readerView?.next();
		} else {
			// Center 50%: toggle toolbar
			reader.toggleToolbar();
		}
	}

	onMount(() => {
		document.addEventListener('fullscreenchange', handleFullscreenChange);

		return () => {
			document.removeEventListener('fullscreenchange', handleFullscreenChange);
			clearAutoHideTimer();
			reader.destroy();
		};
	});
</script>

<svelte:window
	onkeydown={handleKeydown}
	onbeforeunload={handleBeforeUnload}
	onpointermove={handleInteraction}
/>

<svelte:head>
	<title>{book?.title ?? 'Reader'} — Archivis</title>
</svelte:head>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="relative flex h-screen flex-col" style:background-color={containerBg}>
	<!-- Toolbar -->
	<ReaderToolbar
		{bookId}
		bookTitle={book?.title ?? ''}
		visible={reader.toolbarVisible}
		onToggleToc={() => reader.toggleTocPanel()}
		onToggleBookmarks={() => reader.toggleBookmarksPanel()}
		onToggleSettings={() => reader.toggleSettingsPanel()}
		onToggleFullscreen={() => reader.toggleFullscreen()}
	/>

	<!-- TOC Panel -->
	<ReaderTocPanel
		toc={reader.toc}
		currentHref={reader.currentHref}
		open={reader.tocPanelOpen}
		onClose={() => reader.toggleTocPanel()}
		onNavigate={handleTocNavigate}
	/>

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
		<div class="relative flex-1 overflow-hidden">
			<ReaderView
				bind:this={readerView}
				{bookBlob}
				{savedLocation}
				onRelocate={handleRelocate}
				onTocLoaded={handleTocLoaded}
				onLoad={handleLoad}
			/>
			<!-- Touch/click tap zones overlay -->
			<!-- svelte-ignore a11y_click_events_have_key_events -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="absolute inset-0 z-10"
				onclick={handleViewportClick}
			></div>
		</div>
	{/if}
</div>
