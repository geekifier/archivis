<script lang="ts">
  import { api, ApiError } from '$lib/api/index.js';
  import type { BookDetail, DuplicateLinkResponse, MergeRequest } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
  import * as Select from '$lib/components/ui/select/index.js';
  import CoverImage from './CoverImage.svelte';
  import {
    placeholderHue,
    formatFileSize,
    formatIdentifierType,
    formatFormatLabel
  } from '$lib/utils.js';

  interface Props {
    link: DuplicateLinkResponse;
    open: boolean;
    onmerge: (merged: BookDetail) => void;
    oncancel: () => void;
  }

  let { link, open = $bindable(), onmerge, oncancel }: Props = $props();

  // Full detail for both books
  let bookA = $state<BookDetail | null>(null);
  let bookB = $state<BookDetail | null>(null);
  let detailLoading = $state(true);
  let detailError = $state<string | null>(null);

  // Merge options
  let primaryId = $state<string>('');
  type MergePreference = NonNullable<MergeRequest['prefer_metadata_from']>;

  let metadataPreference = $state<MergePreference>('primary');

  // Merge action state
  let merging = $state(false);
  let mergeError = $state<string | null>(null);
  let confirmOpen = $state(false);

  const metadataPreferenceOptions = [
    { value: 'primary', label: 'Keep primary metadata' },
    { value: 'secondary', label: 'Prefer secondary metadata' },
    { value: 'higher_ingest_quality', label: 'Use higher ingest quality' }
  ];

  const primaryBook = $derived(primaryId === bookA?.id ? bookA : bookB);
  const secondaryBook = $derived(primaryId === bookA?.id ? bookB : bookA);

  // Load full book details when dialog opens
  $effect(() => {
    if (open && link) {
      detailLoading = true;
      detailError = null;
      primaryId = link.book_a.id;

      Promise.all([api.books.get(link.book_a.id), api.books.get(link.book_b.id)])
        .then(([a, b]) => {
          bookA = a;
          bookB = b;
        })
        .catch((err) => {
          detailError = err instanceof Error ? err.message : 'Failed to load book details';
        })
        .finally(() => {
          detailLoading = false;
        });
    }
  });

  function handleConfirmMerge() {
    confirmOpen = true;
  }

  async function handleMerge() {
    if (!primaryBook || !secondaryBook) return;
    merging = true;
    mergeError = null;
    confirmOpen = false;

    try {
      const merged = await api.duplicates.merge(link.id, {
        primary_id: primaryBook.id,
        secondary_id: secondaryBook.id,
        prefer_metadata_from: metadataPreference
      });
      onmerge(merged);
    } catch (err) {
      mergeError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to merge books';
    } finally {
      merging = false;
    }
  }

  function isDifferent(valA: string | null | undefined, valB: string | null | undefined): boolean {
    const a = valA ?? '';
    const b = valB ?? '';
    return a !== b;
  }

  function diffClass(valA: string | null | undefined, valB: string | null | undefined): string {
    return isDifferent(valA, valB) ? 'border-l-2 border-l-amber-400 pl-2' : '';
  }

  function formatResolutionLabel(book: BookDetail): string {
    const raw = book.resolution_outcome ?? book.resolution_state;
    return raw
      .split('_')
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(' ');
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="max-w-5xl max-h-[90vh] overflow-y-auto">
    <Dialog.Header>
      <Dialog.Title>Merge Duplicate Books</Dialog.Title>
      <Dialog.Description>
        Compare both books side by side. Choose which book to keep as the primary, then merge to
        combine all files and metadata.
      </Dialog.Description>
    </Dialog.Header>

    {#if detailLoading}
      <div class="flex items-center justify-center py-12">
        <div class="text-center">
          <svg
            class="mx-auto size-6 animate-spin text-muted-foreground"
            xmlns="http://www.w3.org/2000/svg"
            fill="none"
            viewBox="0 0 24 24"
          >
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"
            ></circle>
            <path
              class="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
            ></path>
          </svg>
          <p class="mt-2 text-sm text-muted-foreground">Loading book details...</p>
        </div>
      </div>
    {:else if detailError}
      <div
        class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {detailError}
      </div>
    {:else if bookA && bookB}
      <!-- Merge options -->
      <div
        class="mb-4 flex flex-wrap items-center gap-4 rounded-lg border border-border bg-muted/30 p-3"
      >
        <div class="flex items-center gap-2 text-sm">
          <span class="font-medium text-muted-foreground">Metadata preference:</span>
          <Select.Root type="single" bind:value={metadataPreference}>
            <Select.Trigger class="h-8 w-52 text-xs">
              {metadataPreferenceOptions.find((o) => o.value === metadataPreference)?.label ??
                metadataPreference}
            </Select.Trigger>
            <Select.Content>
              {#each metadataPreferenceOptions as option (option.value)}
                <Select.Item value={option.value} label={option.label} />
              {/each}
            </Select.Content>
          </Select.Root>
        </div>
      </div>

      <!-- Side-by-side comparison -->
      <div class="grid gap-4 md:grid-cols-2">
        <!-- Book A column -->
        {@render bookColumn(bookA, bookB, 'left')}

        <!-- Book B column -->
        {@render bookColumn(bookB, bookA, 'right')}
      </div>

      {#if mergeError}
        <div
          class="mt-4 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        >
          {mergeError}
        </div>
      {/if}

      <Dialog.Footer class="mt-4">
        <Button variant="outline" onclick={oncancel} disabled={merging}>Cancel</Button>
        <Button onclick={handleConfirmMerge} disabled={merging || !primaryBook || !secondaryBook}>
          {#if merging}
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
            Merging...
          {:else}
            Merge Books
          {/if}
        </Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>

<!-- Merge confirmation dialog -->
<AlertDialog.Root bind:open={confirmOpen}>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>Confirm Merge</AlertDialog.Title>
      <AlertDialog.Description>
        {#if secondaryBook}
          The catalog entry for <strong>{secondaryBook.title}</strong> will be merged into
          <strong>{primaryBook?.title}</strong>. All files, identifiers, authors, series, and tags
          from both books will be combined under the primary entry. No files will be deleted. This
          action cannot be undone.
        {/if}
      </AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel disabled={merging}>Cancel</AlertDialog.Cancel>
      <AlertDialog.Action onclick={handleMerge} disabled={merging}>
        {#if merging}
          Merging...
        {:else}
          Merge
        {/if}
      </AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>

{#snippet bookColumn(book: BookDetail, otherBook: BookDetail, position: 'left' | 'right')}
  {@const isPrimary = primaryId === book.id}
  {@const facesRight = position === 'left'}
  {@const hasIncoming = isPrimary && otherBook.files.length > 0}
  {@const hasOutgoing = !isPrimary && book.files.length > 0}
  {@const isExtending = hasIncoming || hasOutgoing}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="cursor-pointer overflow-visible rounded-lg border p-4 transition-colors {isPrimary
      ? 'relative z-10 border-sky-500/40 bg-sky-500/[0.03] dark:border-sky-400/40 dark:bg-sky-400/[0.03]'
      : 'border-transparent hover:border-border/50'} {isExtending
      ? facesRight
        ? 'md:rounded-r-none md:border-r-0'
        : 'md:rounded-l-none md:border-l-0'
      : ''}"
    onclick={() => (primaryId = book.id)}
  >
    <!-- Primary selection -->
    <div class="mb-3 flex items-center justify-between">
      <label class="flex items-center gap-2 text-sm">
        <input
          type="radio"
          name="primary-book"
          value={book.id}
          checked={primaryId === book.id}
          onchange={() => (primaryId = book.id)}
          class="h-4 w-4 accent-primary"
        />
        <span class="font-medium">
          {#if primaryId === book.id}
            Primary
          {:else}
            Secondary
          {/if}
        </span>
      </label>
      {#if primaryId === book.id}
        <span
          class="inline-flex rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-800 dark:bg-green-900/30 dark:text-green-400"
        >
          Keep
        </span>
      {:else}
        <span
          class="inline-flex rounded-full bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-800 dark:bg-blue-900/30 dark:text-blue-400"
        >
          Merge
        </span>
      {/if}
    </div>

    <!-- Cover -->
    <div class="mb-3 flex justify-center">
      <div class="relative h-40 w-28 overflow-hidden rounded-md bg-muted shadow-sm">
        {#if book.has_cover}
          <CoverImage src="/api/books/{book.id}/cover?size=md" alt="Cover of {book.title}" />
        {:else}
          <div
            class="flex h-full w-full items-center justify-center p-2"
            style="background-color: hsl({placeholderHue(book.id)}, 30%, 25%);"
          >
            <span class="line-clamp-4 text-center text-xs font-medium text-white/80">
              {book.title}
            </span>
          </div>
        {/if}
      </div>
    </div>

    <!-- Metadata fields -->
    <div class="space-y-2 text-sm">
      <div class={diffClass(book.title, otherBook.title)}>
        <dt class="text-xs font-medium text-muted-foreground">Title</dt>
        <dd class="font-medium">{book.title}</dd>
      </div>

      <div
        class={diffClass(
          book.authors.map((a) => a.name).join(', '),
          otherBook.authors.map((a) => a.name).join(', ')
        )}
      >
        <dt class="text-xs font-medium text-muted-foreground">Authors</dt>
        <dd>
          {#if book.authors.length > 0}
            {book.authors.map((a) => a.name).join(', ')}
          {:else}
            <span class="text-muted-foreground">--</span>
          {/if}
        </dd>
      </div>

      <div class={diffClass(book.publisher_name, otherBook.publisher_name)}>
        <dt class="text-xs font-medium text-muted-foreground">Publisher</dt>
        <dd>{book.publisher_name ?? '--'}</dd>
      </div>

      <div class={diffClass(String(book.publication_year ?? ''), String(otherBook.publication_year ?? ''))}>
        <dt class="text-xs font-medium text-muted-foreground">Published</dt>
        <dd>{book.publication_year ?? '--'}</dd>
      </div>

      <div class={diffClass(book.language, otherBook.language)}>
        <dt class="text-xs font-medium text-muted-foreground">Language</dt>
        <dd>{book.language_label ?? book.language ?? '--'}</dd>
      </div>

      {#if book.series.length > 0 || otherBook.series.length > 0}
        <div
          class={diffClass(
            book.series.map((s) => s.name).join(', '),
            otherBook.series.map((s) => s.name).join(', ')
          )}
        >
          <dt class="text-xs font-medium text-muted-foreground">Series</dt>
          <dd>
            {#if book.series.length > 0}
              {book.series
                .map((s) => (s.position != null ? `${s.name} #${s.position}` : s.name))
                .join(', ')}
            {:else}
              <span class="text-muted-foreground">--</span>
            {/if}
          </dd>
        </div>
      {/if}

      {#if book.tags.length > 0 || otherBook.tags.length > 0}
        <div>
          <dt class="text-xs font-medium text-muted-foreground">Tags</dt>
          <dd>
            {#if book.tags.length > 0}
              <div class="mt-0.5 flex flex-wrap gap-1">
                {#each book.tags as tag (tag.id)}
                  <span
                    class="inline-flex rounded-full border border-border bg-muted px-2 py-0.5 text-xs"
                  >
                    {tag.name}
                  </span>
                {/each}
              </div>
            {:else}
              <span class="text-muted-foreground">--</span>
            {/if}
          </dd>
        </div>
      {/if}

      {#if book.identifiers.length > 0 || otherBook.identifiers.length > 0}
        <div>
          <dt class="text-xs font-medium text-muted-foreground">Identifiers</dt>
          <dd>
            {#if book.identifiers.length > 0}
              <div class="mt-0.5 space-y-0.5">
                {#each book.identifiers as ident (ident.id)}
                  <div class="flex items-center gap-1.5 text-xs">
                    <span class="font-medium text-muted-foreground">
                      {formatIdentifierType(ident.identifier_type)}:
                    </span>
                    <span class="font-mono">{ident.value}</span>
                  </div>
                {/each}
              </div>
            {:else}
              <span class="text-muted-foreground">--</span>
            {/if}
          </dd>
        </div>
      {/if}

      <!-- Files — puzzle-piece visualization -->
      {#if book.files.length > 0 || otherBook.files.length > 0}
        <div>
          <dd>
            {#if isPrimary}
              <!-- Primary: unified puzzle block — extends across gap to meet secondary -->
              <div class="mt-1 text-xs">
                <div
                  class="rounded-lg border border-sky-500/40 bg-sky-500/15 px-2 py-1.5 dark:border-sky-400/40 dark:bg-sky-400/15 {hasIncoming
                    ? facesRight
                      ? 'md:mr-[-2rem] md:rounded-tr-none'
                      : 'md:ml-[-2rem] md:rounded-tl-none'
                    : ''}"
                >
                  <div class="space-y-1">
                    {#each book.files as file (file.id)}
                      <div class="flex items-center gap-1.5">
                        <span
                          class="inline-flex rounded bg-sky-600/20 px-1 py-0.5 text-[10px] font-semibold text-sky-700 dark:bg-sky-400/20 dark:text-sky-300"
                        >
                          {formatFormatLabel(file.format, file.format_version)}
                        </span>
                        <span class="text-muted-foreground">
                          {formatFileSize(file.file_size)}
                        </span>
                      </div>
                    {/each}
                  </div>
                  {#if hasIncoming}
                    <div class="mt-1.5 border-t border-sky-500/30 pt-1.5 dark:border-sky-400/30">
                      <div class="space-y-1">
                        {#each otherBook.files as file (file.id)}
                          <div class="flex items-center gap-1.5 opacity-50">
                            <span
                              class="inline-flex rounded bg-sky-600/20 px-1 py-0.5 text-[10px] font-semibold text-sky-700 dark:bg-sky-400/20 dark:text-sky-300"
                            >
                              {formatFormatLabel(file.format, file.format_version)}
                            </span>
                            <span class="text-muted-foreground">
                              {formatFileSize(file.file_size)}
                            </span>
                          </div>
                        {/each}
                      </div>
                    </div>
                  {/if}
                </div>
              </div>
            {:else}
              <!-- Secondary: files being merged away, extends toward primary -->
              {#if book.files.length > 0}
                <div
                  class="mt-1 rounded-lg border border-sky-500/40 bg-sky-500/15 px-2 py-1.5 text-xs dark:border-sky-400/40 dark:bg-sky-400/15 {facesRight
                    ? 'md:mr-[-1rem] md:border-r-0 md:rounded-r-none'
                    : 'md:ml-[-1rem] md:border-l-0 md:rounded-l-none'}"
                >
                  <div class="space-y-1">
                    {#each book.files as file (file.id)}
                      <div class="flex items-center gap-1.5 opacity-50">
                        <span
                          class="inline-flex rounded bg-sky-600/20 px-1 py-0.5 text-[10px] font-semibold text-sky-700 dark:bg-sky-400/20 dark:text-sky-300"
                        >
                          {formatFormatLabel(file.format, file.format_version)}
                        </span>
                        <span class="text-muted-foreground">
                          {formatFileSize(file.file_size)}
                        </span>
                      </div>
                    {/each}
                  </div>
                </div>
              {:else}
                <span class="mt-0.5 text-muted-foreground">--</span>
              {/if}
            {/if}
          </dd>
        </div>
      {/if}

      <div class={diffClass(book.metadata_status, otherBook.metadata_status)}>
        <dt class="text-xs font-medium text-muted-foreground">Status</dt>
        <dd class="flex items-center gap-2">
          <span>{book.metadata_status}</span>
          <span class="text-xs text-muted-foreground">({formatResolutionLabel(book)})</span>
        </dd>
      </div>

      {#if book.description || otherBook.description}
        <div class={diffClass(book.description, otherBook.description)}>
          <dt class="text-xs font-medium text-muted-foreground">Description</dt>
          <dd>
            {#if book.description}
              <p class="line-clamp-3 text-xs">{book.description}</p>
            {:else}
              <span class="text-muted-foreground">--</span>
            {/if}
          </dd>
        </div>
      {/if}
    </div>
  </div>
{/snippet}
