<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api/index.js';
	import type { BookDetail } from '$lib/api/types.js';

	const bookId = $derived(page.params.bookId ?? '');
	const fileId = $derived(page.params.fileId ?? '');

	let book = $state<BookDetail | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let viewContainer = $state<HTMLDivElement | null>(null);

	/** Load a script from a URL, returning a promise that resolves when loaded. */
	function loadScript(src: string): Promise<void> {
		return new Promise((resolve, reject) => {
			if (document.querySelector(`script[src="${src}"]`)) {
				resolve();
				return;
			}
			const script = document.createElement('script');
			script.type = 'module';
			script.src = src;
			script.onload = () => resolve();
			script.onerror = () => reject(new Error(`Failed to load script: ${src}`));
			document.head.append(script);
		});
	}

	$effect(() => {
		void loadReader();
	});

	async function loadReader() {
		loading = true;
		error = null;
		try {
			// 1. Fetch book metadata
			book = await api.books.get(bookId);
			// 2. Fetch file as Blob
			const blob = await api.reader.fetchFileBlob(bookId, fileId);
			// 3. Load foliate-js as a runtime script (not bundled by Vite)
			await loadScript('/vendor/foliate-js/view.js');
			// 4. Create and mount <foliate-view> (custom element requires direct DOM manipulation)
			if (viewContainer) {
				const view = document.createElement('foliate-view');
				view.style.width = '100%';
				view.style.height = '100%';
				// eslint-disable-next-line svelte/no-dom-manipulating -- foliate-js custom element must be mounted imperatively
				viewContainer.append(view);
				await (view as unknown as { open: (blob: Blob) => Promise<void> }).open(blob);
			}
		} catch (err: unknown) {
			error = err instanceof Error ? err.message : 'Failed to load reader';
		} finally {
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>{book?.title ?? 'Reader'} — Archivis</title>
</svelte:head>

<div class="flex h-screen flex-col bg-background">
	<!-- Minimal header for now -->
	<div class="flex items-center gap-3 border-b border-border px-4 py-2">
		<button
			onclick={() => goto(`/books/${bookId}`)}
			class="text-sm text-muted-foreground hover:text-foreground"
		>
			&larr; Back
		</button>
		{#if book}
			<span class="truncate text-sm font-medium">{book.title}</span>
		{/if}
	</div>

	<!-- Reader viewport -->
	{#if loading}
		<div class="flex flex-1 items-center justify-center">
			<div class="text-muted-foreground">Loading book...</div>
		</div>
	{:else if error}
		<div class="flex flex-1 items-center justify-center">
			<div class="text-destructive">{error}</div>
		</div>
	{:else}
		<div bind:this={viewContainer} class="flex-1 overflow-hidden"></div>
	{/if}
</div>
