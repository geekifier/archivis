<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { api, ApiError } from '$lib/api/index.js';
	import type { BookDetail, CandidateResponse, DuplicateLinkResponse, TaskProgressEvent, TaskStatus } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import BookEditForm from '$lib/components/library/BookEditForm.svelte';
	import CoverUploadDialog from '$lib/components/library/CoverUploadDialog.svelte';
	import CandidateReview from '$lib/components/library/CandidateReview.svelte';
	import IdentifierEditor from '$lib/components/library/IdentifierEditor.svelte';
	import MergeDialog from '$lib/components/library/MergeDialog.svelte';
	import AutocompleteInput from '$lib/components/library/AutocompleteInput.svelte';
	import CoverImage from '$lib/components/library/CoverImage.svelte';
	import {
		placeholderHue,
		formatFileSize
	} from '$lib/utils.js';

	let book = $state<BookDetail | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
	let coverError = $state(false);
	let editing = $state(false);
	let deleteDialogOpen = $state(false);
	let deleting = $state(false);
	let deleteError = $state<string | null>(null);
	let coverCacheBust = $state(0);
	let coverDialogOpen = $state(false);
	let markingIdentified = $state(false);
	let markIdentifiedError = $state<string | null>(null);

	// --- Identification state ---
	let identifying = $state(false);
	let identifyError = $state<string | null>(null);
	let identifyProgress = $state<TaskProgressEvent | null>(null);
	let identifyEventSource: EventSource | null = null;
	let candidates = $state<CandidateResponse[]>([]);
	let candidatesError = $state<string | null>(null);
	let candidatesExpanded = $state(false);
	let candidatesLoaded = $state(false);

	// --- ISBN content scan state ---
	let scanning = $state(false);
	let scanError = $state<string | null>(null);
	let scanProgress = $state<TaskProgressEvent | null>(null);
	let scanEventSource: EventSource | null = null;

	// --- Duplicate state ---
	let duplicateLinks = $state<DuplicateLinkResponse[]>([]);
	let dismissingDupId = $state<string | null>(null);
	let mergeDialogOpen = $state(false);
	let selectedDupLink = $state<DuplicateLinkResponse | null>(null);
	let flagDialogOpen = $state(false);
	let flagging = $state(false);
	let flagError = $state<string | null>(null);

	const bookId = $derived(page.params.id ?? '');
	const hue = $derived(placeholderHue(bookId));
	const coverUrl = $derived(
		`/api/books/${bookId}/cover?size=lg${coverCacheBust ? `&t=${coverCacheBust}` : ''}`
	);

	const authors = $derived(book?.authors ?? []);
	const primaryAuthors = $derived(authors.filter((a) => a.role === 'author'));
	const otherContributors = $derived(authors.filter((a) => a.role !== 'author'));

	const canIdentify = $derived(
		book?.metadata_status === 'needs_review' || book?.metadata_status === 'unidentified'
	);

	const appliedCandidate = $derived(candidates.find((c) => c.status === 'applied'));


	function fetchBook() {
		loading = true;
		error = null;
		notFound = false;
		coverError = false;
		candidatesLoaded = false;

		api.books
			.get(bookId)
			.then((result) => {
				book = result;
			})
			.catch((err) => {
				if (err instanceof ApiError && err.isNotFound) {
					notFound = true;
				} else {
					error = err instanceof Error ? err.message : 'Failed to load book';
				}
			})
			.finally(() => {
				loading = false;
			});
	}

	$effect(() => {
		void bookId;
		fetchBook();
	});

	function enterEditMode() {
		editing = true;
	}

	function cancelEdit() {
		editing = false;
	}

	function handleSave(updated: BookDetail) {
		book = updated;
		editing = false;
	}

	async function handleDelete() {
		deleting = true;
		deleteError = null;
		try {
			await api.books.delete(bookId);
			deleteDialogOpen = false;
			goto('/');
		} catch (err) {
			deleteError = err instanceof Error ? err.message : 'Failed to delete book';
		} finally {
			deleting = false;
		}
	}

	function handleCoverUpdate(updated: BookDetail) {
		book = updated;
		coverError = false;
		coverCacheBust = Date.now();
	}

	async function handleMarkIdentified() {
		markingIdentified = true;
		markIdentifiedError = null;
		try {
			const updated = await api.books.update(bookId, { metadata_status: 'identified' });
			book = updated;
		} catch (err) {
			markIdentifiedError = err instanceof Error ? err.message : 'Failed to update status';
		} finally {
			markingIdentified = false;
		}
	}

	// --- Identification ---

	async function handleIdentify() {
		identifying = true;
		identifyError = null;
		identifyProgress = null;

		try {
			const response = await api.identify.book(bookId);
			subscribeToIdentifyProgress(response.task_id);
		} catch (err) {
			identifyError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to start identification';
			identifying = false;
		}
	}

	function subscribeToIdentifyProgress(taskId: string) {
		if (identifyEventSource) {
			identifyEventSource.close();
		}

		const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);
		identifyEventSource = es;

		es.addEventListener('task:progress', (event: MessageEvent) => {
			try {
				identifyProgress = JSON.parse(event.data) as TaskProgressEvent;
			} catch {
				// Ignore malformed events
			}
		});

		es.addEventListener('task:complete', (event: MessageEvent) => {
			try {
				const data = JSON.parse(event.data) as TaskProgressEvent;
				identifyProgress = { ...data, status: 'completed' as TaskStatus, progress: 100 };
			} catch {
				// Ignore malformed events
			}
			es.close();
			identifyEventSource = null;
			identifying = false;
			// Reload book and candidates
			fetchBook();
			loadCandidates();
		});

		es.addEventListener('task:error', (event: MessageEvent) => {
			try {
				const data = JSON.parse(event.data) as TaskProgressEvent;
				identifyProgress = { ...data, status: 'failed' as TaskStatus };
				identifyError = data.error ?? 'Identification failed';
			} catch {
				identifyError = 'Identification failed';
			}
			es.close();
			identifyEventSource = null;
			identifying = false;
		});

		es.onerror = () => {
			es.close();
			identifyEventSource = null;
			identifying = false;
		};
	}

	// --- ISBN Content Scan ---

	async function handleScanIsbn() {
		scanning = true;
		scanError = null;
		scanProgress = null;

		try {
			const response = await api.isbnScan.scanBook(bookId);
			subscribeToScanProgress(response.task_id);
		} catch (err) {
			scanError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to start ISBN scan';
			scanning = false;
		}
	}

	function subscribeToScanProgress(taskId: string) {
		if (scanEventSource) {
			scanEventSource.close();
		}

		const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);
		scanEventSource = es;

		es.addEventListener('task:progress', (event: MessageEvent) => {
			try {
				scanProgress = JSON.parse(event.data) as TaskProgressEvent;
			} catch {
				// Ignore malformed events
			}
		});

		es.addEventListener('task:complete', (event: MessageEvent) => {
			try {
				const data = JSON.parse(event.data) as TaskProgressEvent;
				scanProgress = { ...data, status: 'completed' as TaskStatus, progress: 100 };
			} catch {
				// Ignore malformed events
			}
			es.close();
			scanEventSource = null;
			scanning = false;
			// Reload book to pick up newly found identifiers
			fetchBook();
		});

		es.addEventListener('task:error', (event: MessageEvent) => {
			try {
				const data = JSON.parse(event.data) as TaskProgressEvent;
				scanProgress = { ...data, status: 'failed' as TaskStatus };
				scanError = data.error ?? 'ISBN scan failed';
			} catch {
				scanError = 'ISBN scan failed';
			}
			es.close();
			scanEventSource = null;
			scanning = false;
		});

		es.onerror = () => {
			es.close();
			scanEventSource = null;
			scanning = false;
		};
	}

	async function loadCandidates() {
		candidatesError = null;
		try {
			candidates = await api.identify.candidates(bookId);
			if (candidates.length > 0 && !candidatesLoaded) {
				candidatesExpanded = book?.metadata_status === 'needs_review';
			}
			candidatesLoaded = true;
		} catch (err) {
			candidatesError =
				err instanceof Error ? err.message : 'Failed to load candidates';
		}
	}

	function handleCandidateApplied(updated: BookDetail) {
		book = updated;
		coverError = false;
		coverCacheBust = Date.now();
		loadCandidates();
	}

	function handleCandidateRejected(candidateId: string) {
		candidates = candidates.map((c) =>
			c.id === candidateId ? { ...c, status: 'rejected' as const } : c
		);
	}

	function handleCandidateUndone(updated: BookDetail) {
		book = updated;
		loadCandidates();
	}

	function handleIdentifierUpdate(updated: BookDetail) {
		book = updated;
	}

	// --- Duplicate functions ---

	async function loadDuplicateLinks() {
		try {
			duplicateLinks = await api.duplicates.forBook(bookId);
		} catch {
			// Silently ignore — duplicates are supplemental info
			duplicateLinks = [];
		}
	}

	async function handleDismissDuplicate(linkId: string) {
		dismissingDupId = linkId;
		try {
			await api.duplicates.dismiss(linkId);
			duplicateLinks = duplicateLinks.filter((l) => l.id !== linkId);
		} catch {
			// Silently ignore
		} finally {
			dismissingDupId = null;
		}
	}

	function openDupMergeDialog(link: DuplicateLinkResponse) {
		selectedDupLink = link;
		mergeDialogOpen = true;
	}

	function handleDupMergeComplete(merged: BookDetail) {
		mergeDialogOpen = false;
		selectedDupLink = null;
		goto(`/books/${merged.id}`);
	}

	function handleDupMergeCancel() {
		mergeDialogOpen = false;
		selectedDupLink = null;
	}

	function otherBookInLink(link: DuplicateLinkResponse): { id: string; title: string } {
		if (link.book_a.id === bookId) {
			return { id: link.book_b.id, title: link.book_b.title };
		}
		return { id: link.book_a.id, title: link.book_a.title };
	}

	async function handleFlagDuplicate(item: { id: string; label: string }) {
		flagging = true;
		flagError = null;
		flagDialogOpen = false;
		try {
			const newLink = await api.duplicates.flag(bookId, item.id);
			duplicateLinks = [...duplicateLinks, newLink];
		} catch (err) {
			flagError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to flag duplicate';
		} finally {
			flagging = false;
		}
	}

	async function searchBooksForFlag(query: string): Promise<{ id: string; label: string; sublabel?: string }[]> {
		try {
			const result = await api.books.list({ q: query, per_page: 10 });
			return result.items
				.filter((b) => b.id !== bookId)
				.map((b) => ({
					id: b.id,
					label: b.title,
					sublabel: b.authors?.map((a) => a.name).join(', ')
				}));
		} catch {
			return [];
		}
	}

	// Load candidates and duplicate links when book loads
	$effect(() => {
		if (book && !loading) {
			loadCandidates();
			loadDuplicateLinks();
		}

		return () => {
			// Cleanup SSE on unmount
			if (identifyEventSource) {
				identifyEventSource.close();
				identifyEventSource = null;
			}
			if (scanEventSource) {
				scanEventSource.close();
				scanEventSource = null;
			}
		};
	});

	function statusLabel(status: string): string {
		switch (status) {
			case 'identified':
				return 'Identified';
			case 'needs_review':
				return 'Needs Review';
			case 'unidentified':
				return 'Unidentified';
			default:
				return status;
		}
	}

	function statusClasses(status: string): string {
		switch (status) {
			case 'identified':
				return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400';
			case 'needs_review':
				return 'bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400';
			case 'unidentified':
				return 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400';
			default:
				return 'bg-muted text-muted-foreground';
		}
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString(undefined, {
			year: 'numeric',
			month: 'long',
			day: 'numeric'
		});
	}

	function formatPubDate(date: string): string {
		// publication_date is "YYYY-MM-DD"
		const parts = date.split('-');
		if (parts.length === 3) {
			const d = new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2]));
			return d.toLocaleDateString(undefined, {
				year: 'numeric',
				month: 'long',
				day: 'numeric'
			});
		}
		return date;
	}

	function formatRating(rating: number): string {
		return `${rating.toFixed(1)} / 5`;
	}

	function formatFormatBadge(format: string): string {
		return format.toUpperCase();
	}
</script>

<div class="mx-auto max-w-5xl space-y-6">
	<!-- Back navigation -->
	<a
		href="/"
		class="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
	>
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
			<path d="m15 18-6-6 6-6" />
		</svg>
		Back to Library
	</a>

	{#if loading}
		<!-- Loading skeleton -->
		<div class="grid gap-8 md:grid-cols-[280px_1fr]">
			<div class="aspect-[2/3] w-full animate-pulse rounded-lg bg-muted"></div>
			<div class="space-y-4">
				<div class="h-8 w-3/4 animate-pulse rounded bg-muted"></div>
				<div class="h-5 w-1/3 animate-pulse rounded bg-muted"></div>
				<div class="h-4 w-1/4 animate-pulse rounded bg-muted"></div>
				<div class="mt-6 space-y-2">
					<div class="h-4 w-full animate-pulse rounded bg-muted"></div>
					<div class="h-4 w-full animate-pulse rounded bg-muted"></div>
					<div class="h-4 w-2/3 animate-pulse rounded bg-muted"></div>
				</div>
			</div>
		</div>
	{:else if notFound}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
		>
			<div class="text-center">
				<p class="text-lg font-medium text-destructive">Book not found</p>
				<p class="mt-1 text-sm text-muted-foreground">
					The book you're looking for doesn't exist or has been removed.
				</p>
				<Button variant="outline" class="mt-4" href="/">Back to Library</Button>
			</div>
		</div>
	{:else if error}
		<div
			class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
		>
			<div class="text-center">
				<p class="text-destructive">{error}</p>
				<Button variant="outline" class="mt-4" onclick={fetchBook}>Retry</Button>
			</div>
		</div>
	{:else if book}
		<!-- Duplicate alert banners -->
		{#if duplicateLinks.length > 0}
			<div class="space-y-2">
				{#each duplicateLinks as dupLink (dupLink.id)}
					{@const other = otherBookInLink(dupLink)}
					<div
						class="flex items-center justify-between rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 dark:border-amber-800 dark:bg-amber-900/20"
					>
						<div class="flex items-center gap-2 text-sm">
							<svg
								class="size-4 flex-shrink-0 text-amber-600 dark:text-amber-400"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
							>
								<path
									d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"
								/>
								<line x1="12" x2="12" y1="9" y2="13" />
								<line x1="12" x2="12.01" y1="17" y2="17" />
							</svg>
							<span class="text-amber-800 dark:text-amber-300">
								This book may be a duplicate of
								<a
									href="/books/{other.id}"
									class="font-medium underline transition-colors hover:text-amber-900 dark:hover:text-amber-200"
								>
									{other.title}
								</a>
							</span>
						</div>
						<div class="flex items-center gap-2">
							<Button
								size="sm"
								variant="outline"
								class="h-7 px-2 text-xs"
								disabled={dismissingDupId === dupLink.id}
								onclick={() => handleDismissDuplicate(dupLink.id)}
							>
								{#if dismissingDupId === dupLink.id}
									Dismissing...
								{:else}
									Dismiss
								{/if}
							</Button>
							<Button
								size="sm"
								class="h-7 px-2 text-xs"
								onclick={() => openDupMergeDialog(dupLink)}
							>
								Review
							</Button>
						</div>
					</div>
				{/each}
			</div>
		{/if}

		{#if flagError}
			<div
				class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
			>
				{flagError}
			</div>
		{/if}

		<div class="grid gap-8 md:grid-cols-[280px_1fr]">
			<!-- Left column: Cover -->
			<div>
				<div class="group relative aspect-[2/3] w-full overflow-hidden rounded-lg bg-muted shadow-md">
					{#if book.has_cover && !coverError}
						<CoverImage src={coverUrl} alt="Cover of {book.title}" fadeIn={true} onerror={() => (coverError = true)} />
						<div
							class="absolute inset-x-0 bottom-0 flex items-center justify-center bg-black/60 p-2 opacity-0 transition-opacity group-hover:opacity-100"
						>
							<button
								type="button"
								class="inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-white/20"
								onclick={() => (coverDialogOpen = true)}
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
									<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
									<polyline points="17 8 12 3 7 8" />
									<line x1="12" x2="12" y1="3" y2="15" />
								</svg>
								Change Cover
							</button>
						</div>
					{:else}
						<div
							class="flex h-full w-full flex-col items-center justify-center gap-3 p-6"
							style="background-color: hsl({hue}, 30%, 25%);"
						>
							<span class="line-clamp-6 text-center text-lg font-medium text-white/80">
								{book.title}
							</span>
							<button
								type="button"
								class="inline-flex items-center gap-1.5 rounded-md border border-white/30 px-3 py-1.5 text-xs font-medium text-white/90 transition-colors hover:bg-white/20"
								onclick={() => (coverDialogOpen = true)}
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
									<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
									<polyline points="17 8 12 3 7 8" />
									<line x1="12" x2="12" y1="3" y2="15" />
								</svg>
								Add Cover
							</button>
						</div>
					{/if}
				</div>
				<!-- Files section (below cover on desktop) -->
				{#if book.files.length > 0}
					<div class="mt-4 space-y-2">
						<h3 class="text-sm font-semibold text-muted-foreground">Files</h3>
						{#each book.files as file (file.id)}
							<a
								href="/api/books/{book.id}/files/{file.id}/download"
								class="flex items-center justify-between rounded-md border border-border p-2.5 text-sm transition-colors hover:bg-muted"
							>
								<div class="flex items-center gap-2">
									<span
										class="inline-flex rounded bg-primary/10 px-1.5 py-0.5 text-xs font-semibold text-primary"
									>
										{formatFormatBadge(file.format)}
									</span>
									<span class="text-muted-foreground">{formatFileSize(file.file_size)}</span>
								</div>
								<svg
									class="size-4 text-muted-foreground"
									xmlns="http://www.w3.org/2000/svg"
									viewBox="0 0 24 24"
									fill="none"
									stroke="currentColor"
									stroke-width="2"
									stroke-linecap="round"
									stroke-linejoin="round"
								>
									<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
									<polyline points="7 10 12 15 17 10" />
									<line x1="12" x2="12" y1="15" y2="3" />
								</svg>
							</a>
						{/each}
					</div>
				{/if}
			</div>

			<!-- Right column: Metadata or Edit Form -->
			<div class="space-y-6">
				{#if editing}
					<BookEditForm {book} oncancel={cancelEdit} onsave={handleSave} oncoverupdate={handleCoverUpdate} />
				{:else}
					<!-- Header: Title, status badge, confidence, edit button -->
					<div>
						<div class="flex items-start justify-between gap-4">
							<h1 class="text-2xl font-bold tracking-tight md:text-3xl">{book.title}</h1>
							<div class="flex items-center gap-2">
								<Button size="sm" variant="outline" onclick={enterEditMode}>
									<svg
										class="size-4"
										viewBox="0 0 24 24"
										fill="none"
										stroke="currentColor"
										stroke-width="2"
										stroke-linecap="round"
										stroke-linejoin="round"
									>
										<path
											d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"
										/>
										<path d="m15 5 4 4" />
									</svg>
									Edit
								</Button>
								<Button
									size="sm"
									variant="outline"
									onclick={() => (flagDialogOpen = !flagDialogOpen)}
									disabled={flagging}
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
										<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z" />
										<line x1="4" x2="4" y1="22" y2="15" />
									</svg>
									Flag Duplicate
								</Button>
								<Button
									size="sm"
									variant="destructive"
									onclick={() => (deleteDialogOpen = true)}
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
										<path d="M3 6h18" />
										<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
										<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
										<line x1="10" x2="10" y1="11" y2="17" />
										<line x1="14" x2="14" y1="11" y2="17" />
									</svg>
									Delete
								</Button>
							</div>
						</div>
						{#if primaryAuthors.length > 0}
							<p class="mt-1 text-lg text-muted-foreground">
								{#each primaryAuthors as a, i (a.id)}<a
										href="/authors/{a.id}"
										class="transition-colors hover:text-foreground hover:underline"
									>{a.name}</a>{#if i < primaryAuthors.length - 1},&nbsp;{/if}{/each}
							</p>
						{:else if authors.length > 0}
							<p class="mt-1 text-lg text-muted-foreground">
								{#each authors as a, i (a.id)}<a
										href="/authors/{a.id}"
										class="transition-colors hover:text-foreground hover:underline"
									>{a.name}</a>{#if i < authors.length - 1},&nbsp;{/if}{/each}
							</p>
						{/if}
						<div class="mt-3 flex flex-wrap items-center gap-3">
							<span
								class="inline-flex rounded-full px-2.5 py-0.5 text-xs font-medium {statusClasses(book.metadata_status)}"
							>
								{statusLabel(book.metadata_status)}
							</span>
							{#if canIdentify}
								<Button
									size="sm"
									onclick={handleIdentify}
									disabled={identifying || markingIdentified}
									class="h-6 px-2 text-xs"
								>
									{#if identifying}
										<svg
											class="size-3 animate-spin"
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
										Identifying...
									{:else}
										<svg
											class="size-3"
											viewBox="0 0 24 24"
											fill="none"
											stroke="currentColor"
											stroke-width="2"
											stroke-linecap="round"
											stroke-linejoin="round"
										>
											<circle cx="11" cy="11" r="8" />
											<path d="m21 21-4.3-4.3" />
										</svg>
										Identify
									{/if}
								</Button>
								<Button
									size="sm"
									variant="outline"
									onclick={handleMarkIdentified}
									disabled={markingIdentified || identifying}
									class="h-6 px-2 text-xs"
								>
									{#if markingIdentified}
										Updating...
									{:else}
										Mark as Identified
									{/if}
								</Button>
							{/if}
							{#if identifyError}
								<span class="text-xs text-destructive">{identifyError}</span>
							{/if}
							{#if markIdentifiedError}
								<span class="text-xs text-destructive">{markIdentifiedError}</span>
							{/if}
							<div class="flex items-center gap-2 text-xs text-muted-foreground">
								<span>{Math.round(book.metadata_confidence * 100)}% confidence</span>
								<div class="h-1.5 w-20 overflow-hidden rounded-full bg-muted">
									<div
										class="h-full rounded-full bg-primary transition-all"
										style="width: {book.metadata_confidence * 100}%"
									></div>
								</div>
							</div>
							{#if appliedCandidate}
								<span class="text-xs text-muted-foreground">
									via {appliedCandidate.provider_name}
								</span>
							{/if}
							{#if candidates.length > 0 && !editing}
								<button
									type="button"
									class="inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
									onclick={() => (candidatesExpanded = !candidatesExpanded)}
								>
									<svg
										class="size-3.5 transition-transform duration-150 {candidatesExpanded ? 'rotate-90' : ''}"
										viewBox="0 0 24 24"
										fill="none"
										stroke="currentColor"
										stroke-width="2"
										stroke-linecap="round"
										stroke-linejoin="round"
									>
										<path d="m9 18 6-6-6-6" />
									</svg>
									{candidates.length}
									{candidates.length === 1 ? 'candidate' : 'candidates'}
								</button>
							{/if}
						</div>
					</div>

					<!-- Identification progress -->
					{#if identifying && identifyProgress}
						<div class="rounded-lg border border-border p-3">
							<div class="flex items-center justify-between text-sm">
								<span class="font-medium">Identifying book...</span>
								<span class="text-xs text-muted-foreground">{identifyProgress.progress}%</span>
							</div>
							<div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
								<div
									class="h-full rounded-full bg-primary transition-all duration-300"
									style="width: {identifyProgress.progress}%"
								></div>
							</div>
							{#if identifyProgress.message}
								<p class="mt-1.5 text-xs text-muted-foreground">{identifyProgress.message}</p>
							{/if}
						</div>
					{/if}

					<!-- Candidates review (collapsible) -->
					{#if candidatesExpanded && candidates.length > 0 && !editing}
						<CandidateReview
							{book}
							{candidates}
							onapply={handleCandidateApplied}
							onreject={handleCandidateRejected}
							onundo={handleCandidateUndone}
						/>
					{/if}

					{#if candidatesError}
						<p class="text-sm text-destructive">{candidatesError}</p>
					{/if}

					<!-- Other contributors -->
					{#if otherContributors.length > 0}
						<div>
							<h3 class="text-sm font-semibold text-muted-foreground">Contributors</h3>
							<div class="mt-1 space-y-0.5">
								{#each otherContributors as contributor (contributor.id)}
									<p class="text-sm">
										<a
											href="/authors/{contributor.id}"
											class="transition-colors hover:text-primary hover:underline"
										>{contributor.name}</a>
										<span class="text-muted-foreground">({contributor.role})</span>
									</p>
								{/each}
							</div>
						</div>
					{/if}

					<!-- Description -->
					{#if book.description}
						<div>
							<h3 class="text-sm font-semibold text-muted-foreground">Description</h3>
							<p class="mt-1 whitespace-pre-line text-sm leading-relaxed">
								{book.description}
							</p>
						</div>
					{/if}

					<!-- Details grid -->
					<div>
						<h3 class="text-sm font-semibold text-muted-foreground">Details</h3>
						<dl class="mt-2 grid grid-cols-2 gap-x-6 gap-y-3 text-sm">
							{#if book.publisher_name}
								<div>
									<dt class="text-muted-foreground">Publisher</dt>
									<dd class="font-medium">{book.publisher_name}</dd>
								</div>
							{/if}
							{#if book.publication_date}
								<div>
									<dt class="text-muted-foreground">Published</dt>
									<dd class="font-medium">{formatPubDate(book.publication_date)}</dd>
								</div>
							{/if}
							{#if book.language}
								<div>
									<dt class="text-muted-foreground">Language</dt>
									<dd class="font-medium">{book.language}</dd>
								</div>
							{/if}
							{#if book.page_count != null}
								<div>
									<dt class="text-muted-foreground">Pages</dt>
									<dd class="font-medium">{book.page_count}</dd>
								</div>
							{/if}
							{#if book.rating != null}
								<div>
									<dt class="text-muted-foreground">Rating</dt>
									<dd class="font-medium">{formatRating(book.rating)}</dd>
								</div>
							{/if}
							<div>
								<dt class="text-muted-foreground">Added</dt>
								<dd class="font-medium">{formatDate(book.added_at)}</dd>
							</div>
						</dl>
					</div>

					<!-- Series -->
					{#if book.series.length > 0}
						<div>
							<h3 class="text-sm font-semibold text-muted-foreground">Series</h3>
							<div class="mt-1 space-y-1">
								{#each book.series as s (s.id)}
									<p class="text-sm font-medium">
										{#if s.position != null}
											Book {Number.isInteger(s.position) ? s.position.toString() : s.position.toFixed(1)} in
										{/if}
										<a
											href="/series/{s.id}"
											class="transition-colors hover:text-primary hover:underline"
										>{s.name}</a>
									</p>
								{/each}
							</div>
						</div>
					{/if}

					<!-- Tags -->
					{#if book.tags.length > 0}
						<div>
							<h3 class="text-sm font-semibold text-muted-foreground">Tags</h3>
							<div class="mt-1.5 flex flex-wrap gap-1.5">
								{#each book.tags as tag (tag.id)}
									<span
										class="inline-flex rounded-full border border-border bg-muted px-2.5 py-0.5 text-xs font-medium"
									>
										{#if tag.category}
											<span class="mr-1 text-muted-foreground">{tag.category}:</span>
										{/if}
										{tag.name}
									</span>
								{/each}
							</div>
						</div>
					{/if}

					<!-- Identifiers (editable) + ISBN scan -->
					<div class="space-y-4">
						<div class="flex items-center justify-between">
							<h3 class="text-sm font-semibold text-muted-foreground">Identifiers</h3>
							<Button
								size="sm"
								variant="outline"
								onclick={handleScanIsbn}
								disabled={scanning || identifying}
								class="h-7 px-2 text-xs"
							>
								{#if scanning}
									<svg
										class="size-3 animate-spin"
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
									<svg
										class="size-3"
										viewBox="0 0 24 24"
										fill="none"
										stroke="currentColor"
										stroke-width="2"
										stroke-linecap="round"
										stroke-linejoin="round"
									>
										<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
										<path d="M14 2v4a2 2 0 0 0 2 2h4" />
										<circle cx="11.5" cy="14.5" r="2.5" />
										<path d="M13.3 16.3 15 18" />
									</svg>
									Scan for ISBNs
								{/if}
							</Button>
						</div>

						{#if scanning && scanProgress}
							<div class="rounded-lg border border-border p-3">
								<div class="flex items-center justify-between text-sm">
									<span class="font-medium">Scanning book content...</span>
									<span class="text-xs text-muted-foreground">{scanProgress.progress}%</span>
								</div>
								<div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
									<div
										class="h-full rounded-full bg-primary transition-all duration-300"
										style="width: {scanProgress.progress}%"
									></div>
								</div>
								{#if scanProgress.message}
									<p class="mt-1.5 text-xs text-muted-foreground">{scanProgress.message}</p>
								{/if}
							</div>
						{/if}

						{#if scanError}
							<div class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive">
								{scanError}
							</div>
						{/if}
					</div>

					<IdentifierEditor {book} onupdate={handleIdentifierUpdate} />
				{/if}
			</div>
		</div>
	{/if}
</div>

<!-- Cover upload dialog -->
{#if book}
	<CoverUploadDialog
		bookId={book.id}
		hasCover={book.has_cover}
		bind:open={coverDialogOpen}
		onupdate={handleCoverUpdate}
	/>
{/if}

<!-- Delete confirmation dialog -->
{#if book}
	<AlertDialog.Root bind:open={deleteDialogOpen}>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title>Delete Book</AlertDialog.Title>
				<AlertDialog.Description>
					Are you sure you want to delete <strong>{book.title}</strong>? This will permanently
					remove the book, all associated files, and cover images. This action cannot be undone.
				</AlertDialog.Description>
			</AlertDialog.Header>
			{#if deleteError}
				<p class="text-sm text-destructive">{deleteError}</p>
			{/if}
			<AlertDialog.Footer>
				<AlertDialog.Cancel disabled={deleting}>Cancel</AlertDialog.Cancel>
				<AlertDialog.Action
					class="bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90"
					onclick={handleDelete}
					disabled={deleting}
				>
					{#if deleting}
						Deleting...
					{:else}
						Delete
					{/if}
				</AlertDialog.Action>
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Root>
{/if}

<!-- Flag as duplicate dialog -->
{#if book}
	<Dialog.Root bind:open={flagDialogOpen}>
		<Dialog.Content class="sm:max-w-md">
			<Dialog.Header>
				<Dialog.Title>Flag as Duplicate</Dialog.Title>
				<Dialog.Description>
					Search for the other book that is a duplicate of <strong>{book.title}</strong>.
				</Dialog.Description>
			</Dialog.Header>
			<div class="py-2">
				<AutocompleteInput
					placeholder="Search for a book..."
					search={searchBooksForFlag}
					onselect={handleFlagDuplicate}
				/>
			</div>
		</Dialog.Content>
	</Dialog.Root>
{/if}

<!-- Merge dialog for duplicate review -->
{#if selectedDupLink}
	<MergeDialog
		link={selectedDupLink}
		bind:open={mergeDialogOpen}
		onmerge={handleDupMergeComplete}
		oncancel={handleDupMergeCancel}
	/>
{/if}
