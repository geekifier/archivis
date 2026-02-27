<script lang="ts">
	import { api, ApiError } from '$lib/api/index.js';
	import type { WatchedDirectoryResponse } from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import WatchedDirectoryRow from './WatchedDirectoryRow.svelte';
	import AddWatchedDirectoryDialog from './AddWatchedDirectoryDialog.svelte';

	let directories = $state<WatchedDirectoryResponse[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let watcherDisabled = $state(false);
	let dialogOpen = $state(false);
	let dialogComponent = $state<AddWatchedDirectoryDialog | null>(null);

	async function fetchDirectories() {
		loading = true;
		error = null;
		watcherDisabled = false;
		try {
			directories = await api.watchedDirectories.list();
		} catch (err) {
			if (err instanceof ApiError && err.status === 503) {
				watcherDisabled = true;
				// Still try to show directories from the list endpoint
				// (the list endpoint may work even if watcher is disabled)
				directories = [];
			} else {
				error = err instanceof Error ? err.message : 'Failed to load watched directories';
			}
		} finally {
			loading = false;
		}
	}

	function handleAdd(dir: WatchedDirectoryResponse) {
		// Check if it's an update (edit mode)
		const existingIndex = directories.findIndex((d) => d.id === dir.id);
		if (existingIndex >= 0) {
			directories = directories.map((d) => (d.id === dir.id ? dir : d));
		} else {
			directories = [...directories, dir];
		}
	}

	function handleUpdate(dir: WatchedDirectoryResponse) {
		directories = directories.map((d) => (d.id === dir.id ? dir : d));
	}

	function handleDelete(id: string) {
		directories = directories.filter((d) => d.id !== id);
	}

	function handleEdit(dir: WatchedDirectoryResponse) {
		dialogComponent?.openForEdit(dir);
	}

	function handleAddNew() {
		dialogComponent?.resetAndOpen();
	}

	$effect(() => {
		fetchDirectories();
	});
</script>

<div class="rounded-lg border border-border bg-card">
	<div class="border-b border-border px-6 py-4">
		<div class="flex items-center justify-between">
			<div>
				<h2 class="text-base font-semibold">Watched Directories</h2>
				<p class="mt-0.5 text-xs text-muted-foreground">
					Monitor directories for new ebook files and automatically import them.
				</p>
			</div>
			{#if !loading}
				<Button variant="outline" size="sm" onclick={handleAddNew}>
					<svg
						class="mr-1.5 size-4"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<path d="M5 12h14" />
						<path d="M12 5v14" />
					</svg>
					Add Directory
				</Button>
			{/if}
		</div>
	</div>

	<!-- Watcher disabled banner -->
	{#if watcherDisabled}
		<div class="px-6 py-4">
			<div
				class="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-700 dark:text-amber-400"
			>
				<svg
					class="mt-0.5 size-4 shrink-0"
					xmlns="http://www.w3.org/2000/svg"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<circle cx="12" cy="12" r="10" />
					<line x1="12" x2="12" y1="8" y2="12" />
					<line x1="12" x2="12.01" y1="16" y2="16" />
				</svg>
				<div>
					<p class="font-medium">Filesystem watching is disabled</p>
					<p class="mt-1">
						Set <code class="rounded bg-amber-500/10 px-1 py-0.5 font-mono text-xs"
							>watcher.enabled = true</code
						>
						in your configuration file or
						<code class="rounded bg-amber-500/10 px-1 py-0.5 font-mono text-xs"
							>ARCHIVIS_WATCHER__ENABLED=true</code
						>
						environment variable and restart to enable it.
					</p>
				</div>
			</div>
		</div>
	{/if}

	<!-- Error state -->
	{#if error}
		<div class="px-6 py-4">
			<div
				class="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
			>
				<svg
					class="size-4 shrink-0"
					xmlns="http://www.w3.org/2000/svg"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<circle cx="12" cy="12" r="10" />
					<line x1="12" x2="12" y1="8" y2="12" />
					<line x1="12" x2="12.01" y1="16" y2="16" />
				</svg>
				<span>{error}</span>
			</div>
		</div>
	{/if}

	<!-- Loading state -->
	{#if loading}
		<div class="flex items-center justify-center px-6 py-8">
			<span class="text-sm text-muted-foreground">Loading watched directories...</span>
		</div>
	{:else if !error}
		<!-- Directory list -->
		{#if directories.length === 0}
			<div class="px-6 py-8 text-center">
				<p class="text-sm text-muted-foreground">
					No directories are being watched. Add a directory to automatically import new
					ebook files.
				</p>
			</div>
		{:else}
			<div class="divide-y divide-border">
				{#each directories as dir (dir.id)}
					<WatchedDirectoryRow
						directory={dir}
						onupdate={handleUpdate}
						ondelete={handleDelete}
						onedit={handleEdit}
					/>
				{/each}
			</div>
		{/if}
	{/if}
</div>

<AddWatchedDirectoryDialog
	bind:this={dialogComponent}
	bind:open={dialogOpen}
	onadd={handleAdd}
	onclose={() => {}}
/>
