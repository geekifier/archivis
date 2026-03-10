<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type { MetadataRuleResponse } from '$lib/api/types.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';

  interface Props {
    rule: MetadataRuleResponse;
    onupdate: (rule: MetadataRuleResponse) => void;
    ondelete: (id: string) => void;
  }

  let { rule, onupdate, ondelete }: Props = $props();

  let toggling = $state(false);
  let deleting = $state(false);
  let deleteOpen = $state(false);

  const outcomeBadgeClass = $derived(
    rule.outcome === 'trust_metadata'
      ? 'bg-green-500/10 text-green-700 dark:text-green-400'
      : 'bg-blue-500/10 text-blue-700 dark:text-blue-400'
  );

  const outcomeBadgeText = $derived(
    rule.outcome === 'trust_metadata' ? 'trust metadata' : rule.outcome
  );

  const matchModeBadgeClass = 'bg-zinc-500/10 text-zinc-700 dark:text-zinc-400';

  async function toggleEnabled() {
    toggling = true;
    try {
      const updated = await api.metadataRules.update(rule.id, {
        enabled: !rule.enabled
      });
      onupdate(updated);
    } catch {
      // Silently handle -- the row will remain in its current state
    } finally {
      toggling = false;
    }
  }

  async function handleDelete() {
    deleting = true;
    try {
      await api.metadataRules.delete(rule.id);
      ondelete(rule.id);
      deleteOpen = false;
    } catch {
      // Delete failure -- row remains
    } finally {
      deleting = false;
    }
  }
</script>

<div class="px-6 py-4">
  <div class="flex items-start justify-between gap-4">
    <!-- Left: rule info -->
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <!-- Status dot -->
        <span
          class="inline-block size-2.5 shrink-0 rounded-full {rule.enabled
            ? 'bg-green-500'
            : 'bg-zinc-400'}"
          title={rule.enabled ? 'Enabled' : 'Disabled'}
        ></span>

        <!-- Match value -->
        <span class="truncate text-sm font-medium" title={rule.match_value}>
          {rule.match_value}
        </span>

        {#if rule.builtin}
          <span
            class="inline-flex items-center rounded-full bg-violet-500/10 px-2 py-0.5 text-xs font-medium text-violet-700 dark:text-violet-400"
          >
            builtin
          </span>
        {/if}
      </div>

      <div class="mt-1 flex flex-wrap items-center gap-2">
        <!-- Rule type badge -->
        <span
          class="inline-flex items-center rounded-full bg-blue-500/10 px-2 py-0.5 text-xs font-medium text-blue-700 dark:text-blue-400"
        >
          {rule.rule_type}
        </span>

        <!-- Match mode badge -->
        <span
          class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium {matchModeBadgeClass}"
        >
          {rule.match_mode}
        </span>

        <!-- Outcome badge -->
        <span
          class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium {outcomeBadgeClass}"
        >
          {outcomeBadgeText}
        </span>
      </div>
    </div>

    <!-- Right: actions -->
    <div class="flex shrink-0 items-center gap-1">
      <!-- Enabled toggle -->
      <button
        type="button"
        role="switch"
        aria-checked={rule.enabled}
        aria-label="Toggle rule"
        title={rule.enabled ? 'Disable rule' : 'Enable rule'}
        class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors
          {rule.enabled ? 'bg-primary' : 'bg-muted'}"
        disabled={toggling}
        onclick={toggleEnabled}
      >
        <span
          class="pointer-events-none inline-block size-4 rounded-full bg-background shadow-sm ring-0 transition-transform
            {rule.enabled ? 'translate-x-4' : 'translate-x-0'}"
        ></span>
      </button>

      <!-- Delete (hidden for builtin rules) -->
      {#if !rule.builtin}
        <AlertDialog.Root bind:open={deleteOpen}>
          <AlertDialog.Trigger>
            {#snippet child({ props })}
              <Button variant="ghost" size="icon-sm" title="Delete" {...props}>
                <svg
                  class="size-4 text-destructive"
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
                  <line x1="10" x2="10" y1="11" y2="17" />
                  <line x1="14" x2="14" y1="11" y2="17" />
                </svg>
              </Button>
            {/snippet}
          </AlertDialog.Trigger>
          <AlertDialog.Content>
            <AlertDialog.Header>
              <AlertDialog.Title>Remove Metadata Rule</AlertDialog.Title>
              <AlertDialog.Description>
                This will remove the rule for "{rule.match_value}". Books already imported with this
                rule will not be affected.
              </AlertDialog.Description>
            </AlertDialog.Header>
            <AlertDialog.Footer>
              <AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
              <AlertDialog.Action onclick={handleDelete} disabled={deleting}>
                {deleting ? 'Removing...' : 'Remove'}
              </AlertDialog.Action>
            </AlertDialog.Footer>
          </AlertDialog.Content>
        </AlertDialog.Root>
      {/if}
    </div>
  </div>
</div>
