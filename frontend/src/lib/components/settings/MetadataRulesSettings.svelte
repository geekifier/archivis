<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type { MetadataRuleResponse } from '$lib/api/types.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import MetadataRuleRow from './MetadataRuleRow.svelte';
  import AddMetadataRuleDialog from './AddMetadataRuleDialog.svelte';

  let rules = $state<MetadataRuleResponse[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let dialogOpen = $state(false);
  let dialogComponent = $state<AddMetadataRuleDialog | null>(null);

  async function fetchRules() {
    loading = true;
    error = null;
    try {
      rules = await api.metadataRules.list();
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to load metadata rules';
    } finally {
      loading = false;
    }
  }

  function handleAdd(rule: MetadataRuleResponse) {
    rules = [...rules, rule];
  }

  function handleUpdate(rule: MetadataRuleResponse) {
    rules = rules.map((r) => (r.id === rule.id ? rule : r));
  }

  function handleDelete(id: string) {
    rules = rules.filter((r) => r.id !== id);
  }

  $effect(() => {
    fetchRules();
  });
</script>

<div class="rounded-lg border border-border bg-card">
  <div class="border-b border-border px-6 py-4">
    <div class="flex items-center justify-between">
      <div>
        <h2 class="text-base font-semibold">Metadata Rules</h2>
        <p class="mt-0.5 text-xs text-muted-foreground">
          Define rules for trusted publishers whose embedded metadata should be accepted without
          external lookups.
        </p>
      </div>
      {#if !loading}
        <Button
          variant="outline"
          size="sm"
          onclick={() => dialogComponent?.resetAndOpen()}
        >
          <svg
            class="mr-1.5 size-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M5 12h14" />
            <path d="M12 5v14" />
          </svg>
          Add Rule
        </Button>
      {/if}
    </div>
  </div>

  <!-- Error state -->
  {#if error}
    <div class="px-6 py-4">
      <div
        class="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
      >
        <svg
          class="size-4 shrink-0"
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="12" x2="12" y1="8" y2="12" />
          <line x1="12" x2="12.01" y1="16" y2="16" />
        </svg>
        <span>{error}</span>
      </div>
    </div>
  {/if}

  <!-- Loading state -->
  {#if loading}
    <div class="flex items-center justify-center px-6 py-8">
      <span class="text-sm text-muted-foreground">Loading metadata rules...</span>
    </div>
  {:else if !error}
    <!-- Rules list -->
    {#if rules.length === 0}
      <div class="px-6 py-8 text-center">
        <p class="text-sm text-muted-foreground">
          No metadata rules configured. Add a rule to trust embedded metadata from specific
          publishers.
        </p>
      </div>
    {:else}
      <div class="divide-y divide-border">
        {#each rules as rule (rule.id)}
          <MetadataRuleRow {rule} onupdate={handleUpdate} ondelete={handleDelete} />
        {/each}
      </div>
    {/if}
  {/if}
</div>

<AddMetadataRuleDialog
  bind:this={dialogComponent}
  bind:open={dialogOpen}
  onadd={handleAdd}
/>
