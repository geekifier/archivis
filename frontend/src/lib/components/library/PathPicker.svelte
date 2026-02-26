<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { FsEntry } from '$lib/api/index.js';
	import { formatFileSize } from '$lib/utils.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';

	interface Props {
		value: string;
		open: boolean;
		mode?: 'directory' | 'file';
		title?: string;
		onselect?: (path: string, hasContent: boolean) => void;
	}

	let { value = $bindable(), open = $bindable(), mode = 'directory', title, onselect }: Props = $props();

	let currentPath = $state('/');
	let entries = $state<FsEntry[]>([]);
	let fileCount = $state(0);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let manualInput = $state('');

	const dialogTitle = $derived(title ?? (mode === 'directory' ? 'Select Directory' : 'Select File'));

	// Navigate to a path when the dialog opens.
	$effect(() => {
		if (open) {
			navigateTo(value || undefined);
		}
	});

	async function navigateTo(path?: string) {
		loading = true;
		error = null;
		try {
			const result = await api.filesystem.browse(path, mode === 'directory');
			currentPath = result.path;
			manualInput = result.path;
			entries = result.entries;
			fileCount = result.file_count;
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to browse directory';
			entries = [];
			fileCount = 0;
		} finally {
			loading = false;
		}
	}

	function handleEntryClick(entry: FsEntry) {
		if (entry.is_dir) {
			navigateTo(currentPath === '/' ? `/${entry.name}` : `${currentPath}/${entry.name}`);
		} else if (mode === 'file') {
			// In file mode, select file and close.
			value = currentPath === '/' ? `/${entry.name}` : `${currentPath}/${entry.name}`;
			open = false;
		}
	}

	function handleGoUp() {
		// Navigate to parent by removing the last path segment.
		const parent = currentPath.replace(/\/[^/]+$/, '') || '/';
		navigateTo(parent);
	}

	function handleManualGo() {
		const trimmed = manualInput.trim();
		if (trimmed) {
			navigateTo(trimmed);
		}
	}

	function handleSelect() {
		value = currentPath;
		open = false;
		onselect?.(currentPath, entries.length > 0 || fileCount > 0);
	}

	function handleOpenChange(isOpen: boolean) {
		open = isOpen;
		if (!isOpen) {
			error = null;
		}
	}

	// Build breadcrumb segments from currentPath.
	// The root segment always uses label "/" with no preceding separator.
	// Subsequent segments use their directory name with a "/" separator before them.
	const breadcrumbs = $derived.by(() => {
		const parts = currentPath.split('/').filter(Boolean);
		const crumbs: Array<{ label: string; path: string }> = [];
		let acc = '';
		for (const part of parts) {
			acc += `/${part}`;
			crumbs.push({ label: part, path: acc });
		}
		return crumbs;
	});

	const isAtRoot = $derived(currentPath === '/');
</script>

<Dialog.Root open={open} onOpenChange={handleOpenChange}>
	<Dialog.Content class="flex max-h-[80vh] flex-col sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>{dialogTitle}</Dialog.Title>
			<Dialog.Description>
				Navigate to a {mode === 'directory' ? 'directory' : 'file'} on the server.
			</Dialog.Description>
		</Dialog.Header>

		<!-- Manual path input -->
		<div class="flex gap-2">
			<div class="flex-1">
				<Input
					type="text"
					placeholder="/path/to/directory"
					bind:value={manualInput}
					onkeydown={(e: KeyboardEvent) => {
						if (e.key === 'Enter') handleManualGo();
					}}
					disabled={loading}
				/>
			</div>
			<Button variant="outline" size="sm" onclick={handleManualGo} disabled={loading}>
				Go
			</Button>
		</div>

		<!-- Breadcrumb navigation -->
		<nav class="flex flex-wrap items-center gap-0.5 text-sm" aria-label="Path breadcrumbs">
			<button
				class="rounded px-1 py-0.5 text-sm hover:bg-muted {breadcrumbs.length === 0
					? 'font-medium text-foreground'
					: 'text-muted-foreground hover:text-foreground'}"
				onclick={() => navigateTo('/')}
				disabled={loading}
			>
				/
			</button>
			{#each breadcrumbs as crumb, i (crumb.path)}
				<span class="text-muted-foreground/50">/</span>
				<button
					class="rounded px-1 py-0.5 text-sm hover:bg-muted {i === breadcrumbs.length - 1
						? 'font-medium text-foreground'
						: 'text-muted-foreground hover:text-foreground'}"
					onclick={() => navigateTo(crumb.path)}
					disabled={loading}
				>
					{crumb.label}
				</button>
			{/each}
		</nav>

		<!-- Error state -->
		{#if error}
			<div class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
				{error}
			</div>
		{/if}

		<!-- Directory listing -->
		<div class="min-h-[200px] flex-1 overflow-y-auto rounded-md border border-border">
			{#if loading}
				<div class="flex items-center justify-center p-8">
					<svg
						class="size-5 animate-spin text-muted-foreground"
						xmlns="http://www.w3.org/2000/svg"
						fill="none"
						viewBox="0 0 24 24"
					>
						<circle
							class="opacity-25"
							cx="12"
							cy="12"
							r="10"
							stroke="currentColor"
							stroke-width="4"
						></circle>
						<path
							class="opacity-75"
							fill="currentColor"
							d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
						></path>
					</svg>
				</div>
			{:else}
				<!-- Up button (always shown when not at root) -->
				{#if !isAtRoot}
					<button
						class="flex w-full items-center gap-2 border-b border-border px-3 py-2 text-sm hover:bg-muted"
						onclick={handleGoUp}
					>
						<svg
							class="size-4 text-muted-foreground"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="2"
							stroke-linecap="round"
							stroke-linejoin="round"
						>
							<path d="m15 18-6-6 6-6" />
						</svg>
						<span class="text-muted-foreground">..</span>
					</button>
				{/if}
				{#if entries.length === 0 && !error}
					<div class="flex items-center justify-center p-8 text-sm text-muted-foreground">
						{#if fileCount > 0}
							{fileCount} {fileCount === 1 ? 'file' : 'files'} in this directory
						{:else}
							Empty directory
						{/if}
					</div>
				{/if}
				{#each entries as entry (entry.name)}
					<button
						class="flex w-full items-center gap-2 px-3 py-2 text-sm hover:bg-muted {!entry.is_dir && mode === 'directory' ? 'opacity-50' : ''}"
						onclick={() => handleEntryClick(entry)}
						disabled={!entry.is_dir && mode === 'directory'}
					>
						{#if entry.is_dir}
							<!-- Folder icon -->
							<svg
								class="size-4 shrink-0 text-blue-500"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
							</svg>
						{:else}
							<!-- File icon -->
							<svg
								class="size-4 shrink-0 text-muted-foreground"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
								<path d="M14 2v4a2 2 0 0 0 2 2h4" />
							</svg>
						{/if}
						<span class="flex-1 truncate text-left">{entry.name}</span>
						{#if !entry.is_dir && entry.size > 0}
							<span class="shrink-0 text-xs text-muted-foreground">
								{formatFileSize(entry.size)}
							</span>
						{/if}
					</button>
				{/each}
			{/if}
		</div>

		<Dialog.Footer class="flex justify-between gap-2">
			<Dialog.Close>Cancel</Dialog.Close>
			{#if mode === 'directory'}
				<Button onclick={handleSelect} disabled={loading || !!error}>
					Select This Directory
				</Button>
			{/if}
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
