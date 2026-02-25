<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { ScanManifestResponse, TaskResponse } from '$lib/api/index.js';
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
	import ActiveTaskPanel from '$lib/components/tasks/ActiveTaskPanel.svelte';
	import TaskStatusBadge from '$lib/components/tasks/TaskStatusBadge.svelte';
	import {
		taskTypeLabel,
		formatRelativeTime,
		isTerminalStatus
	} from '$lib/components/tasks/task-utils.js';
	import { formatFileSize } from '$lib/utils.js';
	import { SvelteSet } from 'svelte/reactivity';

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

	// --- Recent tasks ---
	let recentTasks = $state<TaskResponse[]>([]);
	let loadingTasks = $state(true);
	let tasksError = $state<string | null>(null);

	// Exclude tasks already shown in the ActiveTaskPanel, sort running first
	const activeIdSet = $derived(new Set(activeTaskIds));
	const sortedRecentTasks = $derived(
		recentTasks
			.filter((t) => !activeIdSet.has(t.id))
			.sort((a, b) => {
				const termA = isTerminalStatus(a.status) ? 1 : 0;
				const termB = isTerminalStatus(b.status) ? 1 : 0;
				if (termA !== termB) return termA - termB;
				return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
			})
	);

	// Expandable recent task children
	let expandedTaskIds = new SvelteSet<string>();
	let taskChildrenMap = $state<Record<string, TaskResponse[]>>({});
	let loadingChildren = new SvelteSet<string>();

	// --- Load recent tasks on mount ---
	$effect(() => {
		loadRecentTasks();
	});

	async function loadRecentTasks() {
		loadingTasks = true;
		tasksError = null;
		try {
			recentTasks = await api.tasks.list();
			// Auto-detect running/pending tasks and add them to activeTaskIds
			// so the ActiveTaskPanel picks them up after navigation
			const runningIds = recentTasks
				.filter((t) => !isTerminalStatus(t.status))
				.map((t) => t.id);
			if (runningIds.length > 0) {
				const existing = new Set(activeTaskIds);
				const newIds = runningIds.filter((id) => !existing.has(id));
				if (newIds.length > 0) {
					activeTaskIds = [...activeTaskIds, ...newIds];
				}
			}
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
			activeTaskIds = [...activeTaskIds, ...taskIds];
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
			activeTaskIds = [...activeTaskIds, response.task_id];
		} catch (err) {
			scanError = err instanceof Error ? err.message : 'Failed to start import';
		} finally {
			startingImport = false;
		}
	}

	// --- Expand/collapse recent task children ---
	async function toggleChildren(taskId: string) {
		if (expandedTaskIds.has(taskId)) {
			expandedTaskIds.delete(taskId);
			return;
		}

		// Load children if not cached
		if (!taskChildrenMap[taskId]) {
			loadingChildren.add(taskId);
			try {
				const children = await api.tasks.children(taskId);
				taskChildrenMap = { ...taskChildrenMap, [taskId]: children };
			} catch {
				// Silently fail
			} finally {
				loadingChildren.delete(taskId);
			}
		}

		expandedTaskIds.add(taskId);
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

	<!-- Active task progress (using new component) -->
	<ActiveTaskPanel bind:taskIds={activeTaskIds} onAllDone={loadRecentTasks} />

	<!-- Recent Activity -->
	<Card>
		<CardHeader>
			<div class="flex items-center justify-between">
				<CardTitle>Recent Activity</CardTitle>
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
			{:else if sortedRecentTasks.length === 0}
				<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-6">
					<p class="text-sm text-muted-foreground">No tasks yet.</p>
				</div>
			{:else}
				<div class="space-y-2">
					{#each sortedRecentTasks as task (task.id)}
						<div>
							<div class="flex items-center gap-3 rounded-md border border-border px-3 py-2 text-sm">
								<span class="shrink-0 font-medium">
									{taskTypeLabel(task.task_type)}
								</span>
								<TaskStatusBadge status={task.status} />
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
								<!-- Children indicator -->
								{#if task.children_summary && task.children_summary.total > 0}
									<button
										class="shrink-0 text-xs text-muted-foreground hover:text-foreground"
										onclick={() => toggleChildren(task.id)}
										aria-label="Toggle subtasks"
									>
										<span class="inline-flex items-center gap-1">
											{task.children_summary.total} subtask{task.children_summary.total === 1 ? '' : 's'}
											<svg
												class="size-3 transition-transform {expandedTaskIds.has(task.id) ? 'rotate-180' : ''}"
												xmlns="http://www.w3.org/2000/svg"
												viewBox="0 0 24 24"
												fill="none"
												stroke="currentColor"
												stroke-width="2"
												stroke-linecap="round"
												stroke-linejoin="round"
											>
												<path d="m6 9 6 6 6-6" />
											</svg>
										</span>
									</button>
								{/if}
							</div>
							<!-- Expanded children -->
							{#if expandedTaskIds.has(task.id)}
								<div class="ml-4 mt-1 space-y-1 border-l-2 border-border pl-3">
									{#if loadingChildren.has(task.id)}
										<div class="py-2 text-xs text-muted-foreground">Loading subtasks...</div>
									{:else if taskChildrenMap[task.id]}
										{#each taskChildrenMap[task.id] as child (child.id)}
											<div class="flex items-center gap-2 rounded px-2 py-1 text-xs">
												<span class="shrink-0 font-medium">{taskTypeLabel(child.task_type)}</span>
												<TaskStatusBadge status={child.status} />
												{#if child.message}
													<span class="flex-1 truncate text-muted-foreground">{child.message}</span>
												{:else if child.error_message}
													<span class="flex-1 truncate text-destructive">{child.error_message}</span>
												{:else}
													<span class="flex-1"></span>
												{/if}
												<span class="shrink-0 text-muted-foreground">
													{formatRelativeTime(child.created_at)}
												</span>
											</div>
										{/each}
									{/if}
								</div>
							{/if}
						</div>
					{/each}
				</div>
			{/if}
		</CardContent>
	</Card>
</div>
