<script lang="ts" module>
  /** Return a Tailwind class string for an identifier type badge. */
  function typeColorClass(type: string): string {
    switch (type) {
      case 'isbn13':
      case 'isbn10':
        return 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400';
      case 'asin':
        return 'bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-400';
      case 'google_books':
        return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400';
      case 'open_library':
        return 'bg-indigo-100 text-indigo-800 dark:bg-indigo-900/30 dark:text-indigo-400';
      case 'hardcover':
        return 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400';
      default:
        return 'bg-muted text-muted-foreground';
    }
  }
</script>

<script lang="ts">
  import type { BookDetail, IdentifierEntry } from '$lib/api/index.js';
  import { api, ApiError } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import * as Select from '$lib/components/ui/select/index.js';
  import { formatIdentifierType, formatMetadataSource } from '$lib/utils.js';
  import type { IsbnValidation } from '$lib/utils/isbn.js';
  import { validateIsbn } from '$lib/utils/isbn.js';
  import { formatScore, scoreColor } from './candidate-utils.js';

  const identifierTypeOptions: { value: string; label: string }[] = [
    { value: 'isbn13', label: 'ISBN-13' },
    { value: 'isbn10', label: 'ISBN-10' },
    { value: 'asin', label: 'ASIN' },
    { value: 'google_books', label: 'Google Books' },
    { value: 'open_library', label: 'Open Library' },
    { value: 'hardcover', label: 'Hardcover' }
  ];

  interface Props {
    book: BookDetail;
    onupdate: (updated: BookDetail) => void;
  }

  let { book, onupdate }: Props = $props();

  // --- Add identifier state ---
  let addType = $state<string>('isbn13');
  let addValue = $state('');
  let adding = $state(false);
  let addError = $state<string | null>(null);

  // --- Edit identifier state ---
  let editingId = $state<string | null>(null);
  let editValue = $state('');
  let editType = $state('');
  let saving = $state(false);
  let editError = $state<string | null>(null);

  // --- Delete state ---
  let deletingId = $state<string | null>(null);
  let deleteConfirmId = $state<string | null>(null);
  let deleteError = $state<string | null>(null);

  const isIsbnType = (type: string) => type === 'isbn13' || type === 'isbn10';

  const addIsIsbn = $derived(isIsbnType(addType));
  const addValidation = $derived<IsbnValidation | null>(
    addIsIsbn && addValue.trim() ? validateIsbn(addValue) : null
  );
  const addDisabled = $derived(
    adding || !addValue.trim() || (addIsIsbn && (!addValidation || !addValidation.valid))
  );

  const editIsIsbn = $derived(isIsbnType(editType));
  const editValidation = $derived<IsbnValidation | null>(
    editIsIsbn && editValue.trim() ? validateIsbn(editValue) : null
  );
  const editSaveDisabled = $derived(
    saving || !editValue.trim() || (editIsIsbn && (!editValidation || !editValidation.valid))
  );

  function isProviderSourced(ident: IdentifierEntry): boolean {
    return ident.source.type !== 'user';
  }

  function startEdit(ident: IdentifierEntry) {
    editingId = ident.id;
    editValue = ident.value;
    editType = ident.identifier_type;
    editError = null;
  }

  function cancelEdit() {
    editingId = null;
    editValue = '';
    editType = '';
    editError = null;
  }

  async function handleAdd() {
    adding = true;
    addError = null;
    try {
      const normalizedValue =
        addIsIsbn && addValidation ? addValidation.normalized : addValue.trim();
      const updated = await api.identifiers.add(book.id, {
        identifier_type: addType,
        value: normalizedValue
      });
      onupdate(updated);
      addValue = '';
    } catch (err) {
      addError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to add identifier';
    } finally {
      adding = false;
    }
  }

  async function handleSaveEdit() {
    if (!editingId) return;
    saving = true;
    editError = null;
    try {
      const normalizedValue =
        editIsIsbn && editValidation ? editValidation.normalized : editValue.trim();
      const updated = await api.identifiers.update(book.id, editingId, {
        identifier_type: editType,
        value: normalizedValue
      });
      onupdate(updated);
      cancelEdit();
    } catch (err) {
      editError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to update identifier';
    } finally {
      saving = false;
    }
  }

  async function handleDelete(identId: string) {
    deletingId = identId;
    deleteError = null;
    try {
      await api.identifiers.delete(book.id, identId);
      // Refetch full book to get updated identifiers
      const updated = await api.books.get(book.id);
      onupdate(updated);
      deleteConfirmId = null;
    } catch (err) {
      deleteError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to delete identifier';
    } finally {
      deletingId = null;
    }
  }

  function toggleDeleteConfirm(identId: string) {
    deleteConfirmId = deleteConfirmId === identId ? null : identId;
  }
</script>

<div class="space-y-4">
  <!-- Existing identifiers list -->
  {#if book.identifiers.length > 0}
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-border text-left text-xs text-muted-foreground">
            <th class="pb-2 pr-3 font-medium">Type</th>
            <th class="pb-2 pr-3 font-medium">Value</th>
            <th class="pb-2 pr-3 font-medium">Source</th>
            <th class="pb-2 pr-3 font-medium">Confidence</th>
            <th class="pb-2 font-medium">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each book.identifiers as ident (ident.id)}
            <tr class="border-b border-border/50">
              {#if editingId === ident.id}
                <!-- Edit mode row -->
                <td class="py-2 pr-3">
                  <Select.Root type="single" bind:value={editType}>
                    <Select.Trigger class="h-8 w-28 text-xs">
                      {identifierTypeOptions.find((o) => o.value === editType)?.label ?? editType}
                    </Select.Trigger>
                    <Select.Content>
                      {#each identifierTypeOptions as option (option.value)}
                        <Select.Item value={option.value} label={option.label} />
                      {/each}
                    </Select.Content>
                  </Select.Root>
                </td>
                <td class="py-2 pr-3">
                  <div class="space-y-1">
                    <Input
                      type="text"
                      bind:value={editValue}
                      class="h-8 font-mono text-xs"
                      onkeydown={(e: KeyboardEvent) => {
                        if (e.key === 'Enter' && !editSaveDisabled) handleSaveEdit();
                        if (e.key === 'Escape') cancelEdit();
                      }}
                    />
                    {#if editIsIsbn && editValidation}
                      <div class="flex items-center gap-1.5 text-xs">
                        {#if editValidation.valid}
                          <svg
                            class="size-3.5 text-green-600 dark:text-green-400"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                          >
                            <path d="M20 6 9 17l-5-5" />
                          </svg>
                          <span class="text-green-600 dark:text-green-400">
                            {editValidation.message}
                          </span>
                        {:else}
                          <svg
                            class="size-3.5 text-destructive"
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
                          <span class="text-destructive">{editValidation.message}</span>
                        {/if}
                      </div>
                      {#if editValidation.valid && editValidation.normalized !== editValue}
                        <p class="text-xs text-muted-foreground">
                          Normalized: <span class="font-mono">{editValidation.normalized}</span>
                        </p>
                      {/if}
                      {#if editValidation.valid && editValidation.isbnType === 'isbn13' && editValidation.isbn10Equivalent}
                        <p class="text-xs text-muted-foreground">
                          ISBN-10: <span class="font-mono">{editValidation.isbn10Equivalent}</span>
                        </p>
                      {/if}
                      {#if editValidation.valid && editValidation.isbnType === 'isbn10' && editValidation.isbn13Equivalent}
                        <p class="text-xs text-muted-foreground">
                          ISBN-13: <span class="font-mono">{editValidation.isbn13Equivalent}</span>
                        </p>
                      {/if}
                    {/if}
                    {#if editError}
                      <p class="text-xs text-destructive">{editError}</p>
                    {/if}
                  </div>
                </td>
                <td class="py-2 pr-3"></td>
                <td class="py-2 pr-3"></td>
                <td class="py-2">
                  <div class="flex items-center gap-1">
                    <Button
                      size="sm"
                      class="h-7 px-2 text-xs"
                      disabled={editSaveDisabled}
                      onclick={handleSaveEdit}
                    >
                      {saving ? 'Saving...' : 'Save'}
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      class="h-7 px-2 text-xs"
                      disabled={saving}
                      onclick={cancelEdit}
                    >
                      Cancel
                    </Button>
                  </div>
                </td>
              {:else}
                <!-- Display mode row -->
                <td class="py-2 pr-3">
                  <span
                    class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {typeColorClass(
                      ident.identifier_type
                    )}"
                  >
                    {formatIdentifierType(ident.identifier_type)}
                  </span>
                </td>
                <td class="py-2 pr-3 font-mono text-xs">{ident.value}</td>
                <td class="py-2 pr-3">
                  <div class="flex items-center gap-1.5">
                    {#if isProviderSourced(ident)}
                      <span title="Auto-detected by {formatMetadataSource(ident.source)}">
                        <svg
                          class="size-3.5 text-muted-foreground"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        >
                          <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
                          <path d="M7 11V7a5 5 0 0 1 10 0v4" />
                        </svg>
                      </span>
                    {/if}
                    <span class="text-xs text-muted-foreground">
                      {formatMetadataSource(ident.source)}
                    </span>
                  </div>
                </td>
                <td class="py-2 pr-3">
                  <div class="flex items-center gap-1.5">
                    <div class="h-1.5 w-12 overflow-hidden rounded-full bg-muted">
                      <div
                        class="h-full rounded-full transition-all {scoreColor(ident.confidence)}"
                        style="width: {ident.confidence * 100}%"
                      ></div>
                    </div>
                    <span class="text-xs text-muted-foreground">
                      {formatScore(ident.confidence)}
                    </span>
                  </div>
                </td>
                <td class="py-2">
                  <div class="flex items-center gap-1">
                    <button
                      type="button"
                      class="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                      onclick={() => startEdit(ident)}
                      title="Edit identifier"
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
                        <path
                          d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"
                        />
                        <path d="m15 5 4 4" />
                      </svg>
                    </button>
                    {#if deleteConfirmId === ident.id}
                      <div
                        class="flex items-center gap-1 rounded border border-destructive/30 bg-destructive/5 px-2 py-0.5"
                      >
                        <span class="text-xs text-destructive">
                          {isProviderSourced(ident) ? 'Auto-detected. Remove?' : 'Remove?'}
                        </span>
                        <button
                          type="button"
                          class="rounded p-0.5 text-destructive transition-colors hover:bg-destructive/20"
                          onclick={() => handleDelete(ident.id)}
                          disabled={deletingId === ident.id}
                        >
                          {#if deletingId === ident.id}
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
                              <path d="M20 6 9 17l-5-5" />
                            </svg>
                          {/if}
                        </button>
                        <button
                          type="button"
                          class="rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
                          onclick={() => (deleteConfirmId = null)}
                          aria-label="Cancel delete"
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
                      </div>
                    {:else}
                      <button
                        type="button"
                        class="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-destructive"
                        onclick={() => toggleDeleteConfirm(ident.id)}
                        title="Delete identifier"
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
                          <path d="M3 6h18" />
                          <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
                          <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
                        </svg>
                      </button>
                    {/if}
                  </div>
                  {#if deleteError && deleteConfirmId === ident.id}
                    <p class="mt-1 text-xs text-destructive">{deleteError}</p>
                  {/if}
                </td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <p class="text-sm text-muted-foreground">No identifiers yet.</p>
  {/if}

  <!-- Add identifier form -->
  <div class="rounded-lg border border-border bg-card p-3">
    <h4 class="mb-2 text-xs font-medium text-muted-foreground">Add Identifier</h4>
    <div class="flex items-start gap-2">
      <Select.Root type="single" bind:value={addType}>
        <Select.Trigger class="h-9 w-32 text-xs">
          {identifierTypeOptions.find((o) => o.value === addType)?.label ?? addType}
        </Select.Trigger>
        <Select.Content>
          {#each identifierTypeOptions as option (option.value)}
            <Select.Item value={option.value} label={option.label} />
          {/each}
        </Select.Content>
      </Select.Root>
      <div class="flex-1 space-y-1">
        <Input
          type="text"
          bind:value={addValue}
          placeholder={addIsIsbn ? 'Enter ISBN...' : 'Enter identifier value...'}
          class="h-9 font-mono text-xs"
          onkeydown={(e: KeyboardEvent) => {
            if (e.key === 'Enter' && !addDisabled) handleAdd();
          }}
        />
        {#if addIsIsbn && addValidation}
          <div class="flex items-center gap-1.5 text-xs">
            {#if addValidation.valid}
              <svg
                class="size-3.5 text-green-600 dark:text-green-400"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M20 6 9 17l-5-5" />
              </svg>
              <span class="text-green-600 dark:text-green-400">{addValidation.message}</span>
            {:else}
              <svg
                class="size-3.5 text-destructive"
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
              <span class="text-destructive">{addValidation.message}</span>
            {/if}
          </div>
          {#if addValidation.valid && addValidation.normalized !== addValue
                .replace(/[\s-]/g, '')
                .toUpperCase()}
            <p class="text-xs text-muted-foreground">
              Normalized: <span class="font-mono">{addValidation.normalized}</span>
            </p>
          {/if}
          {#if addValidation.valid && addValidation.isbnType === 'isbn13' && addValidation.isbn10Equivalent}
            <p class="text-xs text-muted-foreground">
              ISBN-10 equivalent: <span class="font-mono">{addValidation.isbn10Equivalent}</span>
            </p>
          {/if}
          {#if addValidation.valid && addValidation.isbnType === 'isbn10' && addValidation.isbn13Equivalent}
            <p class="text-xs text-muted-foreground">
              ISBN-13 equivalent: <span class="font-mono">{addValidation.isbn13Equivalent}</span>
            </p>
          {/if}
        {/if}
        {#if addError}
          <p class="text-xs text-destructive">{addError}</p>
        {/if}
      </div>
      <Button size="sm" class="h-9 px-3 text-xs" disabled={addDisabled} onclick={handleAdd}>
        {#if adding}
          Adding...
        {:else}
          Add
        {/if}
      </Button>
    </div>
  </div>
</div>
