<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type { MetadataRuleResponse } from '$lib/api/types.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Input } from '$lib/components/ui/input/index.js';

  interface Props {
    open: boolean;
    onadd: (rule: MetadataRuleResponse) => void;
  }

  let { open = $bindable(), onadd }: Props = $props();

  let matchValue = $state('');
  let matchMode = $state<'exact' | 'contains'>('exact');
  let adding = $state(false);
  let addError = $state<string | null>(null);

  export function resetAndOpen() {
    matchValue = '';
    matchMode = 'exact';
    addError = null;
    open = true;
  }

  function handleOpenChange(isOpen: boolean) {
    open = isOpen;
  }

  async function handleSubmit() {
    const trimmed = matchValue.trim();
    if (!trimmed) return;

    adding = true;
    addError = null;

    try {
      const rule = await api.metadataRules.create({
        rule_type: 'publisher',
        match_value: trimmed,
        match_mode: matchMode
      });
      onadd(rule);
      open = false;
    } catch (err) {
      addError = err instanceof Error ? err.message : 'Failed to create rule';
    } finally {
      adding = false;
    }
  }
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>Add Metadata Rule</Dialog.Title>
      <Dialog.Description>
        Books from matching publishers will be marked as identified and skip external metadata
        lookups.
      </Dialog.Description>
    </Dialog.Header>

    <div class="space-y-4">
      <!-- Rule type (read-only for now, only publisher) -->
      <div>
        <label for="rule-type" class="mb-1.5 block text-sm font-medium">Rule Type</label>
        <div
          class="flex h-9 items-center rounded-md border border-input bg-muted/50 px-3 text-sm text-muted-foreground"
        >
          Publisher
        </div>
      </div>

      <!-- Match value -->
      <div>
        <label for="match-value" class="mb-1.5 block text-sm font-medium">Publisher Name</label>
        <Input
          id="match-value"
          type="text"
          placeholder="e.g. Standard Ebooks"
          bind:value={matchValue}
          onkeydown={(e: KeyboardEvent) => {
            if (e.key === 'Enter') handleSubmit();
          }}
        />
      </div>

      <!-- Match mode selector -->
      <fieldset>
        <legend class="mb-2 text-sm font-medium">Match Mode</legend>
        <div class="space-y-2">
          <label
            class="flex cursor-pointer gap-3 rounded-lg border px-4 py-3 transition-colors {matchMode ===
            'exact'
              ? 'border-primary bg-primary/5'
              : 'border-border hover:bg-muted/50'}"
          >
            <input
              type="radio"
              name="match-mode"
              value="exact"
              bind:group={matchMode}
              class="mt-0.5"
            />
            <div class="flex-1">
              <p class="text-sm font-medium">Exact</p>
              <p class="mt-0.5 text-xs text-muted-foreground">
                Publisher name must match exactly (case-insensitive).
              </p>
            </div>
          </label>

          <label
            class="flex cursor-pointer gap-3 rounded-lg border px-4 py-3 transition-colors {matchMode ===
            'contains'
              ? 'border-primary bg-primary/5'
              : 'border-border hover:bg-muted/50'}"
          >
            <input
              type="radio"
              name="match-mode"
              value="contains"
              bind:group={matchMode}
              class="mt-0.5"
            />
            <div class="flex-1">
              <p class="text-sm font-medium">Contains</p>
              <p class="mt-0.5 text-xs text-muted-foreground">
                Publisher name must contain this text (case-insensitive). Useful for publishers with
                varying suffixes.
              </p>
            </div>
          </label>
        </div>
      </fieldset>

      <!-- Outcome (read-only for now) -->
      <div>
        <label for="outcome" class="mb-1.5 block text-sm font-medium">Outcome</label>
        <div
          class="flex h-9 items-center rounded-md border border-input bg-muted/50 px-3 text-sm text-muted-foreground"
        >
          Trust Metadata
        </div>
        <p class="mt-1 text-xs text-muted-foreground">
          Accept the book's embedded metadata as-is and skip external provider lookups.
        </p>
      </div>

      <!-- Error -->
      {#if addError}
        <div
          class="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
        >
          {addError}
        </div>
      {/if}
    </div>

    <Dialog.Footer>
      <Dialog.Close>Cancel</Dialog.Close>
      <Button onclick={handleSubmit} disabled={adding || !matchValue.trim()}>
        {adding ? 'Adding...' : 'Add Rule'}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
