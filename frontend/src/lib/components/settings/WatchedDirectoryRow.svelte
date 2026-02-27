<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { WatchedDirectoryResponse } from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';

	interface Props {
		directory: WatchedDirectoryResponse;
		deleteSourceAfterImport: boolean;
		onupdate: (dir: WatchedDirectoryResponse) => void;
		ondelete: (id: string) => void;
		onedit: (dir: WatchedDirectoryResponse) => void;
	}

	let { directory, deleteSourceAfterImport, onupdate, ondelete, onedit }: Props = $props();

	let toggling = $state(false);
	let scanning = $state(false);
	let deleting = $state(false);
	let deleteOpen = $state(false);
	let errorExpanded = $state(false);

	const statusColor = $derived.by(() => {
		if (directory.last_error) return 'bg-red-500';
		if (
			directory.watch_mode === 'native' &&
			directory.detected_fs?.native_likely_works !== 'likely'
		) {
			return 'bg-yellow-500';
		}
		if (!directory.enabled) return 'bg-zinc-400';
		return 'bg-green-500';
	});

	const statusLabel = $derived.by(() => {
		if (directory.last_error) return 'Error';
		if (
			directory.watch_mode === 'native' &&
			directory.detected_fs?.native_likely_works !== 'likely'
		) {
			return 'Warning';
		}
		if (!directory.enabled) return 'Disabled';
		return 'Healthy';
	});

	const modeBadgeClass = $derived(
		directory.watch_mode === 'native'
			? 'bg-green-500/10 text-green-700 dark:text-green-400'
			: 'bg-blue-500/10 text-blue-700 dark:text-blue-400'
	);

	const modeBadgeText = $derived(
		directory.watch_mode === 'native'
			? 'native'
			: `polling \u00B7 ${directory.effective_poll_interval_secs}s`
	);

	const deleteBadgeClass = $derived(
		deleteSourceAfterImport
			? 'bg-red-500/10 text-red-700 dark:text-red-400'
			: 'bg-green-500/10 text-green-700 dark:text-green-400'
	);

	const deleteBadgeText = $derived(
		deleteSourceAfterImport ? 'Delete Imported Files' : 'Keep Imported Files'
	);

	async function toggleEnabled() {
		toggling = true;
		try {
			const updated = await api.watchedDirectories.update(directory.id, {
				enabled: !directory.enabled
			});
			onupdate(updated);
		} catch {
			// Silently handle -- the row will remain in its current state
		} finally {
			toggling = false;
		}
	}

	async function handleScan() {
		scanning = true;
		try {
			await api.watchedDirectories.triggerScan(directory.id);
		} catch {
			// Scan failure is shown via task status
		} finally {
			scanning = false;
		}
	}

	async function handleDelete() {
		deleting = true;
		try {
			await api.watchedDirectories.delete(directory.id);
			ondelete(directory.id);
			deleteOpen = false;
		} catch {
			// Delete failure -- row remains
		} finally {
			deleting = false;
		}
	}
</script>

<div class="px-6 py-4">
	<div class="flex items-start justify-between gap-4">
		<!-- Left: path, mode badge, detection hint, status -->
		<div class="min-w-0 flex-1">
			<div class="flex items-center gap-2">
				<!-- Status dot -->
				<span
					class="inline-block size-2.5 shrink-0 rounded-full {statusColor}"
					title={statusLabel}
				></span>

				<!-- Path -->
				<span class="truncate font-mono text-sm" title={directory.path}>
					{directory.path}
				</span>
			</div>

			<div class="mt-1 flex flex-wrap items-center gap-2">
				<!-- Watch mode badge -->
				<span
					class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium {modeBadgeClass}"
				>
					{modeBadgeText}
				</span>

				<!-- Delete source badge -->
				<span
					class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium {deleteBadgeClass}"
				>
					{deleteBadgeText}
				</span>

			</div>

			<!-- Error display -->
			{#if directory.last_error}
				<div class="mt-2">
					<button
						class="flex items-center gap-1 text-xs text-red-600 dark:text-red-400"
						onclick={() => (errorExpanded = !errorExpanded)}
					>
						<svg
							class="size-3.5 shrink-0 transition-transform {errorExpanded
								? 'rotate-90'
								: ''}"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="2"
							stroke-linecap="round"
							stroke-linejoin="round"
						>
							<path d="m9 18 6-6-6-6" />
						</svg>
						<span>Error details</span>
					</button>
					{#if errorExpanded}
						<p
							class="mt-1 rounded-md border border-red-500/20 bg-red-500/5 px-3 py-2 font-mono text-xs text-red-600 dark:text-red-400"
						>
							{directory.last_error}
						</p>
					{/if}
				</div>
			{/if}
		</div>

		<!-- Right: actions -->
		<div class="flex shrink-0 items-center gap-1">
			<!-- Enabled toggle -->
			<button
				type="button"
				role="switch"
				aria-checked={directory.enabled}
				aria-label="Toggle watching"
				title={directory.enabled ? 'Disable watching' : 'Enable watching'}
				class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors
					{directory.enabled ? 'bg-primary' : 'bg-muted'}"
				disabled={toggling}
				onclick={toggleEnabled}
			>
				<span
					class="pointer-events-none inline-block size-4 rounded-full bg-background shadow-sm ring-0 transition-transform
						{directory.enabled ? 'translate-x-4' : 'translate-x-0'}"
				></span>
			</button>

			<!-- Scan Now -->
			<Button
				variant="ghost"
				size="icon-sm"
				onclick={handleScan}
				disabled={scanning || !directory.enabled}
				title="Scan now"
			>
				{#if scanning}
					<svg
						class="size-4 animate-spin"
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
				{:else}
					<svg
						class="size-4"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
						<path d="M3 3v5h5" />
						<path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
						<path d="M21 21v-5h-5" />
					</svg>
				{/if}
			</Button>

			<!-- Edit -->
			<Button
				variant="ghost"
				size="icon-sm"
				onclick={() => onedit(directory)}
				title="Edit settings"
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
					<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
					<path d="m15 5 4 4" />
				</svg>
			</Button>

			<!-- Delete -->
			<AlertDialog.Root bind:open={deleteOpen}>
				<AlertDialog.Trigger>
					{#snippet child({ props })}
						<Button variant="ghost" size="icon-sm" title="Delete" {...props}>
							<svg
								class="size-4 text-destructive"
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
								<line x1="10" x2="10" y1="11" y2="17" />
								<line x1="14" x2="14" y1="11" y2="17" />
							</svg>
						</Button>
					{/snippet}
				</AlertDialog.Trigger>
				<AlertDialog.Content>
					<AlertDialog.Header>
						<AlertDialog.Title>Remove Watched Directory</AlertDialog.Title>
						<AlertDialog.Description>
							This will stop watching this directory. Already imported books will not be
							affected.
						</AlertDialog.Description>
					</AlertDialog.Header>
					<AlertDialog.Footer>
						<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
						<AlertDialog.Action onclick={handleDelete} disabled={deleting}>
							{deleting ? 'Removing...' : 'Remove'}
						</AlertDialog.Action>
					</AlertDialog.Footer>
				</AlertDialog.Content>
			</AlertDialog.Root>
		</div>
	</div>
</div>
