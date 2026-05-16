<script lang="ts">
  import { untrack } from 'svelte';
  import { api, formatError } from '$lib/api/index.js';
  import type { MergeSeriesResponse } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';

  interface SelectedSeries {
    id: string;
    name: string;
    book_count: number;
  }

  interface Props {
    open: boolean;
    selected: SelectedSeries[];
    onclose: () => void;
    onmerged: (result: MergeSeriesResponse) => void;
  }

  let { open = $bindable(), selected, onclose, onmerged }: Props = $props();

  // Default survivor: highest book_count, ties broken alphabetically.
  function defaultSurvivorId(items: SelectedSeries[]): string {
    if (items.length === 0) return '';
    const sorted = [...items].sort((a, b) => {
      if (b.book_count !== a.book_count) return b.book_count - a.book_count;
      return a.name.localeCompare(b.name);
    });
    return sorted[0].id;
  }

  let survivorId = $state<string>('');
  let nameInput = $state<string>('');
  let nameTouched = $state(false);
  let submitting = $state(false);
  let errorMsg = $state<string | null>(null);

  // (Re)initialize state when the dialog opens with a fresh selection.
  // NOTE: do not read `survivorId` inside this effect — see also `untrack`.
  $effect(() => {
    if (open) {
      untrack(() => {
        const defaultId = defaultSurvivorId(selected);
        survivorId = defaultId;
        nameInput = selected.find((s) => s.id === defaultId)?.name ?? '';
        nameTouched = false;
        errorMsg = null;
      });
    }
  });

  // If user picks a different survivor without having touched the name, sync.
  function handleSurvivorChange(id: string) {
    survivorId = id;
    if (!nameTouched) {
      const survivor = selected.find((s) => s.id === id);
      nameInput = survivor?.name ?? '';
    }
  }

  function handleNameInput(e: Event) {
    nameInput = (e.target as HTMLInputElement).value;
    nameTouched = true;
  }

  const survivor = $derived(selected.find((s) => s.id === survivorId));
  const sources = $derived(selected.filter((s) => s.id !== survivorId));
  const movingBookCount = $derived(sources.reduce((sum, s) => sum + s.book_count, 0));
  const trimmedName = $derived(nameInput.trim());
  const hasRename = $derived(survivor != null && trimmedName !== '' && trimmedName !== survivor.name);
  const canSubmit = $derived(
    !submitting && survivor != null && sources.length > 0 && trimmedName !== ''
  );

  async function handleMerge() {
    if (!survivor || sources.length === 0 || trimmedName === '') return;
    submitting = true;
    errorMsg = null;
    try {
      const result = await api.series.merge({
        target_id: survivor.id,
        source_ids: sources.map((s) => s.id),
        target_name: hasRename ? trimmedName : undefined
      });
      onmerged(result);
    } catch (err) {
      errorMsg = formatError(err, 'Failed to merge series');
    } finally {
      submitting = false;
    }
  }

  function handleOpenChange(isOpen: boolean) {
    open = isOpen;
    if (!isOpen) onclose();
  }
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Merge {selected.length} series</Dialog.Title>
      <Dialog.Description>
        Pick the series to keep. The others will be deleted and their books moved into the survivor.
      </Dialog.Description>
    </Dialog.Header>

    <div class="space-y-4">
      <div class="space-y-1.5">
        <p class="text-xs font-medium text-muted-foreground">Keep as authoritative</p>
        <div class="rounded-lg border border-border">
          {#each selected as item, idx (item.id)}
            <label
              class="flex cursor-pointer items-center gap-3 px-3 py-2 transition-colors hover:bg-muted/40 {idx >
              0
                ? 'border-t border-border'
                : ''}"
            >
              <input
                type="radio"
                name="merge-survivor"
                value={item.id}
                checked={survivorId === item.id}
                onchange={() => handleSurvivorChange(item.id)}
                class="size-4 accent-primary"
                disabled={submitting}
              />
              <span class="flex-1 truncate text-sm font-medium">{item.name}</span>
              <span class="text-xs text-muted-foreground">
                {item.book_count}
                {item.book_count === 1 ? 'book' : 'books'}
              </span>
            </label>
          {/each}
        </div>
      </div>

      <div class="space-y-1.5">
        <label for="merge-series-name" class="text-xs font-medium text-muted-foreground">
          Series name
        </label>
        <Input
          id="merge-series-name"
          value={nameInput}
          oninput={handleNameInput}
          placeholder="Series name"
          disabled={submitting}
        />
        {#if hasRename}
          <p class="text-xs text-muted-foreground">The survivor will be renamed.</p>
        {/if}
      </div>

      {#if survivor && sources.length > 0}
        <div class="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs">
          <span class="font-medium">
            {movingBookCount}
            {movingBookCount === 1 ? 'book' : 'books'}
          </span>
          from
          <span class="font-medium">
            {sources.length}
            other series
          </span>
          will move into
          <span class="font-medium">"{trimmedName || survivor.name}"</span>. The
          {sources.length}
          other
          series will be deleted. This cannot be undone.
        </div>
      {/if}

      {#if errorMsg}
        <div
          class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        >
          {errorMsg}
        </div>
      {/if}
    </div>

    <Dialog.Footer>
      <Button variant="outline" onclick={onclose} disabled={submitting}>Cancel</Button>
      <Button onclick={handleMerge} disabled={!canSubmit}>
        {#if submitting}
          Merging...
        {:else}
          Merge
        {/if}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
