<script lang="ts">
  import type { QueryWarning } from '$lib/api/types.js';

  let {
    warnings,
    onpick
  }: {
    warnings: QueryWarning[];
    onpick?: (field: string, query: string, id: string, name: string, negated: boolean) => void;
  } = $props();

  /** Produce a stable key for a warning (no `id` field on the type). */
  function warningKey(w: QueryWarning): string {
    switch (w.type) {
      case 'ambiguous_relation':
        return `${w.type}:${w.negated}:${w.field}:${w.query}`;
      case 'unknown_relation':
        return `${w.type}:${w.negated}:${w.field}:${w.query}`;
      case 'invalid_value':
        return `${w.type}:${w.field}:${w.value}`;
      case 'empty_field_value':
        return `${w.type}:${w.field}`;
      case 'unsupported_or_field':
        return `${w.type}:${w.negated}:${w.field}:${w.value}`;
      case 'no_searchable_terms':
        return `${w.type}:${w.field ?? ''}:${w.text}`;
    }
  }
</script>

{#if warnings.length > 0}
  <div class="flex flex-col gap-1.5 text-sm">
    {#each warnings as warning (warningKey(warning))}
      {#if warning.type === 'ambiguous_relation'}
        <div class="text-amber-600 dark:text-amber-400">
          <span class="font-medium">{warning.negated ? '-' : ''}{warning.field}:{warning.query}</span>
          matched {warning.match_count}
          {warning.match_count === 1 ? warning.field : warning.field + 's'}
          — not applied as a filter.
          {#if warning.matches.length > 0 && onpick}
            <span class="text-muted-foreground">Pick one:</span>
            {#each warning.matches as match, i (match.id)}
              {#if i > 0}<span class="text-muted-foreground">,</span>{/if}
              <button
                class="underline cursor-pointer hover:text-amber-700 dark:hover:text-amber-300"
                onclick={() => onpick?.(warning.field, warning.query, match.id, match.name, warning.negated)}
              >
                {match.name}
              </button>
            {/each}
          {/if}
        </div>
      {:else if warning.type === 'unknown_relation'}
        <div class="text-muted-foreground">
          No {warning.field} found matching &lsquo;{warning.negated ? '-' : ''}{warning.query}&rsquo;
        </div>
      {:else if warning.type === 'invalid_value'}
        <div class="text-muted-foreground">
          Invalid {warning.field} value &lsquo;{warning.value}&rsquo;: {warning.reason}
        </div>
      {:else if warning.type === 'empty_field_value'}
        <div class="text-muted-foreground">
          <span class="font-medium">{warning.field}:</span> needs a value
          (e.g. <span class="font-medium">{warning.field}:example</span>)
        </div>
      {:else if warning.type === 'unsupported_or_field'}
        <div class="text-muted-foreground">
          <span class="font-medium">{warning.negated ? '-' : ''}{warning.field}:{warning.value}</span>
          — field filters inside OR are not supported; move the field filter outside OR
          for AND semantics, or run separate searches.
        </div>
      {:else if warning.type === 'no_searchable_terms'}
        <div class="text-muted-foreground">
          <span class="font-medium">{warning.field ? `${warning.field}:${warning.text}` : warning.text}</span>
          — no searchable terms
        </div>
      {/if}
    {/each}
  </div>
{/if}
