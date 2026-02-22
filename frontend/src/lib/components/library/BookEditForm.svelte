<script lang="ts">
	import { untrack } from 'svelte';
	import { api, ApiError } from '$lib/api/index.js';
	import type {
		AuthorEntry,
		BookDetail,
		SeriesEntry,
		TagEntry,
		UpdateBookRequest
	} from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import AutocompleteInput from './AutocompleteInput.svelte';

	interface Props {
		book: BookDetail;
		oncancel: () => void;
		onsave: (updated: BookDetail) => void;
	}

	let { book, oncancel, onsave }: Props = $props();

	// Snapshot initial values from the book prop for the edit form.
	// We intentionally capture initial values (not reactive) for editing.
	function snapshotBook(b: BookDetail) {
		return {
			title: b.title,
			description: b.description ?? '',
			language: b.language ?? '',
			publicationDate: b.publication_date ?? '',
			rating: b.rating != null ? String(b.rating) : '',
			pageCount: b.page_count != null ? String(b.page_count) : '',
			authors: b.authors.map((a): AuthorEntry => ({ ...a })),
			tags: b.tags.map((t): TagEntry => ({ ...t })),
			series: b.series.map((s): SeriesEntry => ({ ...s }))
		};
	}
	const snapshot = untrack(() => snapshotBook(book));

	// --- Editable state ---
	let title = $state(snapshot.title);
	let description = $state(snapshot.description);
	let language = $state(snapshot.language);
	let publicationDate = $state(snapshot.publicationDate);
	let rating = $state(snapshot.rating);
	let pageCount = $state(snapshot.pageCount);
	let editAuthors = $state<AuthorEntry[]>(snapshot.authors);
	let editTags = $state<TagEntry[]>(snapshot.tags);
	let editSeries = $state<SeriesEntry[]>(snapshot.series);

	let saving = $state(false);
	let saveError = $state<string | null>(null);

	// --- Author management ---

	function removeAuthor(index: number) {
		editAuthors = editAuthors.filter((_, i) => i !== index);
	}

	function moveAuthor(index: number, direction: -1 | 1) {
		const newIndex = index + direction;
		if (newIndex < 0 || newIndex >= editAuthors.length) return;
		const arr = [...editAuthors];
		[arr[index], arr[newIndex]] = [arr[newIndex], arr[index]];
		editAuthors = arr;
	}

	async function searchAuthors(q: string) {
		const result = await api.authors.search(q);
		// Filter out authors already added
		const existingIds = new Set(editAuthors.map((a) => a.id));
		return result.items
			.filter((a) => !existingIds.has(a.id))
			.map((a) => ({
				id: a.id,
				label: a.name,
				sublabel: a.sort_name !== a.name ? a.sort_name : undefined
			}));
	}

	function addAuthor(item: { id: string; label: string }) {
		editAuthors = [
			...editAuthors,
			{
				id: item.id,
				name: item.label,
				sort_name: item.label,
				role: 'author',
				position: editAuthors.length
			}
		];
	}

	// --- Tag management ---

	function removeTag(index: number) {
		editTags = editTags.filter((_, i) => i !== index);
	}

	async function searchTags(q: string) {
		const result = await api.tags.search(q);
		const existingIds = new Set(editTags.map((t) => t.id));
		return result.items
			.filter((t) => !existingIds.has(t.id))
			.map((t) => ({
				id: t.id,
				label: t.name,
				sublabel: t.category ?? undefined
			}));
	}

	function addTag(item: { id: string; label: string }) {
		editTags = [
			...editTags,
			{
				id: item.id,
				name: item.label,
				category: null
			}
		];
	}

	function createTag(name: string) {
		// Use a temporary ID for new tags; the backend will create them
		editTags = [
			...editTags,
			{
				id: `new:${name}`,
				name,
				category: null
			}
		];
	}

	// --- Series management ---

	function removeSeries(index: number) {
		editSeries = editSeries.filter((_, i) => i !== index);
	}

	async function searchSeries(q: string) {
		const result = await api.series.search(q);
		const existingIds = new Set(editSeries.map((s) => s.id));
		return result.items
			.filter((s) => !existingIds.has(s.id))
			.map((s) => ({
				id: s.id,
				label: s.name,
				sublabel: s.description ?? undefined
			}));
	}

	function addSeries(item: { id: string; label: string }) {
		editSeries = [
			...editSeries,
			{
				id: item.id,
				name: item.label,
				description: null,
				position: null
			}
		];
	}

	function setSeriesPosition(index: number, value: string) {
		const pos = value === '' ? null : Number(value);
		editSeries = editSeries.map((s, i) => (i === index ? { ...s, position: pos } : s));
	}

	// --- Save ---

	async function handleSave() {
		saving = true;
		saveError = null;

		// Snapshot the original book for rollback
		const originalBook = book;

		// Build the scalar update request — only include changed fields
		const updateData: UpdateBookRequest = {};
		if (title !== book.title) updateData.title = title;
		const descVal = description || null;
		if (descVal !== book.description) updateData.description = description || undefined;
		const langVal = language || null;
		if (langVal !== book.language) updateData.language = language || undefined;
		const pubVal = publicationDate || null;
		if (pubVal !== book.publication_date) updateData.publication_date = publicationDate || undefined;
		const ratingVal = rating === '' ? null : Number(rating);
		if (ratingVal !== book.rating) updateData.rating = ratingVal ?? undefined;
		const pageVal = pageCount === '' ? null : Number(pageCount);
		if (pageVal !== book.page_count) updateData.page_count = pageVal ?? undefined;

		const hasScalarChanges = Object.keys(updateData).length > 0;

		// Check if authors changed
		const authorsChanged =
			editAuthors.length !== book.authors.length ||
			editAuthors.some(
				(a, i) =>
					a.id !== book.authors[i]?.id ||
					a.role !== book.authors[i]?.role ||
					a.position !== i
			);

		// Check if tags changed
		const tagsChanged =
			editTags.length !== book.tags.length ||
			editTags.some((t, i) => t.id !== book.tags[i]?.id);

		try {
			let latestBook = originalBook;

			if (hasScalarChanges) {
				latestBook = await api.books.update(book.id, updateData);
			}

			if (authorsChanged) {
				latestBook = await api.books.setAuthors(book.id, {
					authors: editAuthors.map((a, i) => ({
						author_id: a.id,
						role: a.role,
						position: i
					}))
				});
			}

			if (tagsChanged) {
				latestBook = await api.books.setTags(book.id, {
					tags: editTags.map((t) => {
						if (t.id.startsWith('new:')) {
							return { name: t.name, category: t.category ?? undefined };
						}
						return { tag_id: t.id };
					})
				});
			}

			// If nothing changed, just close
			if (!hasScalarChanges && !authorsChanged && !tagsChanged) {
				oncancel();
				return;
			}

			onsave(latestBook);
		} catch (err) {
			saveError =
				err instanceof ApiError
					? err.message
					: err instanceof Error
						? err.message
						: 'Failed to save changes';
			// Rollback: restore original book in parent
			onsave(originalBook);
		} finally {
			saving = false;
		}
	}
</script>

<div class="space-y-5">
	<!-- Action bar -->
	<div class="flex items-center gap-2">
		<Button size="sm" onclick={handleSave} disabled={saving}>
			{saving ? 'Saving...' : 'Save'}
		</Button>
		<Button size="sm" variant="outline" onclick={oncancel} disabled={saving}>Cancel</Button>
		{#if saveError}
			<span class="text-sm text-destructive">{saveError}</span>
		{/if}
	</div>

	<!-- Title -->
	<div class="space-y-1.5">
		<Label for="edit-title">Title</Label>
		<Input id="edit-title" type="text" bind:value={title} />
	</div>

	<!-- Description -->
	<div class="space-y-1.5">
		<Label for="edit-description">Description</Label>
		<textarea
			id="edit-description"
			bind:value={description}
			rows="4"
			class="border-input bg-background ring-offset-background placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 flex w-full rounded-md border px-3 py-2 text-sm shadow-xs outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50"
		></textarea>
	</div>

	<!-- Authors -->
	<div class="space-y-1.5">
		<Label>Authors</Label>
		{#if editAuthors.length > 0}
			<div class="space-y-1">
				{#each editAuthors as author, i (author.id + '-' + i)}
					<div class="flex items-center gap-2 rounded border border-border px-2 py-1 text-sm">
						<span class="flex-1 font-medium">{author.name}</span>
						<span class="text-xs text-muted-foreground">{author.role}</span>
						<button
							type="button"
							class="p-0.5 text-muted-foreground hover:text-foreground disabled:opacity-30"
							disabled={i === 0}
							onclick={() => moveAuthor(i, -1)}
							aria-label="Move up"
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
								<path d="m18 15-6-6-6 6" />
							</svg>
						</button>
						<button
							type="button"
							class="p-0.5 text-muted-foreground hover:text-foreground disabled:opacity-30"
							disabled={i === editAuthors.length - 1}
							onclick={() => moveAuthor(i, 1)}
							aria-label="Move down"
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
								<path d="m6 9 6 6 6-6" />
							</svg>
						</button>
						<button
							type="button"
							class="p-0.5 text-muted-foreground hover:text-destructive"
							onclick={() => removeAuthor(i)}
							aria-label="Remove author"
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
								<path d="M18 6 6 18" />
								<path d="m6 6 12 12" />
							</svg>
						</button>
					</div>
				{/each}
			</div>
		{/if}
		<AutocompleteInput
			placeholder="Add author..."
			search={searchAuthors}
			onselect={addAuthor}
		/>
	</div>

	<!-- Series -->
	<div class="space-y-1.5">
		<Label>Series</Label>
		{#if editSeries.length > 0}
			<div class="space-y-1">
				{#each editSeries as s, i (s.id)}
					<div class="flex items-center gap-2 rounded border border-border px-2 py-1 text-sm">
						<span class="flex-1 font-medium">{s.name}</span>
						<div class="flex items-center gap-1">
							<span class="text-xs text-muted-foreground">Position:</span>
							<input
								type="number"
								value={s.position ?? ''}
								oninput={(e) =>
									setSeriesPosition(i, (e.target as HTMLInputElement).value)}
								class="border-input bg-background h-6 w-16 rounded border px-1 text-center text-xs"
								placeholder="#"
							/>
						</div>
						<button
							type="button"
							class="p-0.5 text-muted-foreground hover:text-destructive"
							onclick={() => removeSeries(i)}
							aria-label="Remove series"
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
								<path d="M18 6 6 18" />
								<path d="m6 6 12 12" />
							</svg>
						</button>
					</div>
				{/each}
			</div>
		{/if}
		<AutocompleteInput
			placeholder="Add series..."
			search={searchSeries}
			onselect={addSeries}
		/>
	</div>

	<!-- Tags -->
	<div class="space-y-1.5">
		<Label>Tags</Label>
		{#if editTags.length > 0}
			<div class="flex flex-wrap gap-1.5">
				{#each editTags as tag, i (tag.id)}
					<span
						class="inline-flex items-center gap-1 rounded-full border border-border bg-muted px-2.5 py-0.5 text-xs font-medium"
					>
						{#if tag.category}
							<span class="text-muted-foreground">{tag.category}:</span>
						{/if}
						{tag.name}
						<button
							type="button"
							class="ml-0.5 text-muted-foreground hover:text-destructive"
							onclick={() => removeTag(i)}
							aria-label="Remove tag"
						>
							<svg
								class="size-3"
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
						</button>
					</span>
				{/each}
			</div>
		{/if}
		<AutocompleteInput
			placeholder="Add tag..."
			search={searchTags}
			onselect={addTag}
			allowCreate
			oncreate={createTag}
		/>
	</div>

	<!-- Rating -->
	<div class="grid grid-cols-2 gap-4">
		<div class="space-y-1.5">
			<Label for="edit-rating">Rating (0-5)</Label>
			<Input
				id="edit-rating"
				type="number"
				min="0"
				max="5"
				step="0.5"
				bind:value={rating}
				placeholder="0.0"
			/>
		</div>
		<div class="space-y-1.5">
			<Label for="edit-page-count">Pages</Label>
			<Input
				id="edit-page-count"
				type="number"
				min="0"
				bind:value={pageCount}
				placeholder="0"
			/>
		</div>
	</div>

	<!-- Language + Publication Date -->
	<div class="grid grid-cols-2 gap-4">
		<div class="space-y-1.5">
			<Label for="edit-language">Language</Label>
			<Input id="edit-language" type="text" bind:value={language} placeholder="en" />
		</div>
		<div class="space-y-1.5">
			<Label for="edit-pub-date">Publication Date</Label>
			<Input id="edit-pub-date" type="date" bind:value={publicationDate} />
		</div>
	</div>
</div>
