<script lang="ts">
  import { api, ApiError } from '$lib/api/index.js';
  import type { TagEntry } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import AutocompleteInput from './AutocompleteInput.svelte';
  import LanguageCombobox from './LanguageCombobox.svelte';

  interface Props {
    bookIds: string[];
    open: boolean;
    onclose: () => void;
    onapply: () => void;
  }

  let { bookIds, open = $bindable(), onclose, onapply }: Props = $props();

  // --- Field inclusion toggles ---
  let includeLanguage = $state(false);
  let includeRating = $state(false);
  let includePublisher = $state(false);
  let includeTags = $state(false);

  // --- Field values ---
  let language = $state('');
  let rating = $state('');
  let editPublisher = $state<{ id: string; name: string } | null>(null);
  let editTags = $state<TagEntry[]>([]);
  let tagMode = $state<'add' | 'replace'>('add');

  // --- Operation state ---
  let applying = $state(false);
  let applyError = $state<string | null>(null);
  let resultMessage = $state<string | null>(null);

  const hasChanges = $derived(includeLanguage || includeRating || includePublisher || includeTags);

  function resetForm() {
    includeLanguage = false;
    includeRating = false;
    includePublisher = false;
    includeTags = false;
    language = '';
    rating = '';
    editPublisher = null;
    editTags = [];
    tagMode = 'add';
    applying = false;
    applyError = null;
    resultMessage = null;
  }

  function handleClose() {
    resetForm();
    onclose();
  }

  // --- Publisher search ---
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

  function removePublisher() {
    editPublisher = null;
  }

  // --- Tag search ---
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
    editTags = [
      ...editTags,
      {
        id: `new:${name}`,
        name,
        category: null
      }
    ];
  }

  function removeTag(index: number) {
    editTags = editTags.filter((_, i) => i !== index);
  }

  // --- Apply ---
  async function handleApply() {
    applying = true;
    applyError = null;
    resultMessage = null;

    try {
      const errors: Array<{ book_id: string; error: string }> = [];
      let totalUpdated = 0;

      // Batch update scalar fields
      if (includeLanguage || includeRating || includePublisher) {
        const updates: Record<string, unknown> = {};
        if (includeLanguage) updates.language = language || undefined;
        if (includeRating) {
          const ratingVal = rating === '' ? undefined : Number(rating);
          updates.rating = ratingVal;
        }
        if (includePublisher) {
          updates.publisher_id = editPublisher?.id ?? null;
        }

        const result = await api.books.batchUpdate({
          book_ids: bookIds,
          updates
        });
        totalUpdated = Math.max(totalUpdated, result.updated_count);
        errors.push(...result.errors);
      }

      // Batch update tags
      if (includeTags && editTags.length > 0) {
        const tagPayload = editTags.map((t) => {
          if (t.id.startsWith('new:')) {
            return { name: t.name, category: t.category ?? undefined };
          }
          return { tag_id: t.id };
        });

        const result = await api.books.batchTags({
          book_ids: bookIds,
          tags: tagPayload,
          mode: tagMode
        });
        totalUpdated = Math.max(totalUpdated, result.updated_count);
        errors.push(...result.errors);
      }

      if (errors.length > 0) {
        resultMessage = `Updated ${totalUpdated} books, ${errors.length} error(s)`;
        applyError = errors.map((e) => `${e.book_id}: ${e.error}`).join('; ');
      } else {
        resultMessage = `Updated ${totalUpdated} books`;
      }

      // Signal parent to refresh after short delay
      setTimeout(() => {
        onapply();
        handleClose();
      }, 1200);
    } catch (err) {
      applyError =
        err instanceof ApiError
          ? err.message
          : err instanceof Error
            ? err.message
            : 'Failed to apply batch update';
    } finally {
      applying = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Editing {bookIds.length} books</Dialog.Title>
      <Dialog.Description>
        Select fields to update across all selected books. Only checked fields will be modified.
      </Dialog.Description>
    </Dialog.Header>

    <div class="max-h-[60vh] space-y-4 overflow-y-auto py-2">
      <!-- Language -->
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="flex size-4 items-center justify-center rounded border transition-colors {includeLanguage
              ? 'border-primary bg-primary'
              : 'border-muted-foreground/50'}"
            onclick={() => (includeLanguage = !includeLanguage)}
            aria-label="Include language in update"
          >
            {#if includeLanguage}
              <svg
                class="size-3 text-primary-foreground"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="20 6 9 17 4 12" />
              </svg>
            {/if}
          </button>
          <Label for="batch-language">Language</Label>
        </div>
        {#if includeLanguage}
          <LanguageCombobox
            id="batch-language"
            bind:value={language}
            onchange={(code) => (language = code)}
            class="h-8 text-sm"
          />
        {/if}
      </div>

      <!-- Rating -->
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="flex size-4 items-center justify-center rounded border transition-colors {includeRating
              ? 'border-primary bg-primary'
              : 'border-muted-foreground/50'}"
            onclick={() => (includeRating = !includeRating)}
            aria-label="Include rating in update"
          >
            {#if includeRating}
              <svg
                class="size-3 text-primary-foreground"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="20 6 9 17 4 12" />
              </svg>
            {/if}
          </button>
          <Label for="batch-rating">Rating (0-5)</Label>
        </div>
        {#if includeRating}
          <Input
            id="batch-rating"
            type="number"
            min="0"
            max="5"
            step="0.5"
            bind:value={rating}
            placeholder="0.0"
            class="h-8 text-sm"
          />
        {/if}
      </div>

      <!-- Publisher -->
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="flex size-4 items-center justify-center rounded border transition-colors {includePublisher
              ? 'border-primary bg-primary'
              : 'border-muted-foreground/50'}"
            onclick={() => (includePublisher = !includePublisher)}
            aria-label="Include publisher in update"
          >
            {#if includePublisher}
              <svg
                class="size-3 text-primary-foreground"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="20 6 9 17 4 12" />
              </svg>
            {/if}
          </button>
          <Label>Publisher</Label>
        </div>
        {#if includePublisher}
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
            <p class="text-xs text-muted-foreground">
              Leave empty to clear publisher from all selected books.
            </p>
          {/if}
        {/if}
      </div>

      <!-- Tags -->
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="flex size-4 items-center justify-center rounded border transition-colors {includeTags
              ? 'border-primary bg-primary'
              : 'border-muted-foreground/50'}"
            onclick={() => (includeTags = !includeTags)}
            aria-label="Include tags in update"
          >
            {#if includeTags}
              <svg
                class="size-3 text-primary-foreground"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="3"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="20 6 9 17 4 12" />
              </svg>
            {/if}
          </button>
          <Label>Tags</Label>
        </div>
        {#if includeTags}
          <!-- Mode toggle -->
          <div class="flex gap-1 rounded-md border border-input p-0.5">
            <button
              type="button"
              class="flex-1 rounded px-2 py-1 text-xs font-medium transition-colors {tagMode ===
              'add'
                ? 'bg-primary text-primary-foreground'
                : 'text-muted-foreground hover:text-foreground'}"
              onclick={() => (tagMode = 'add')}
            >
              Add tags
            </button>
            <button
              type="button"
              class="flex-1 rounded px-2 py-1 text-xs font-medium transition-colors {tagMode ===
              'replace'
                ? 'bg-primary text-primary-foreground'
                : 'text-muted-foreground hover:text-foreground'}"
              onclick={() => (tagMode = 'replace')}
            >
              Replace tags
            </button>
          </div>
          <p class="text-xs text-muted-foreground">
            {tagMode === 'add'
              ? 'Selected tags will be added to existing tags on each book.'
              : 'All existing tags will be replaced with the selected tags.'}
          </p>

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
        {/if}
      </div>
    </div>

    <!-- Result / error messages -->
    {#if resultMessage}
      <div
        class="rounded-md border border-green-500/30 bg-green-500/10 px-3 py-2 text-sm text-green-700 dark:text-green-400"
      >
        {resultMessage}
      </div>
    {/if}
    {#if applyError}
      <div
        class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {applyError}
      </div>
    {/if}

    <Dialog.Footer>
      <Button variant="outline" onclick={handleClose} disabled={applying}>Cancel</Button>
      <Button onclick={handleApply} disabled={!hasChanges || applying}>
        {#if applying}
          <svg
            class="size-4 animate-spin"
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
          Applying...
        {:else}
          Apply
        {/if}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
