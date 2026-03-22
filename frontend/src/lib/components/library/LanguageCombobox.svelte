<script lang="ts">
  import { Input } from '$lib/components/ui/input/index.js';
  import { LANGUAGES } from '$lib/data/languages.js';

  interface Props {
    value: string;
    id?: string;
    placeholder?: string;
    class?: string;
    onchange: (code: string) => void;
  }

  let { value = $bindable(), id, placeholder = 'Language', class: className, onchange }: Props =
    $props();

  let query = $state('');
  let open = $state(false);
  let highlightIndex = $state(-1);
  let containerRef = $state<HTMLDivElement | null>(null);

  /** Display text for the input when not searching. */
  const displayLabel = $derived.by(() => {
    if (!value) return '';
    const entry = LANGUAGES.find(([code]) => code === value);
    return entry ? `${entry[1]} (${entry[0]})` : value;
  });

  /** Filtered results based on search query. */
  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return LANGUAGES;
    return LANGUAGES.filter(
      ([code, label]) => label.toLowerCase().includes(q) || code.includes(q)
    );
  });

  function handleFocus() {
    query = '';
    open = true;
    highlightIndex = -1;
  }

  function selectItem(code: string) {
    value = code;
    query = '';
    open = false;
    onchange(code);
  }

  function clearValue() {
    value = '';
    query = '';
    open = false;
    onchange('');
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      highlightIndex = Math.min(highlightIndex + 1, filtered.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      highlightIndex = Math.max(highlightIndex - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (highlightIndex >= 0 && highlightIndex < filtered.length) {
        selectItem(filtered[highlightIndex][0]);
      }
    } else if (e.key === 'Escape') {
      open = false;
      query = '';
    }
  }

  function handleBlur(e: FocusEvent) {
    const related = e.relatedTarget as Node | null;
    if (containerRef && related && containerRef.contains(related)) return;
    setTimeout(() => {
      open = false;
      query = '';
    }, 150);
  }
</script>

<div class="relative" bind:this={containerRef}>
  <Input
    type="text"
    {id}
    {placeholder}
    value={open ? query : displayLabel}
    oninput={(e: Event) => {
      query = (e.target as HTMLInputElement).value;
      highlightIndex = -1;
    }}
    onfocus={handleFocus}
    onkeydown={handleKeydown}
    onblur={handleBlur}
    class={className}
  />
  {#if open}
    <div
      class="absolute z-50 mt-1 max-h-48 w-full overflow-y-auto rounded-md border border-border bg-popover shadow-md"
    >
      {#if value}
        <button
          type="button"
          class="flex w-full items-center px-3 py-1.5 text-left text-sm text-muted-foreground transition-colors hover:bg-accent"
          onmousedown={(e) => {
            e.preventDefault();
            clearValue();
          }}
        >
          Clear
        </button>
      {/if}
      {#each filtered as [code, label], i (code)}
        <button
          type="button"
          class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent {i === highlightIndex
            ? 'bg-accent'
            : ''} {code === value ? 'font-semibold' : ''}"
          onmousedown={(e) => {
            e.preventDefault();
            selectItem(code);
          }}
        >
          <span>{label}</span>
          <span class="text-xs text-muted-foreground">({code})</span>
        </button>
      {/each}
      {#if filtered.length === 0}
        <div class="px-3 py-2 text-sm text-muted-foreground">No matching languages</div>
      {/if}
    </div>
  {/if}
</div>
