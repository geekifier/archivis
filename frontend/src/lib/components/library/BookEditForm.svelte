<script lang="ts">
  import { untrack } from 'svelte';
  import { api, ApiError } from '$lib/api/index.js';
  import type {
    AuthorEntry,
    BookDetail,
    MetadataField,
    MetadataProvenance,
    SeriesEntry,
    TagEntry,
    UpdateBookRequest
  } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import AutocompleteInput from './AutocompleteInput.svelte';
  import CoverUploadDialog from './CoverUploadDialog.svelte';
  import FieldProtectionToggle from './FieldProtectionToggle.svelte';
  import LanguageCombobox from './LanguageCombobox.svelte';

  interface PublisherInfo {
    id: string;
    name: string;
  }

  interface Props {
    book: BookDetail;
    oncancel: () => void;
    onsave: (updated: BookDetail) => void;
    oncoverupdate?: (updated: BookDetail) => void;
    metadata_provenance?: MetadataProvenance;
    onprotectionchange?: (updated: BookDetail) => void;
  }

  let { book, oncancel, onsave, oncoverupdate, metadata_provenance, onprotectionchange }: Props =
    $props();

  function isFieldProtected(field: MetadataField): boolean {
    return metadata_provenance?.[field]?.protected ?? false;
  }

  let protectionPending = $state<MetadataField | null>(null);

  async function handleProtectionToggle(field: MetadataField, newProtected: boolean) {
    protectionPending = field;
    try {
      const updated = newProtected
        ? await api.books.protectFields(book.id, [field])
        : await api.books.unprotectFields(book.id, [field]);
      onprotectionchange?.(updated);
    } catch {
      // Silently ignore — the toggle will reflect server state on next render
    } finally {
      protectionPending = null;
    }
  }

  let coverDialogOpen = $state(false);

  // Snapshot initial values from the book prop for the edit form.
  // We intentionally capture initial values (not reactive) for editing.
  function snapshotBook(b: BookDetail) {
    return {
      trusted: b.metadata_user_trusted,
      title: b.title,
      subtitle: b.subtitle ?? '',
      description: b.description ?? '',
      language: b.language ?? '',
      publicationYear: b.publication_year != null ? String(b.publication_year) : '',
      rating: b.rating != null ? String(b.rating) : '',
      pageCount: b.page_count != null ? String(b.page_count) : '',
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
  let editTrusted = $state(snapshot.trusted);
  let title = $state(snapshot.title);
  let subtitle = $state(snapshot.subtitle);
  let description = $state(snapshot.description);
  let language = $state(snapshot.language);
  let publicationYear = $state(snapshot.publicationYear);
  let rating = $state(snapshot.rating);
  let pageCount = $state(snapshot.pageCount);
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

  async function resolveOrCreateAuthor(
    name: string
  ): Promise<{ id: string; name: string; sort_name: string }> {
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
      swapError = err instanceof Error ? err.message : 'Failed to swap title and author';
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
    const subVal = subtitle || null;
    if (subVal !== book.subtitle) updateData.subtitle = subtitle || undefined;
    const descVal = description || null;
    if (descVal !== book.description) updateData.description = description || undefined;
    const langVal = language || null;
    if (langVal !== book.language) updateData.language = language || undefined;
    const pubVal = publicationYear === '' ? null : Number(publicationYear);
    if (pubVal !== book.publication_year)
      updateData.publication_year = pubVal ?? undefined;
    const ratingVal = rating === '' ? null : Number(rating);
    if (ratingVal !== book.rating) updateData.rating = ratingVal ?? undefined;
    const pageVal = pageCount === '' ? null : Number(pageCount);
    if (pageVal !== book.page_count) updateData.page_count = pageVal ?? undefined;

    // Check if publisher changed and include in scalar update
    const originalPublisherId = book.publisher_id ?? null;
    const newPublisherId = editPublisher?.id ?? null;
    const publisherChanged = newPublisherId !== originalPublisherId;
    if (publisherChanged) {
      updateData.publisher_id = newPublisherId;
    }

    // Include trust change if it differs from current state
    if (editTrusted !== book.metadata_user_trusted) {
      updateData.metadata_user_trusted = editTrusted;
    }

    const hasScalarChanges = Object.keys(updateData).length > 0;

    // Check if authors changed
    const authorsChanged =
      editAuthors.length !== book.authors.length ||
      editAuthors.some(
        (a, i) =>
          a.id !== book.authors[i]?.id || a.role !== book.authors[i]?.role || a.position !== i
      );

    // Check if tags changed
    const tagsChanged =
      editTags.length !== book.tags.length || editTags.some((t, i) => t.id !== book.tags[i]?.id);

    // Check if series changed
    const seriesChanged =
      editSeries.length !== book.series.length ||
      editSeries.some(
        (s, i) => s.id !== book.series[i]?.id || s.position !== book.series[i]?.position
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
      if (err instanceof ApiError && err.status === 409) {
        saveError = 'A metadata refresh started. Save again after it completes.';
      } else {
        saveError =
          err instanceof ApiError
            ? err.message
            : err instanceof Error
              ? err.message
              : 'Failed to save changes';
      }
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

  <!-- Trust toggle -->
  <div class="flex items-center gap-3 rounded-lg border px-3 py-2 {editTrusted ? 'border-emerald-500/40 bg-emerald-500/5' : 'border-border bg-muted/30'}">
    <button
      type="button"
      class="inline-flex items-center gap-1.5 text-sm font-medium transition-colors {editTrusted ? 'text-emerald-600 dark:text-emerald-400' : 'text-muted-foreground hover:text-foreground'}"
      disabled={book.resolution_state === 'running'}
      title={book.resolution_state === 'running' ? 'Wait for metadata refresh to complete' : editTrusted ? 'Click to remove trust' : 'Click to trust this metadata'}
      onclick={() => (editTrusted = !editTrusted)}
    >
      <svg
        class="size-4"
        viewBox="0 0 24 24"
        fill={editTrusted ? 'currentColor' : 'none'}
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z" />
        {#if editTrusted}
          <path d="m9 12 2 2 4-4" stroke="white" fill="none" />
        {/if}
      </svg>
      {editTrusted ? 'Metadata trusted' : 'Trust this metadata'}
    </button>
    {#if editTrusted !== book.metadata_user_trusted}
      <span class="text-xs text-amber-600 dark:text-amber-400">(unsaved)</span>
    {/if}
  </div>

  <div class="rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
    <p>
      Locking a field prevents automated metadata refreshes from overwriting it. Saved fields are
      automatically locked. Use the lock toggles to manage protection per field.
    </p>
  </div>

  <!-- Cover Image -->
  <div class="space-y-1.5">
    <div class="flex items-center justify-between">
      <Label>Cover Image</Label>
      <Button
        size="sm"
        variant="outline"
        class="h-7 px-2 text-xs"
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
        {book.has_cover ? 'Change Cover' : 'Add Cover'}
      </Button>
    </div>
  </div>

  <!-- Title -->
  <div class="space-y-1.5">
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-1">
        <Label for="edit-title">Title</Label>
        <FieldProtectionToggle
          field="title"
          protected={isFieldProtected('title')}
          disabled={protectionPending === 'title'}
          ontoggle={handleProtectionToggle}
        />
      </div>
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

  <!-- Subtitle -->
  <div class="space-y-1.5">
    <div class="flex items-center gap-1">
      <Label for="edit-subtitle">Subtitle</Label>
      <FieldProtectionToggle
        field="subtitle"
        protected={isFieldProtected('subtitle')}
        disabled={protectionPending === 'subtitle'}
        ontoggle={handleProtectionToggle}
      />
    </div>
    <Input id="edit-subtitle" type="text" bind:value={subtitle} />
  </div>

  <!-- Authors -->
  <div class="space-y-1.5">
    <div class="flex items-center gap-1">
      <Label>Authors</Label>
      <FieldProtectionToggle
        field="authors"
        protected={isFieldProtected('authors')}
        disabled={protectionPending === 'authors'}
        ontoggle={handleProtectionToggle}
      />
    </div>
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
    <AutocompleteInput placeholder="Add author..." search={searchAuthors} onselect={addAuthor} />
  </div>

  <!-- Description -->
  <div class="space-y-1.5">
    <div class="flex items-center gap-1">
      <Label for="edit-description">Description</Label>
      <FieldProtectionToggle
        field="description"
        protected={isFieldProtected('description')}
        disabled={protectionPending === 'description'}
        ontoggle={handleProtectionToggle}
      />
    </div>
    <textarea
      id="edit-description"
      bind:value={description}
      rows="4"
      class="border-input bg-background ring-offset-background placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 flex w-full rounded-md border px-3 py-2 text-sm shadow-xs outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50"
    ></textarea>
  </div>

  <!-- Publisher -->
  <div class="space-y-1.5">
    <div class="flex items-center gap-1">
      <Label>Publisher</Label>
      <FieldProtectionToggle
        field="publisher"
        protected={isFieldProtected('publisher')}
        disabled={protectionPending === 'publisher'}
        ontoggle={handleProtectionToggle}
      />
    </div>
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
    <div class="flex items-center gap-1">
      <Label>Series</Label>
      <FieldProtectionToggle
        field="series"
        protected={isFieldProtected('series')}
        disabled={protectionPending === 'series'}
        ontoggle={handleProtectionToggle}
      />
    </div>
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
                oninput={(e) => setSeriesPosition(i, (e.target as HTMLInputElement).value)}
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
    <AutocompleteInput placeholder="Add series..." search={searchSeries} onselect={addSeries} />
  </div>

  <!-- Cover protection (standalone toggle, no field to edit) -->
  <div class="space-y-1.5">
    <div class="flex items-center gap-1">
      <Label>Cover Protection</Label>
      <FieldProtectionToggle
        field="cover"
        protected={isFieldProtected('cover')}
        disabled={protectionPending === 'cover'}
        ontoggle={handleProtectionToggle}
      />
    </div>
    <p class="text-xs text-muted-foreground">
      Toggle to prevent metadata refreshes from replacing the cover image.
    </p>
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
      <div class="flex items-center gap-1">
        <Label for="edit-rating">Rating (0-5)</Label>
      </div>
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
      <div class="flex items-center gap-1">
        <Label for="edit-page-count">Pages</Label>
        <FieldProtectionToggle
          field="page_count"
          protected={isFieldProtected('page_count')}
          disabled={protectionPending === 'page_count'}
          ontoggle={handleProtectionToggle}
        />
      </div>
      <Input id="edit-page-count" type="number" min="0" bind:value={pageCount} placeholder="0" />
    </div>
  </div>

  <!-- Language + Publication Date -->
  <div class="grid grid-cols-2 gap-4">
    <div class="space-y-1.5">
      <div class="flex items-center gap-1">
        <Label for="edit-language">Language</Label>
        <FieldProtectionToggle
          field="language"
          protected={isFieldProtected('language')}
          disabled={protectionPending === 'language'}
          ontoggle={handleProtectionToggle}
        />
      </div>
      <LanguageCombobox id="edit-language" bind:value={language} onchange={(code) => (language = code)} />
    </div>
    <div class="space-y-1.5">
      <div class="flex items-center gap-1">
        <Label for="edit-pub-year">Publication Year</Label>
        <FieldProtectionToggle
          field="publication_year"
          protected={isFieldProtected('publication_year')}
          disabled={protectionPending === 'publication_year'}
          ontoggle={handleProtectionToggle}
        />
      </div>
      <Input id="edit-pub-year" type="number" min="1000" max="2100" bind:value={publicationYear} placeholder="e.g. 2024" />
    </div>
  </div>
</div>

<CoverUploadDialog
  bookId={book.id}
  hasCover={book.has_cover}
  bind:open={coverDialogOpen}
  onupdate={(updated) => oncoverupdate?.(updated)}
/>
