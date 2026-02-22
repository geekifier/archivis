<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { api, ApiError } from '$lib/api/index.js';
	import type { BookDetail } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
	import BookEditForm from '$lib/components/library/BookEditForm.svelte';
	import {
		placeholderHue,
		formatFileSize,
		formatIdentifierType,
		formatMetadataSource
	} from '$lib/utils.js';

	let book = $state<BookDetail | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
	let coverLoaded = $state(false);
	let coverError = $state(false);
	let editing = $state(false);
	let deleteDialogOpen = $state(false);
	let deleting = $state(false);
	let deleteError = $state<string | null>(null);

	const bookId = $derived(page.params.id ?? '');
	const hue = $derived(placeholderHue(bookId));
	const coverUrl = $derived(`/api/books/${bookId}/cover?size=lg`);

	const authors = $derived(book?.authors ?? []);
	const primaryAuthors = $derived(authors.filter((a) => a.role === 'author'));
	const otherContributors = $derived(authors.filter((a) => a.role !== 'author'));
	const authorDisplay = $derived(
		primaryAuthors.length > 0
			? primaryAuthors.map((a) => a.name).join(', ')
			: authors.map((a) => a.name).join(', ')
	);

	function fetchBook() {
		loading = true;
		error = null;
		notFound = false;
		coverLoaded = false;
		coverError = false;

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

	function seriesDisplay(series: { name: string; position: number | null }): string {
		if (series.position != null) {
			const pos = Number.isInteger(series.position)
				? series.position.toString()
				: series.position.toFixed(1);
			return `Book ${pos} in ${series.name}`;
		}
		return series.name;
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
		<div class="grid gap-8 md:grid-cols-[280px_1fr]">
			<!-- Left column: Cover -->
			<div>
				<div class="relative aspect-[2/3] w-full overflow-hidden rounded-lg bg-muted shadow-md">
					{#if book.has_cover && !coverError}
						{#if !coverLoaded}
							<div class="absolute inset-0 animate-pulse bg-muted"></div>
						{/if}
						<img
							src={coverUrl}
							alt="Cover of {book.title}"
							onload={() => (coverLoaded = true)}
							onerror={() => (coverError = true)}
							class="absolute inset-0 h-full w-full object-cover transition-opacity duration-200 {coverLoaded
								? 'opacity-100'
								: 'opacity-0'}"
						/>
					{:else}
						<div
							class="flex h-full w-full items-center justify-center p-6"
							style="background-color: hsl({hue}, 30%, 25%);"
						>
							<span class="line-clamp-6 text-center text-lg font-medium text-white/80">
								{book.title}
							</span>
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
					<BookEditForm {book} oncancel={cancelEdit} onsave={handleSave} />
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
						{#if authorDisplay}
							<p class="mt-1 text-lg text-muted-foreground">{authorDisplay}</p>
						{/if}
						<div class="mt-3 flex flex-wrap items-center gap-3">
							<span
								class="inline-flex rounded-full px-2.5 py-0.5 text-xs font-medium {statusClasses(book.metadata_status)}"
							>
								{statusLabel(book.metadata_status)}
							</span>
							<div class="flex items-center gap-2 text-xs text-muted-foreground">
								<span>{Math.round(book.metadata_confidence * 100)}% confidence</span>
								<div class="h-1.5 w-20 overflow-hidden rounded-full bg-muted">
									<div
										class="h-full rounded-full bg-primary transition-all"
										style="width: {book.metadata_confidence * 100}%"
									></div>
								</div>
							</div>
						</div>
					</div>

					<!-- Other contributors -->
					{#if otherContributors.length > 0}
						<div>
							<h3 class="text-sm font-semibold text-muted-foreground">Contributors</h3>
							<div class="mt-1 space-y-0.5">
								{#each otherContributors as contributor (contributor.id)}
									<p class="text-sm">
										{contributor.name}
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
									<p class="text-sm font-medium">{seriesDisplay(s)}</p>
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

					<!-- Identifiers -->
					{#if book.identifiers.length > 0}
						<div>
							<h3 class="text-sm font-semibold text-muted-foreground">Identifiers</h3>
							<div class="mt-2 overflow-x-auto">
								<table class="w-full text-sm">
									<thead>
										<tr class="border-b border-border text-left text-xs text-muted-foreground">
											<th class="pb-2 pr-4 font-medium">Type</th>
											<th class="pb-2 pr-4 font-medium">Value</th>
											<th class="pb-2 pr-4 font-medium">Source</th>
											<th class="pb-2 font-medium">Confidence</th>
										</tr>
									</thead>
									<tbody>
										{#each book.identifiers as ident (ident.id)}
											<tr class="border-b border-border/50">
												<td class="py-2 pr-4 font-medium">
													{formatIdentifierType(ident.identifier_type)}
												</td>
												<td class="py-2 pr-4 font-mono text-xs">{ident.value}</td>
												<td class="py-2 pr-4 text-muted-foreground">
													{formatMetadataSource(ident.source)}
												</td>
												<td class="py-2">{Math.round(ident.confidence * 100)}%</td>
											</tr>
										{/each}
									</tbody>
								</table>
							</div>
						</div>
					{/if}
				{/if}
			</div>
		</div>
	{/if}
</div>

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
