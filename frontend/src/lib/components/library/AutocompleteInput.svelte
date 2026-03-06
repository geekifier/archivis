<script lang="ts">
  import { Input } from '$lib/components/ui/input/index.js';

  interface Item {
    id: string;
    label: string;
    sublabel?: string;
  }

  interface Props {
    placeholder?: string;
    search: (query: string) => Promise<Item[]>;
    onselect: (item: Item) => void;
    /** Allow creating a new entry when no results match. */
    allowCreate?: boolean;
    /** Called when user chooses to create a new entry with the typed text. */
    oncreate?: (text: string) => void;
  }

  let {
    placeholder = 'Search...',
    search,
    onselect,
    allowCreate = false,
    oncreate
  }: Props = $props();

  let query = $state('');
  let results = $state<Item[]>([]);
  let open = $state(false);
  let loading = $state(false);
  let debounceTimer = $state<ReturnType<typeof setTimeout> | undefined>(undefined);
  let containerRef = $state<HTMLDivElement | null>(null);
  let highlightIndex = $state(-1);

  function handleInput() {
    const q = query.trim();
    if (debounceTimer) clearTimeout(debounceTimer);
    if (q.length < 1) {
      results = [];
      open = false;
      return;
    }
    loading = true;
    debounceTimer = setTimeout(async () => {
      try {
        results = await search(q);
      } catch {
        results = [];
      } finally {
        loading = false;
        open = true;
        highlightIndex = -1;
      }
    }, 300);
  }

  function select(item: Item) {
    onselect(item);
    query = '';
    results = [];
    open = false;
  }

  function createNew() {
    const text = query.trim();
    if (text && oncreate) {
      oncreate(text);
      query = '';
      results = [];
      open = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;
    const maxIndex = results.length + (showCreate ? 1 : 0) - 1;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      highlightIndex = Math.min(highlightIndex + 1, maxIndex);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      highlightIndex = Math.max(highlightIndex - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (highlightIndex >= 0 && highlightIndex < results.length) {
        select(results[highlightIndex]);
      } else if (showCreate && highlightIndex === results.length) {
        createNew();
      }
    } else if (e.key === 'Escape') {
      open = false;
    }
  }

  function handleBlur(e: FocusEvent) {
    // Close dropdown when focus leaves the container
    const related = e.relatedTarget as Node | null;
    if (containerRef && related && containerRef.contains(related)) return;
    // Delay to allow click events on dropdown items to fire
    setTimeout(() => {
      open = false;
    }, 150);
  }

  const showCreate = $derived(
    allowCreate && oncreate && query.trim().length > 0 && results.length === 0 && !loading
  );
</script>

<div class="relative" bind:this={containerRef}>
  <Input
    type="text"
    {placeholder}
    bind:value={query}
    oninput={handleInput}
    onkeydown={handleKeydown}
    onblur={handleBlur}
    onfocus={() => {
      if (results.length > 0 || showCreate) open = true;
    }}
    class="h-8 text-sm"
  />
  {#if open && (results.length > 0 || showCreate || loading)}
    <div
      class="absolute z-50 mt-1 max-h-48 w-full overflow-y-auto rounded-md border border-border bg-popover shadow-md"
    >
      {#if loading}
        <div class="px-3 py-2 text-sm text-muted-foreground">Searching...</div>
      {:else}
        {#each results as item, i (item.id)}
          <button
            type="button"
            class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent {i ===
            highlightIndex
              ? 'bg-accent'
              : ''}"
            onmousedown={(e) => {
              e.preventDefault();
              select(item);
            }}
          >
            <span class="font-medium">{item.label}</span>
            {#if item.sublabel}
              <span class="text-xs text-muted-foreground">{item.sublabel}</span>
            {/if}
          </button>
        {/each}
        {#if showCreate}
          <button
            type="button"
            class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent {highlightIndex ===
            results.length
              ? 'bg-accent'
              : ''}"
            onmousedown={(e) => {
              e.preventDefault();
              createNew();
            }}
          >
            <span class="text-muted-foreground">Create</span>
            <span class="font-medium">"{query.trim()}"</span>
          </button>
        {/if}
      {/if}
    </div>
  {/if}
</div>
