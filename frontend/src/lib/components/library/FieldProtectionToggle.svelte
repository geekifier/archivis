<script lang="ts">
  import type { MetadataField } from '$lib/api/index.js';

  interface Props {
    field: MetadataField;
    protected: boolean;
    disabled?: boolean;
    ontoggle: (field: MetadataField, newProtected: boolean) => void;
  }

  let { field, protected: isProtected, disabled = false, ontoggle }: Props = $props();
</script>

<button
  type="button"
  class="p-0.5 transition-colors {isProtected
    ? 'text-primary'
    : 'text-muted-foreground/40 hover:text-muted-foreground'} disabled:opacity-30 disabled:pointer-events-none"
  title={isProtected ? `Unlock ${field}` : `Lock ${field}`}
  {disabled}
  onclick={() => ontoggle(field, !isProtected)}
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
    {#if isProtected}
      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
    {:else}
      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
      <path d="M7 11V7a5 5 0 0 1 9.9-1" />
    {/if}
  </svg>
</button>
