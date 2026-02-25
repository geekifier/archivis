<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type {
		ScanManifestResponse,
		TaskResponse,
		TaskProgressEvent,
		TaskStatus
	} from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import {
		Card,
		CardContent,
		CardDescription,
		CardHeader,
		CardTitle
	} from '$lib/components/ui/card/index.js';
	import PathPicker from '$lib/components/library/PathPicker.svelte';
	import { formatFileSize } from '$lib/utils.js';

	// --- Mode selection ---
	type ImportMode = 'upload' | 'scan';
	let mode = $state<ImportMode>('upload');

	// --- Path picker state ---
	let pickerOpen = $state(false);

	// --- Upload state ---
	let selectedFiles = $state<File[]>([]);
	let uploading = $state(false);
	let uploadError = $state<string | null>(null);
	let dragOver = $state(false);

	// --- Scan state ---
	let scanPath = $state('');
	let scanning = $state(false);
	let scanError = $state<string | null>(null);
	let manifest = $state<ScanManifestResponse | null>(null);
	let startingImport = $state(false);

	// --- Task progress state ---
	let activeTaskIds = $state<string[]>([]);
	let taskProgressMap = $state<Record<string, TaskProgressEvent>>({});
	let eventSources: EventSource[] = [];

	// --- Recent tasks ---
	let recentTasks = $state<TaskResponse[]>([]);
	let loadingTasks = $state(true);
	let tasksError = $state<string | null>(null);

	// Derived: are we tracking any tasks?
	const hasActiveTasks = $derived(activeTaskIds.length > 0);

	// --- Load recent tasks on mount ---
	$effect(() => {
		loadRecentTasks();
		return () => {
			// Cleanup all SSE connections on unmount
			for (const es of eventSources) {
				es.close();
			}
			eventSources = [];
		};
	});

	async function loadRecentTasks() {
		loadingTasks = true;
		tasksError = null;
		try {
			recentTasks = await api.tasks.list();
		} catch (err) {
			tasksError = err instanceof Error ? err.message : 'Failed to load tasks';
		} finally {
			loadingTasks = false;
		}
	}

	// --- File upload handling ---
	const ACCEPTED_EXTENSIONS = ['.epub', '.pdf', '.mobi', '.cbz', '.fb2', '.txt', '.djvu', '.azw3'];

	function isAcceptedFile(file: File): boolean {
		const name = file.name.toLowerCase();
		return ACCEPTED_EXTENSIONS.some((ext) => name.endsWith(ext));
	}

	function handleFileSelect(e: Event) {
		const input = e.target as HTMLInputElement;
		if (input.files) {
			addFiles(Array.from(input.files));
		}
		// Reset input so the same file can be selected again
		input.value = '';
	}

	function addFiles(files: File[]) {
		const valid = files.filter(isAcceptedFile);
		if (valid.length > 0) {
			selectedFiles = [...selectedFiles, ...valid];
		}
		uploadError = null;
	}

	function removeFile(index: number) {
		selectedFiles = selectedFiles.filter((_, i) => i !== index);
	}

	function handleDragOver(e: DragEvent) {
		e.preventDefault();
		dragOver = true;
	}

	function handleDragLeave() {
		dragOver = false;
	}

	function handleDrop(e: DragEvent) {
		e.preventDefault();
		dragOver = false;
		if (e.dataTransfer?.files) {
			addFiles(Array.from(e.dataTransfer.files));
		}
	}

	async function handleUpload() {
		if (selectedFiles.length === 0) return;

		uploading = true;
		uploadError = null;

		try {
			const response = await api.import.upload(selectedFiles);
			const taskIds = response.tasks.map((t) => t.task_id);
			selectedFiles = [];
			startTrackingTasks(taskIds);
		} catch (err) {
			uploadError = err instanceof Error ? err.message : 'Upload failed';
		} finally {
			uploading = false;
		}
	}

	// --- Directory scan handling ---
	async function handleScan() {
		if (!scanPath.trim()) return;

		scanning = true;
		scanError = null;
		manifest = null;

		try {
			manifest = await api.import.scan(scanPath.trim());
		} catch (err) {
			scanError = err instanceof Error ? err.message : 'Scan failed';
		} finally {
			scanning = false;
		}
	}

	async function handleStartImport() {
		if (!scanPath.trim()) return;

		startingImport = true;
		scanError = null;

		try {
			const response = await api.import.startImport(scanPath.trim());
			manifest = null;
			startTrackingTasks([response.task_id]);
		} catch (err) {
			scanError = err instanceof Error ? err.message : 'Failed to start import';
		} finally {
			startingImport = false;
		}
	}

	// --- SSE task progress tracking ---
	function startTrackingTasks(taskIds: string[]) {
		activeTaskIds = [...activeTaskIds, ...taskIds];

		for (const taskId of taskIds) {
			const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);

			es.addEventListener('task:progress', (event: MessageEvent) => {
				try {
					const data = JSON.parse(event.data) as TaskProgressEvent;
					taskProgressMap = { ...taskProgressMap, [taskId]: data };
				} catch {
					// Ignore malformed events
				}
			});

			es.addEventListener('task:complete', (event: MessageEvent) => {
				try {
					const data = JSON.parse(event.data) as TaskProgressEvent;
					taskProgressMap = {
						...taskProgressMap,
						[taskId]: { ...data, status: 'completed' as TaskStatus, progress: 100 }
					};
				} catch {
					// Ignore malformed events
				}
				es.close();
				removeEventSource(es);
				loadRecentTasks();
			});

			es.addEventListener('task:error', (event: MessageEvent) => {
				try {
					const data = JSON.parse(event.data) as TaskProgressEvent;
					taskProgressMap = {
						...taskProgressMap,
						[taskId]: { ...data, status: 'failed' as TaskStatus }
					};
				} catch {
					// Ignore malformed events
				}
				es.close();
				removeEventSource(es);
				loadRecentTasks();
			});

			es.onerror = () => {
				es.close();
				removeEventSource(es);
			};

			eventSources.push(es);
		}
	}

	function removeEventSource(es: EventSource) {
		eventSources = eventSources.filter((e) => e !== es);
	}

	function dismissTask(taskId: string) {
		activeTaskIds = activeTaskIds.filter((id) => id !== taskId);
		const updated = { ...taskProgressMap };
		delete updated[taskId];
		taskProgressMap = updated;
	}

	function taskStatusLabel(status: TaskStatus): string {
		switch (status) {
			case 'pending':
				return 'Pending';
			case 'running':
				return 'Running';
			case 'completed':
				return 'Completed';
			case 'failed':
				return 'Failed';
			default:
				return status;
		}
	}

	function taskTypeLabel(taskType: string): string {
		switch (taskType) {
			case 'import_file':
				return 'File Import';
			case 'import_directory':
				return 'Directory Import';
			case 'scan_isbn':
				return 'ISBN Scan';
			default:
				return taskType;
		}
	}

	function formatRelativeTime(dateStr: string): string {
		const date = new Date(dateStr);
		const now = new Date();
		const diffMs = now.getTime() - date.getTime();
		const diffSec = Math.floor(diffMs / 1000);

		if (diffSec < 60) return 'just now';
		const diffMin = Math.floor(diffSec / 60);
		if (diffMin < 60) return `${diffMin}m ago`;
		const diffHr = Math.floor(diffMin / 60);
		if (diffHr < 24) return `${diffHr}h ago`;
		const diffDay = Math.floor(diffHr / 24);
		return `${diffDay}d ago`;
	}

	function statusColorClass(status: TaskStatus | string): string {
		switch (status) {
			case 'completed':
				return 'text-green-600 dark:text-green-400';
			case 'failed':
				return 'text-destructive';
			case 'running':
				return 'text-blue-600 dark:text-blue-400';
			default:
				return 'text-muted-foreground';
		}
	}
</script>

<div class="space-y-6">
	<div>
		<h1 class="text-3xl font-bold tracking-tight">Import</h1>
		<p class="text-muted-foreground">Import e-books into your library</p>
	</div>

	<!-- Mode tabs -->
	<div class="flex gap-1 rounded-lg border border-input bg-muted p-1">
		<button
			class="flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors
			{mode === 'upload' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
			onclick={() => (mode = 'upload')}
		>
			Upload Files
		</button>
		<button
			class="flex-1 rounded-md px-3 py-1.5 text-sm font-medium transition-colors
			{mode === 'scan' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
			onclick={() => (mode = 'scan')}
		>
			Scan Directory
		</button>
	</div>

	<!-- Upload mode -->
	{#if mode === 'upload'}
		<Card>
			<CardHeader>
				<CardTitle>Upload E-books</CardTitle>
				<CardDescription>
					Drag and drop files or click to browse. Supported formats: EPUB, PDF, MOBI, CBZ, FB2,
					TXT, DJVU, AZW3.
				</CardDescription>
			</CardHeader>
			<CardContent>
				<!-- Drop zone -->
				<button
					class="flex w-full cursor-pointer flex-col items-center justify-center rounded-lg border-2 border-dashed p-8 transition-colors
					{dragOver
						? 'border-primary bg-primary/5'
						: 'border-border hover:border-primary/50 hover:bg-muted/50'}"
					ondragover={handleDragOver}
					ondragleave={handleDragLeave}
					ondrop={handleDrop}
					onclick={() => document.getElementById('file-input')?.click()}
					type="button"
				>
					<svg
						class="mb-3 size-10 text-muted-foreground"
						xmlns="http://www.w3.org/2000/svg"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="1.5"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
						<polyline points="17 8 12 3 7 8" />
						<line x1="12" x2="12" y1="3" y2="15" />
					</svg>
					<p class="text-sm font-medium text-foreground">
						Drop files here or click to browse
					</p>
					<p class="mt-1 text-xs text-muted-foreground">
						EPUB, PDF, MOBI, CBZ, FB2, TXT, DJVU, AZW3
					</p>
				</button>

				<input
					id="file-input"
					type="file"
					multiple
					accept={ACCEPTED_EXTENSIONS.join(',')}
					onchange={handleFileSelect}
					class="hidden"
				/>

				<!-- Selected files list -->
				{#if selectedFiles.length > 0}
					<div class="mt-4 space-y-2">
						<div class="flex items-center justify-between">
							<Label class="text-sm font-medium">
								{selectedFiles.length} file{selectedFiles.length === 1 ? '' : 's'} selected
							</Label>
							<Button
								variant="ghost"
								size="sm"
								onclick={() => (selectedFiles = [])}
							>
								Clear all
							</Button>
						</div>
						<div class="max-h-48 space-y-1 overflow-y-auto rounded-md border border-border p-2">
							{#each selectedFiles as file, i (file.name + i)}
								<div
									class="flex items-center justify-between rounded px-2 py-1 text-sm hover:bg-muted"
								>
									<div class="flex items-center gap-2 overflow-hidden">
										<svg
											class="size-4 shrink-0 text-muted-foreground"
											xmlns="http://www.w3.org/2000/svg"
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
										<span class="truncate">{file.name}</span>
										<span class="shrink-0 text-xs text-muted-foreground">
											{formatFileSize(file.size)}
										</span>
									</div>
									<Button
										variant="ghost"
										size="icon-sm"
										onclick={() => removeFile(i)}
										aria-label="Remove file"
										class="size-6"
									>
										<svg
											class="size-3"
											xmlns="http://www.w3.org/2000/svg"
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
									</Button>
								</div>
							{/each}
						</div>
					</div>
				{/if}

				{#if uploadError}
					<p class="mt-3 text-sm text-destructive">{uploadError}</p>
				{/if}

				<!-- ISBN scan note -->
				<div class="mt-4 flex items-start gap-2 rounded-md border border-border bg-muted/50 px-3 py-2.5">
					<svg
						class="mt-0.5 size-4 shrink-0 text-muted-foreground"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
					>
						<circle cx="12" cy="12" r="10" />
						<path d="M12 16v-4" />
						<path d="M12 8h.01" />
					</svg>
					<div class="text-xs text-muted-foreground">
						<p>
							<strong class="font-medium text-foreground">Scan book content for ISBNs after import</strong>
						</p>
						<p class="mt-0.5">
							Controlled by the <code class="rounded bg-muted px-1 py-0.5 text-xs">scan_on_import</code>
							setting in the server's <code class="rounded bg-muted px-1 py-0.5 text-xs">[isbn_scan]</code> config section.
							When enabled, imported books are automatically scanned for ISBNs embedded in their content.
						</p>
					</div>
				</div>

				<div class="mt-4">
					<Button
						onclick={handleUpload}
						disabled={selectedFiles.length === 0 || uploading}
					>
						{#if uploading}
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
							Uploading...
						{:else}
							Upload {selectedFiles.length} file{selectedFiles.length === 1 ? '' : 's'}
						{/if}
					</Button>
				</div>
			</CardContent>
		</Card>
	{/if}

	<!-- Scan directory mode -->
	{#if mode === 'scan'}
		<Card>
			<CardHeader>
				<CardTitle>Scan Directory</CardTitle>
				<CardDescription>
					Scan a directory on the server for e-book files. Enter the absolute path to a directory
					accessible by the server.
				</CardDescription>
			</CardHeader>
			<CardContent>
				<div class="flex gap-2">
					<div class="flex-1">
						<Input
							type="text"
							placeholder="/path/to/ebooks"
							bind:value={scanPath}
							onkeydown={(e: KeyboardEvent) => {
								if (e.key === 'Enter') handleScan();
							}}
							disabled={scanning}
						/>
					</div>
					<Button variant="outline" onclick={() => (pickerOpen = true)} disabled={scanning}>
						Browse
					</Button>
					<Button onclick={handleScan} disabled={!scanPath.trim() || scanning}>
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
							Scanning...
						{:else}
							Scan
						{/if}
					</Button>
				</div>
				<PathPicker bind:value={scanPath} bind:open={pickerOpen} mode="directory" />

				{#if scanError}
					<p class="mt-3 text-sm text-destructive">{scanError}</p>
				{/if}

				<!-- Scan manifest results -->
				{#if manifest}
					<div class="mt-4 rounded-lg border border-border p-4">
						<h3 class="text-sm font-semibold">Scan Results</h3>
						<div class="mt-2 grid grid-cols-2 gap-3 text-sm">
							<div>
								<span class="text-muted-foreground">Total files:</span>
								<span class="ml-1 font-medium">{manifest.total_files}</span>
							</div>
							<div>
								<span class="text-muted-foreground">Total size:</span>
								<span class="ml-1 font-medium">{formatFileSize(manifest.total_size)}</span>
							</div>
						</div>

						{#if manifest.formats.length > 0}
							<div class="mt-3">
								<span class="text-sm text-muted-foreground">Formats found:</span>
								<div class="mt-1.5 flex flex-wrap gap-2">
									{#each manifest.formats as fmt (fmt.format)}
										<span
											class="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-1 text-xs font-medium"
										>
											{fmt.format}
											<span class="text-muted-foreground">
												({fmt.count} file{fmt.count === 1 ? '' : 's'}, {formatFileSize(
													fmt.total_size
												)})
											</span>
										</span>
									{/each}
								</div>
							</div>
						{/if}

						<div class="mt-4">
							<Button onclick={handleStartImport} disabled={startingImport}>
								{#if startingImport}
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
									Starting import...
								{:else}
									Import {manifest.total_files} file{manifest.total_files === 1 ? '' : 's'}
								{/if}
							</Button>
						</div>
					</div>
				{/if}
			</CardContent>
		</Card>
	{/if}

	<!-- Active task progress -->
	{#if hasActiveTasks}
		<Card>
			<CardHeader>
				<CardTitle>Import Progress</CardTitle>
			</CardHeader>
			<CardContent class="space-y-3">
				{#each activeTaskIds as taskId (taskId)}
					{@const progress = taskProgressMap[taskId]}
					<div class="rounded-lg border border-border p-3">
						<div class="flex items-center justify-between">
							<span class="text-sm font-medium">
								Task {taskId.slice(0, 8)}...
							</span>
							<div class="flex items-center gap-2">
								{#if progress}
									<span class="text-xs {statusColorClass(progress.status)}">
										{taskStatusLabel(progress.status)}
									</span>
								{:else}
									<span class="text-xs text-muted-foreground">Connecting...</span>
								{/if}
								{#if progress?.status === 'completed' || progress?.status === 'failed'}
									<Button
										variant="ghost"
										size="icon-sm"
										class="size-6"
										onclick={() => dismissTask(taskId)}
										aria-label="Dismiss"
									>
										<svg
											class="size-3"
											xmlns="http://www.w3.org/2000/svg"
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
									</Button>
								{/if}
							</div>
						</div>

						<!-- Progress bar -->
						<div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
							<div
								class="h-full rounded-full transition-all duration-300
								{progress?.status === 'failed' ? 'bg-destructive' : 'bg-primary'}"
								style="width: {progress?.progress ?? 0}%"
							></div>
						</div>

						<div class="mt-1.5 flex items-center justify-between text-xs text-muted-foreground">
							<span>
								{#if progress?.message}
									{progress.message}
								{:else if progress?.status === 'completed'}
									Import complete
								{:else if progress?.status === 'failed'}
									{progress?.error ?? 'Import failed'}
								{:else}
									Waiting for progress...
								{/if}
							</span>
							<span>{progress?.progress ?? 0}%</span>
						</div>

						<!-- Error details -->
						{#if progress?.status === 'failed' && progress.error}
							<p class="mt-2 text-xs text-destructive">{progress.error}</p>
						{/if}

						<!-- Completion result -->
						{#if progress?.status === 'completed' && progress.result}
							<div class="mt-2 rounded bg-muted/50 p-2 text-xs">
								{#if progress.result.imported !== undefined}
									<!-- Directory import result summary -->
									<div class="flex flex-wrap gap-3">
										<span>
											<span class="font-semibold text-green-600 dark:text-green-400">{progress.result.imported}</span>
											<span class="text-muted-foreground"> imported</span>
										</span>
										{#if Number(progress.result.skipped) > 0}
											<span>
												<span class="font-semibold text-amber-600 dark:text-amber-400">{progress.result.skipped}</span>
												<span class="text-muted-foreground"> skipped</span>
											</span>
										{/if}
										{#if Number(progress.result.failed) > 0}
											<span>
												<span class="font-semibold text-red-600 dark:text-red-400">{progress.result.failed}</span>
												<span class="text-muted-foreground"> failed</span>
											</span>
										{/if}
									</div>
								{:else if progress.result.book_id}
									<!-- Single file import result -->
									<div class="flex items-center justify-between">
										<span class="text-muted-foreground">Book imported successfully</span>
										<a
											href="/books/{progress.result.book_id}"
											class="font-medium text-primary hover:underline"
										>
											View Book
										</a>
									</div>
								{:else}
									{#each Object.entries(progress.result) as [key, value] (key)}
										<div class="flex justify-between">
											<span class="text-muted-foreground">{key}:</span>
											<span class="font-medium">{value}</span>
										</div>
									{/each}
								{/if}
							</div>
							<div class="mt-2">
								<a
									href="/"
									class="inline-flex items-center gap-1.5 text-xs font-medium text-primary hover:underline"
								>
									<svg class="size-3.5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
										<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20" />
									</svg>
									View Library
								</a>
							</div>
						{/if}
					</div>
				{/each}
			</CardContent>
		</Card>
	{/if}

	<!-- Recent imports -->
	<Card>
		<CardHeader>
			<div class="flex items-center justify-between">
				<CardTitle>Recent Imports</CardTitle>
				<Button variant="ghost" size="sm" onclick={loadRecentTasks}>
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
						<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
						<path d="M21 3v5h-5" />
						<path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
						<path d="M8 16H3v5" />
					</svg>
				</Button>
			</div>
		</CardHeader>
		<CardContent>
			{#if loadingTasks}
				<div class="space-y-2">
					{#each [0, 1, 2] as i (i)}
						<div class="flex items-center gap-3 rounded-md border border-border p-3">
							<div class="h-4 w-20 animate-pulse rounded bg-muted"></div>
							<div class="h-4 w-16 animate-pulse rounded bg-muted"></div>
							<div class="h-4 flex-1 animate-pulse rounded bg-muted"></div>
						</div>
					{/each}
				</div>
			{:else if tasksError}
				<div class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-6">
					<div class="text-center">
						<p class="text-sm text-destructive">{tasksError}</p>
						<Button variant="outline" size="sm" class="mt-3" onclick={loadRecentTasks}>
							Retry
						</Button>
					</div>
				</div>
			{:else if recentTasks.length === 0}
				<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-6">
					<p class="text-sm text-muted-foreground">No import tasks yet.</p>
				</div>
			{:else}
				<div class="space-y-2">
					{#each recentTasks as task (task.id)}
						<div class="flex items-center gap-3 rounded-md border border-border px-3 py-2 text-sm">
							<span class="shrink-0 font-medium">
								{taskTypeLabel(task.task_type)}
							</span>
							<span class="shrink-0 {statusColorClass(task.status)}">
								{taskStatusLabel(task.status)}
							</span>
							{#if task.status === 'running'}
								<div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
									<div
										class="h-full rounded-full bg-primary transition-all duration-300"
										style="width: {task.progress}%"
									></div>
								</div>
								<span class="shrink-0 text-xs text-muted-foreground">{task.progress}%</span>
							{:else if task.message}
								<span class="flex-1 truncate text-muted-foreground">{task.message}</span>
							{:else if task.error_message}
								<span class="flex-1 truncate text-destructive">{task.error_message}</span>
							{:else}
								<span class="flex-1"></span>
							{/if}
							<span class="shrink-0 text-xs text-muted-foreground">
								{formatRelativeTime(task.created_at)}
							</span>
						</div>
					{/each}
				</div>
			{/if}
		</CardContent>
	</Card>
</div>
