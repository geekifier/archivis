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
  title={isProtected ? `Unprotect ${field}` : `Protect ${field}`}
  {disabled}
  onclick={() => ontoggle(field, !isProtected)}
>
  <svg
    class="size-3.5"
    viewBox="0 0 24 24"
    fill={isProtected ? 'currentColor' : 'none'}
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path
      d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"
    />
  </svg>
</button>
