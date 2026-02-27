<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type {
		FsDetectionResponse,
		WatchedDirectoryResponse,
		WatchMode
	} from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Switch } from '$lib/components/ui/switch/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import PathPicker from '$lib/components/library/PathPicker.svelte';

	interface Props {
		open: boolean;
		onadd: (dir: WatchedDirectoryResponse) => void;
		onclose: () => void;
	}

	let { open = $bindable(), onadd, onclose }: Props = $props();

	let watchPath = $state('');
	let pickerOpen = $state(false);
	let detection = $state<FsDetectionResponse | null>(null);
	let detecting = $state(false);
	let detectError = $state<string | null>(null);
	let watchMode = $state<WatchMode>('poll');
	let pollInterval = $state(30);
	let adding = $state(false);
	let addError = $state<string | null>(null);

	// Toggle states
	let importExisting = $state(false);
	let deleteSourceAfterImport = $state(false);
	let deleteSourceLoading = $state(false);

	// Editing mode reuse
	let editingDir = $state<WatchedDirectoryResponse | null>(null);

	const isEditing = $derived(editingDir !== null);
	const dialogTitle = $derived(isEditing ? 'Edit Watched Directory' : 'Add Watched Directory');

	const showNativeWarning = $derived(
		watchMode === 'native' &&
			detection !== null &&
			detection.native_likely_works !== 'likely'
	);

	async function loadDeleteSourceSetting() {
		deleteSourceLoading = true;
		try {
			const response = await api.settings.get();
			const entry = response.settings.find(
				(s) => s.key === 'watcher.delete_source_after_import'
			);
			if (entry) {
				deleteSourceAfterImport = entry.effective_value === true;
			}
		} catch {
			// Silently fall back to default (false)
		} finally {
			deleteSourceLoading = false;
		}
	}

	async function handleDeleteSourceToggle(checked: boolean) {
		deleteSourceAfterImport = checked;
		try {
			await api.settings.update({ 'watcher.delete_source_after_import': checked });
		} catch {
			// Revert on failure
			deleteSourceAfterImport = !checked;
		}
	}

	export function openForEdit(dir: WatchedDirectoryResponse) {
		editingDir = dir;
		watchPath = dir.path;
		watchMode = dir.watch_mode;
		pollInterval = dir.effective_poll_interval_secs;
		detection = dir.detected_fs;
		detectError = null;
		addError = null;
		open = true;
		loadDeleteSourceSetting();
	}

	export function resetAndOpen() {
		editingDir = null;
		watchPath = '';
		watchMode = 'poll';
		pollInterval = 30;
		detection = null;
		detectError = null;
		addError = null;
		importExisting = false;
		open = true;
		loadDeleteSourceSetting();
	}

	function handleOpenChange(isOpen: boolean) {
		open = isOpen;
		if (!isOpen) {
			onclose();
		}
	}

	async function runDetection(path: string) {
		detecting = true;
		detectError = null;
		detection = null;
		try {
			detection = await api.watchedDirectories.detectFilesystem(path);
		} catch (err) {
			detectError = err instanceof Error ? err.message : 'Failed to detect filesystem type';
		} finally {
			detecting = false;
		}
	}

	function handlePathSelected(path: string) {
		watchPath = path;
		runDetection(path);
	}

	async function handleSubmit() {
		if (!watchPath.trim()) return;

		adding = true;
		addError = null;

		try {
			if (isEditing && editingDir) {
				const updated = await api.watchedDirectories.update(editingDir.id, {
					watch_mode: watchMode,
					poll_interval_secs: watchMode === 'poll' ? pollInterval : null
				});
				onadd(updated);
			} else {
				const dir = await api.watchedDirectories.add({
					path: watchPath.trim(),
					watch_mode: watchMode,
					poll_interval_secs: watchMode === 'poll' ? pollInterval : undefined
				});
				onadd(dir);

				// Fire-and-forget: trigger scan for existing files
				if (importExisting) {
					api.watchedDirectories.triggerScan(dir.id).catch(() => {
						// Scan trigger is best-effort; ignore errors
					});
				}
			}
			open = false;
		} catch (err) {
			addError = err instanceof Error ? err.message : 'Failed to save watched directory';
		} finally {
			adding = false;
		}
	}
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>{dialogTitle}</Dialog.Title>
			<Dialog.Description>
				{#if isEditing}
					Change watch mode or polling interval for this directory.
				{:else}
					Select a directory to monitor for new ebook files.
				{/if}
			</Dialog.Description>
		</Dialog.Header>

		<div class="space-y-4">
			<!-- Path selection -->
			{#if !isEditing}
				<div>
					<label for="watch-path" class="mb-1.5 block text-sm font-medium">
						Directory Path
					</label>
					<div class="flex gap-2">
						<div class="flex-1">
							<Input
								id="watch-path"
								type="text"
								placeholder="/path/to/watch"
								bind:value={watchPath}
								readonly
							/>
						</div>
						<Button variant="outline" size="sm" onclick={() => (pickerOpen = true)}>
							Browse
						</Button>
					</div>
				</div>
			{:else}
				<div>
					<p class="mb-1.5 text-sm font-medium">Directory Path</p>
					<p class="font-mono text-sm text-muted-foreground">{watchPath}</p>
				</div>
			{/if}

			<!-- Toggle settings -->
			{#if !isEditing}
				<div class="flex items-center justify-between gap-4">
					<div class="space-y-0.5">
						<Label for="import-existing" class="text-sm font-medium">
							Import existing files
						</Label>
						<p class="text-xs text-muted-foreground">
							Scan and import all ebook files already present in this directory when it
							is first added.
						</p>
					</div>
					<Switch
						id="import-existing"
						checked={importExisting}
						onCheckedChange={(checked) => (importExisting = checked)}
					/>
				</div>
			{/if}

			<div class="flex items-center justify-between gap-4">
				<div class="space-y-0.5">
					<Label for="delete-source" class="text-sm font-medium">
						Delete source after import
					</Label>
					<p class="text-xs text-muted-foreground">
						Remove the original file from this directory after it has been successfully
						imported into the library.
					</p>
				</div>
				<Switch
					id="delete-source"
					checked={deleteSourceAfterImport}
					disabled={deleteSourceLoading}
					onCheckedChange={(checked) => handleDeleteSourceToggle(checked)}
				/>
			</div>

			<!-- Detection result -->
			{#if detecting}
				<div
					class="flex items-center gap-2 rounded-lg border border-border bg-muted/50 px-4 py-3 text-sm text-muted-foreground"
				>
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
					Detecting filesystem type...
				</div>
			{/if}

			{#if detectError}
				<div
					class="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
				>
					{detectError}
				</div>
			{/if}

			{#if detection}
				<div class="space-y-2 rounded-lg border border-border bg-muted/50 px-4 py-3">
					<div class="flex items-center gap-2 text-sm">
						<span class="font-medium">Filesystem:</span>
						<span class="font-mono text-muted-foreground">{detection.fs_type}</span>
					</div>
					<p class="text-sm text-muted-foreground">{detection.explanation}</p>
					<p class="text-xs text-muted-foreground/70">
						If this path is mounted from a network share via a container runtime (Docker,
						Kubernetes), the detection may not reflect the actual backing storage. When in
						doubt, use polling.
					</p>
				</div>
			{/if}

			<!-- Watch mode selector -->
			{#if watchPath || isEditing}
				<fieldset>
					<legend class="mb-2 text-sm font-medium">Watch Mode</legend>
					<div class="space-y-2">
						<!-- Polling option -->
						<label
							class="flex cursor-pointer gap-3 rounded-lg border px-4 py-3 transition-colors {watchMode ===
							'poll'
								? 'border-primary bg-primary/5'
								: 'border-border hover:bg-muted/50'}"
						>
							<input
								type="radio"
								name="watch-mode"
								value="poll"
								bind:group={watchMode}
								class="mt-0.5"
							/>
							<div class="flex-1">
								<div class="flex items-center gap-2 text-sm font-medium">
									Polling
									<span
										class="inline-flex items-center rounded-full bg-green-500/10 px-1.5 py-0.5 text-[10px] font-medium text-green-700 dark:text-green-400"
									>
										recommended
									</span>
								</div>
								<p class="mt-0.5 text-xs text-muted-foreground">
									Periodically scans for changes. Works everywhere — local disks, network
									shares, container volumes, cloud storage mounts. Changes are detected
									within the polling interval.
								</p>
							</div>
						</label>

						<!-- Native option -->
						<label
							class="flex cursor-pointer gap-3 rounded-lg border px-4 py-3 transition-colors {watchMode ===
							'native'
								? 'border-primary bg-primary/5'
								: 'border-border hover:bg-muted/50'}"
						>
							<input
								type="radio"
								name="watch-mode"
								value="native"
								bind:group={watchMode}
								class="mt-0.5"
							/>
							<div class="flex-1">
								<p class="text-sm font-medium">Native events</p>
								<p class="mt-0.5 text-xs text-muted-foreground">
									Uses OS-level filesystem notifications (inotify on Linux, FSEvents on
									macOS) for near-instant detection. Only works on local filesystems.
									Does NOT detect changes made by other machines on network shares (NFS,
									SMB).
								</p>
							</div>
						</label>
					</div>
				</fieldset>

				<!-- Native warning -->
				{#if showNativeWarning}
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
							<path
								d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"
							/>
							<path d="M12 9v4" />
							<path d="M12 17h.01" />
						</svg>
						<span>
							Detection suggests this path may not support native events. If you
							experience missed imports, switch to polling.
						</span>
					</div>
				{/if}

				<!-- Polling interval -->
				{#if watchMode === 'poll'}
					<div>
						<label for="poll-interval" class="mb-1.5 block text-sm font-medium">
							Polling Interval (seconds)
						</label>
						<Input
							id="poll-interval"
							type="number"
							min={5}
							max={3600}
							bind:value={pollInterval}
						/>
						<p class="mt-1 text-xs text-muted-foreground">
							How often to check for new files. Lower values detect changes faster but use
							more resources.
						</p>
					</div>
				{/if}
			{/if}

			<!-- Add error -->
			{#if addError}
				<div
					class="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
				>
					{addError}
				</div>
			{/if}
		</div>

		<Dialog.Footer>
			<Dialog.Close>Cancel</Dialog.Close>
			<Button
				onclick={handleSubmit}
				disabled={adding || (!watchPath.trim() && !isEditing)}
			>
				{#if adding}
					{isEditing ? 'Saving...' : 'Adding...'}
				{:else}
					{isEditing ? 'Save Changes' : 'Add Directory'}
				{/if}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<PathPicker
	bind:value={watchPath}
	bind:open={pickerOpen}
	mode="directory"
	title="Select Directory to Watch"
	onselect={(path) => handlePathSelected(path)}
/>
