<script lang="ts">
	import { untrack } from 'svelte';
	import { api, ApiError } from '$lib/api/index.js';
	import type {
		AuthorEntry,
		BookDetail,
		MetadataStatus,
		SeriesEntry,
		TagEntry,
		UpdateBookRequest
	} from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import * as Select from '$lib/components/ui/select/index.js';
	import AutocompleteInput from './AutocompleteInput.svelte';

	const metadataStatusOptions: { value: MetadataStatus; label: string }[] = [
		{ value: 'identified', label: 'Identified' },
		{ value: 'needs_review', label: 'Needs Review' },
		{ value: 'unidentified', label: 'Unidentified' }
	];

	interface PublisherInfo {
		id: string;
		name: string;
	}

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
			metadataStatus: b.metadata_status,
			authors: b.authors.map((a): AuthorEntry => ({ ...a })),
			tags: b.tags.map((t): TagEntry => ({ ...t })),
			series: b.series.map((s): SeriesEntry => ({ ...s })),
			publisher:
				b.publisher_id && b.publisher_name
					? ({ id: b.publisher_id, name: b.publisher_name } as PublisherInfo)
					: null
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
	let metadataStatus = $state<MetadataStatus>(snapshot.metadataStatus);
	let editAuthors = $state<AuthorEntry[]>(snapshot.authors);
	let editTags = $state<TagEntry[]>(snapshot.tags);
	let editSeries = $state<SeriesEntry[]>(snapshot.series);
	let editPublisher = $state<PublisherInfo | null>(snapshot.publisher);

	let saving = $state(false);
	let saveError = $state<string | null>(null);
	let swapping = $state(false);
	let swapError = $state<string | null>(null);
	let swapSnapshot = $state<{ title: string; firstAuthor: AuthorEntry } | null>(null);

	// --- Swap title/author ---

	async function resolveOrCreateAuthor(name: string): Promise<{ id: string; name: string; sort_name: string }> {
		const results = await api.authors.search(name);
		const match = results.items.find((a) => a.name.toLowerCase() === name.toLowerCase());
		if (match) return match;
		return await api.authors.create({ name });
	}

	async function handleSwapTitleAuthor() {
		if (editAuthors.length === 0) return;
		if (!title.trim()) {
			swapError = 'Title is empty — nothing to swap';
			return;
		}
		if (title.trim().toLowerCase() === editAuthors[0].name.toLowerCase()) return;

		swapping = true;
		swapError = null;

		const oldTitle = title;
		const oldFirstAuthor = { ...editAuthors[0] };

		try {
			const newAuthor = await resolveOrCreateAuthor(oldTitle);
			swapSnapshot = { title: oldTitle, firstAuthor: oldFirstAuthor };
			title = oldFirstAuthor.name;
			editAuthors = [
				{
					id: newAuthor.id,
					name: newAuthor.name,
					sort_name: newAuthor.sort_name,
					role: oldFirstAuthor.role,
					position: 0
				},
				...editAuthors.slice(1)
			];
		} catch (err) {
			swapError =
				err instanceof Error ? err.message : 'Failed to swap title and author';
			swapSnapshot = null;
		} finally {
			swapping = false;
		}
	}

	function handleUndoSwap() {
		if (!swapSnapshot) return;
		title = swapSnapshot.title;
		editAuthors = [swapSnapshot.firstAuthor, ...editAuthors.slice(1)];
		swapSnapshot = null;
		swapError = null;
	}

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

	// --- Publisher management ---

	function removePublisher() {
		editPublisher = null;
	}

	async function searchPublishers(q: string) {
		const result = await api.publishers.search(q);
		return result.items.map((p) => ({
			id: p.id,
			label: p.name
		}));
	}

	function selectPublisher(item: { id: string; label: string }) {
		editPublisher = { id: item.id, name: item.label };
	}

	async function createPublisher(name: string) {
		const publisher = await api.publishers.create({ name });
		editPublisher = { id: publisher.id, name: publisher.name };
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
		swapSnapshot = null;
		swapError = null;

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

		// Check if metadata status changed
		if (metadataStatus !== book.metadata_status) {
			updateData.metadata_status = metadataStatus;
		}

		// Check if publisher changed and include in scalar update
		const originalPublisherId = book.publisher_id ?? null;
		const newPublisherId = editPublisher?.id ?? null;
		const publisherChanged = newPublisherId !== originalPublisherId;
		if (publisherChanged) {
			updateData.publisher_id = newPublisherId;
		}

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

		// Check if series changed
		const seriesChanged =
			editSeries.length !== book.series.length ||
			editSeries.some(
				(s, i) =>
					s.id !== book.series[i]?.id ||
					s.position !== book.series[i]?.position
			);

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

			if (seriesChanged) {
				latestBook = await api.books.setSeries(book.id, {
					series: editSeries.map((s) => ({
						series_id: s.id,
						position: s.position
					}))
				});
			}

			// If nothing changed, just close
			if (!hasScalarChanges && !authorsChanged && !tagsChanged && !seriesChanged) {
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
		<div class="flex items-center justify-between">
			<Label for="edit-title">Title</Label>
			{#if swapSnapshot}
				<button
					type="button"
					class="inline-flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-xs text-muted-foreground hover:text-foreground hover:border-foreground/30 transition-colors"
					onclick={handleUndoSwap}
					title="Revert to original title and author"
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
						<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
						<path d="M3 3v5h5" />
					</svg>
					Undo Swap
				</button>
			{:else}
				<button
					type="button"
					class="inline-flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-xs text-muted-foreground hover:text-foreground hover:border-foreground/30 transition-colors disabled:opacity-40 disabled:pointer-events-none"
					onclick={handleSwapTitleAuthor}
					disabled={editAuthors.length === 0 || swapping}
					title="Swap title with first author's name"
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
						<path d="m16 3 4 4-4 4" />
						<path d="M20 7H4" />
						<path d="m8 21-4-4 4-4" />
						<path d="M4 17h16" />
					</svg>
					{swapping ? 'Swapping...' : 'Swap with Author'}
				</button>
			{/if}
		</div>
		<Input id="edit-title" type="text" bind:value={title} />
		{#if swapError}<p class="text-xs text-destructive">{swapError}</p>{/if}
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

	<!-- Metadata Status -->
	<div class="space-y-1.5">
		<Label>Metadata Status</Label>
		<Select.Root type="single" bind:value={metadataStatus}>
			<Select.Trigger class="w-full">
				{metadataStatusOptions.find((o) => o.value === metadataStatus)?.label ?? metadataStatus}
			</Select.Trigger>
			<Select.Content>
				{#each metadataStatusOptions as option (option.value)}
					<Select.Item value={option.value} label={option.label} />
				{/each}
			</Select.Content>
		</Select.Root>
	</div>

	<!-- Publisher -->
	<div class="space-y-1.5">
		<Label>Publisher</Label>
		{#if editPublisher}
			<div class="flex items-center gap-2 rounded border border-border px-2 py-1 text-sm">
				<span class="flex-1 font-medium">{editPublisher.name}</span>
				<button
					type="button"
					class="p-0.5 text-muted-foreground hover:text-destructive"
					onclick={removePublisher}
					aria-label="Remove publisher"
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
		{:else}
			<AutocompleteInput
				placeholder="Search publisher..."
				search={searchPublishers}
				onselect={selectPublisher}
				allowCreate
				oncreate={createPublisher}
			/>
		{/if}
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
