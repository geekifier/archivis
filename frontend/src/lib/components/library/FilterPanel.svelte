<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type {
    ResolutionState,
    ResolutionOutcome,
    BookFormat,
    MetadataStatus
  } from '$lib/api/types.js';
  import { filters, type IdentifierType } from '$lib/stores/filters.svelte.js';
  import AutocompleteInput from './AutocompleteInput.svelte';
  import LanguageCombobox from './LanguageCombobox.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';

  let { open = false } = $props();

  // --- Identifier state (derived from store, with local editing overrides) ---
  const idTypeDerived = $derived<IdentifierType>(filters.activeIdentifierType ?? 'isbn');
  const idValueDerived = $derived(filters.activeIdentifierValue);
  let idTypeEditing = $state<IdentifierType | null>(null);
  let idValueEditing = $state<string | null>(null);
  const idType = $derived(idTypeEditing ?? idTypeDerived);
  const idValue = $derived(idValueEditing ?? idValueDerived);

  // --- Year range (derived from store, with local editing overrides) ---
  const yearMinDerived = $derived(filters.activeYearMin !== null ? String(filters.activeYearMin) : '');
  const yearMaxDerived = $derived(filters.activeYearMax !== null ? String(filters.activeYearMax) : '');
  let yearMinEditing = $state<string | null>(null);
  let yearMaxEditing = $state<string | null>(null);
  const yearMinInput = $derived(yearMinEditing ?? yearMinDerived);
  const yearMaxInput = $derived(yearMaxEditing ?? yearMaxDerived);

  const identifierTypes: { value: IdentifierType; label: string }[] = [
    { value: 'isbn', label: 'ISBN' },
    { value: 'asin', label: 'ASIN' },
    { value: 'open_library_id', label: 'Open Library' },
    { value: 'hardcover_id', label: 'Hardcover' }
  ];

  const formatOptions: { value: BookFormat; label: string }[] = [
    { value: 'epub', label: 'EPUB' },
    { value: 'pdf', label: 'PDF' },
    { value: 'mobi', label: 'MOBI' },
    { value: 'cbz', label: 'CBZ' },
    { value: 'fb2', label: 'FB2' },
    { value: 'txt', label: 'TXT' },
    { value: 'djvu', label: 'DJVU' },
    { value: 'azw3', label: 'AZW3' }
  ];

  const statusOptions: { value: MetadataStatus; label: string }[] = [
    { value: 'identified', label: 'Identified' },
    { value: 'needs_review', label: 'Needs Review' },
    { value: 'unidentified', label: 'Unidentified' }
  ];

  const resolutionStateOptions: { value: ResolutionState; label: string }[] = [
    { value: 'pending', label: 'Pending' },
    { value: 'running', label: 'Running' },
    { value: 'done', label: 'Done' },
    { value: 'failed', label: 'Failed' }
  ];

  const resolutionOutcomeOptions: { value: ResolutionOutcome; label: string }[] = [
    { value: 'confirmed', label: 'Confirmed' },
    { value: 'enriched', label: 'Enriched' },
    { value: 'disputed', label: 'Disputed' },
    { value: 'ambiguous', label: 'Ambiguous' },
    { value: 'unmatched', label: 'Unmatched' }
  ];

  const boolOptions = [
    { value: '', label: 'Any' },
    { value: 'true', label: 'Yes' },
    { value: 'false', label: 'No' }
  ];

  // --- Autocomplete search functions ---

  async function searchAuthors(query: string) {
    const result = await api.authors.search(query);
    return result.items.map((a) => ({
      id: a.id,
      label: a.name,
      sublabel: `${a.book_count} book${a.book_count !== 1 ? 's' : ''}`
    }));
  }

  async function searchSeries(query: string) {
    const result = await api.series.search(query);
    return result.items.map((s) => ({
      id: s.id,
      label: s.name,
      sublabel: `${s.book_count} book${s.book_count !== 1 ? 's' : ''}`
    }));
  }

  async function searchPublishers(query: string) {
    const result = await api.publishers.search(query);
    return result.items.map((p) => ({ id: p.id, label: p.name }));
  }

  async function searchTags(query: string) {
    const result = await api.tags.search(query);
    return result.items.map((t) => ({
      id: t.id,
      label: t.name,
      sublabel: t.category ?? undefined
    }));
  }

  // --- Helpers ---

  function boolSelectValue(v: boolean | null): string {
    if (v === null) return '';
    return String(v);
  }

  function parseBoolSelect(val: string): boolean | null {
    if (val === 'true') return true;
    if (val === 'false') return false;
    return null;
  }

  function commitYearMin() {
    const v = yearMinInput.trim();
    filters.setYearMin(v ? parseInt(v, 10) || null : null);
    yearMinEditing = null;
  }

  function commitYearMax() {
    const v = yearMaxInput.trim();
    filters.setYearMax(v ? parseInt(v, 10) || null : null);
    yearMaxEditing = null;
  }

  function commitIdentifier() {
    const val = idValue.trim();
    if (val) {
      filters.setIdentifier(idType, val);
    } else {
      filters.clearIdentifier();
    }
    idTypeEditing = null;
    idValueEditing = null;
  }

  // Language value synced from store (writable for LanguageCombobox bind:value)
  let languageValue = $derived(filters.activeLanguage ?? '');
</script>

{#if open}
    <div class="mt-3 rounded-lg border border-border bg-card p-4">
      <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        <!-- Format -->
        <div class="space-y-1.5">
          <label for="filter-format" class="text-xs font-medium text-muted-foreground">Format</label>
          <select
            id="filter-format"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={filters.activeFormat ?? ''}
            onchange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              filters.setFormat(v ? (v as BookFormat) : null);
            }}
          >
            <option value="">Any</option>
            {#each formatOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <!-- Status -->
        <div class="space-y-1.5">
          <label for="filter-status" class="text-xs font-medium text-muted-foreground">Status</label>
          <select
            id="filter-status"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={filters.activeStatus ?? ''}
            onchange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              filters.setStatus(v ? (v as MetadataStatus) : null);
            }}
          >
            <option value="">Any</option>
            {#each statusOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <!-- Resolution State -->
        <div class="space-y-1.5">
          <label for="filter-resolution-state" class="text-xs font-medium text-muted-foreground">Resolution State</label>
          <select
            id="filter-resolution-state"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={filters.activeResolutionState ?? ''}
            onchange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              filters.setResolutionState(v ? (v as ResolutionState) : null);
            }}
          >
            <option value="">Any</option>
            {#each resolutionStateOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <!-- Resolution Outcome -->
        <div class="space-y-1.5">
          <label for="filter-resolution-outcome" class="text-xs font-medium text-muted-foreground">Resolution Outcome</label>
          <select
            id="filter-resolution-outcome"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={filters.activeResolutionOutcome ?? ''}
            onchange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              filters.setResolutionOutcome(v ? (v as ResolutionOutcome) : null);
            }}
          >
            <option value="">Any</option>
            {#each resolutionOutcomeOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <!-- Author -->
        <div class="space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">Author</span>
          {#if filters.activeAuthor}
            <div
              class="flex h-8 items-center justify-between rounded-md border border-input bg-background px-2 text-sm"
            >
              <span class="truncate">{filters.activeAuthor.name}</span>
              <button
                onclick={() => filters.setAuthor(null)}
                class="ml-1 shrink-0 rounded p-0.5 text-muted-foreground hover:text-foreground"
                aria-label="Clear author filter"
              >
                <svg class="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M18 6 6 18" /><path d="m6 6 12 12" />
                </svg>
              </button>
            </div>
          {:else}
            <AutocompleteInput
              placeholder="Search authors..."
              search={searchAuthors}
              onselect={(item) => filters.setAuthor({ id: item.id, name: item.label })}
            />
          {/if}
        </div>

        <!-- Series -->
        <div class="space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">Series</span>
          {#if filters.activeSeries}
            <div
              class="flex h-8 items-center justify-between rounded-md border border-input bg-background px-2 text-sm"
            >
              <span class="truncate">{filters.activeSeries.name}</span>
              <button
                onclick={() => filters.setSeries(null)}
                class="ml-1 shrink-0 rounded p-0.5 text-muted-foreground hover:text-foreground"
                aria-label="Clear series filter"
              >
                <svg class="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M18 6 6 18" /><path d="m6 6 12 12" />
                </svg>
              </button>
            </div>
          {:else}
            <AutocompleteInput
              placeholder="Search series..."
              search={searchSeries}
              onselect={(item) => filters.setSeries({ id: item.id, name: item.label })}
            />
          {/if}
        </div>

        <!-- Publisher -->
        <div class="space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">Publisher</span>
          {#if filters.activePublisher}
            <div
              class="flex h-8 items-center justify-between rounded-md border border-input bg-background px-2 text-sm"
            >
              <span class="truncate">{filters.activePublisher.name}</span>
              <button
                onclick={() => filters.setPublisher(null)}
                class="ml-1 shrink-0 rounded p-0.5 text-muted-foreground hover:text-foreground"
                aria-label="Clear publisher filter"
              >
                <svg class="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M18 6 6 18" /><path d="m6 6 12 12" />
                </svg>
              </button>
            </div>
          {:else}
            <AutocompleteInput
              placeholder="Search publishers..."
              search={searchPublishers}
              onselect={(item) => filters.setPublisher({ id: item.id, name: item.label })}
            />
          {/if}
        </div>

        <!-- Language -->
        <div class="space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">Language</span>
          <LanguageCombobox
            bind:value={languageValue}
            onchange={(code) => filters.setLanguage(code || null)}
            class="h-8 text-sm"
          />
        </div>

        <!-- Tags -->
        <div class="space-y-1.5 sm:col-span-2">
          <div class="flex items-center gap-2">
            <span class="text-xs font-medium text-muted-foreground">Tags</span>
            {#if filters.activeTags.length > 1}
              <div class="flex rounded border border-input text-[10px]">
                <button
                  class="px-1.5 py-0.5 transition-colors {filters.activeTagMatch === 'any'
                    ? 'bg-primary text-primary-foreground'
                    : 'hover:bg-accent'}"
                  onclick={() => filters.setTagMatch('any')}
                >
                  Any
                </button>
                <button
                  class="px-1.5 py-0.5 transition-colors {filters.activeTagMatch === 'all'
                    ? 'bg-primary text-primary-foreground'
                    : 'hover:bg-accent'}"
                  onclick={() => filters.setTagMatch('all')}
                >
                  All
                </button>
              </div>
            {/if}
          </div>
          {#if filters.activeTags.length > 0}
            <div class="flex flex-wrap gap-1 pb-1">
              {#each filters.activeTags as tag (tag.id)}
                <span
                  class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
                >
                  {tag.name}
                  <button
                    onclick={() => filters.removeTag(tag.id)}
                    class="rounded-full p-0.5 transition-colors hover:bg-primary/20"
                    aria-label="Remove tag {tag.name}"
                  >
                    <svg class="size-2.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M18 6 6 18" /><path d="m6 6 12 12" />
                    </svg>
                  </button>
                </span>
              {/each}
            </div>
          {/if}
          <AutocompleteInput
            placeholder="Search tags..."
            search={searchTags}
            onselect={(item) =>
              filters.addTag({ id: item.id, name: item.label, category: item.sublabel ?? null })}
          />
        </div>

        <!-- Year Range -->
        <div class="space-y-1.5">
          <span class="text-xs font-medium text-muted-foreground">Year Range</span>
          <div class="flex items-center gap-1.5">
            <Input
              type="number"
              placeholder="From"
              value={yearMinInput}
              oninput={(e) => {
                yearMinEditing = (e.target as HTMLInputElement).value;
              }}
              onblur={commitYearMin}
              onkeydown={(e) => {
                if (e.key === 'Enter') commitYearMin();
              }}
              class="h-8 text-sm"
            />
            <span class="text-xs text-muted-foreground">&ndash;</span>
            <Input
              type="number"
              placeholder="To"
              value={yearMaxInput}
              oninput={(e) => {
                yearMaxEditing = (e.target as HTMLInputElement).value;
              }}
              onblur={commitYearMax}
              onkeydown={(e) => {
                if (e.key === 'Enter') commitYearMax();
              }}
              class="h-8 text-sm"
            />
          </div>
        </div>

        <!-- Identifier Lookup -->
        <div class="space-y-1.5 sm:col-span-2 lg:col-span-1">
          <span class="text-xs font-medium text-muted-foreground">Identifier</span>
          <div class="flex gap-1.5">
            <select
              class="h-8 rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none"
              value={idType}
              onchange={(e) => {
                idTypeEditing = (e.target as HTMLSelectElement).value as IdentifierType;
                if (idValue.trim()) commitIdentifier();
              }}
            >
              {#each identifierTypes as opt (opt.value)}
                <option value={opt.value}>{opt.label}</option>
              {/each}
            </select>
            <Input
              type="text"
              placeholder="Value..."
              value={idValue}
              oninput={(e) => {
                idValueEditing = (e.target as HTMLInputElement).value;
              }}
              onblur={commitIdentifier}
              onkeydown={(e) => {
                if (e.key === 'Enter') commitIdentifier();
              }}
              class="h-8 flex-1 text-sm"
            />
            {#if filters.activeIdentifierType}
              <button
                onclick={() => {
                  idValueEditing = null;
                  idTypeEditing = null;
                  filters.clearIdentifier();
                }}
                class="shrink-0 rounded p-1 text-muted-foreground hover:text-foreground"
                aria-label="Clear identifier filter"
              >
                <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M18 6 6 18" /><path d="m6 6 12 12" />
                </svg>
              </button>
            {/if}
          </div>
        </div>

        <!-- Boolean toggles -->
        <div class="space-y-1.5">
          <label for="filter-trusted" class="text-xs font-medium text-muted-foreground">Trusted</label>
          <select
            id="filter-trusted"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={boolSelectValue(filters.activeTrusted)}
            onchange={(e) => filters.setTrusted(parseBoolSelect((e.target as HTMLSelectElement).value))}
          >
            {#each boolOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="space-y-1.5">
          <label for="filter-locked" class="text-xs font-medium text-muted-foreground">Locked</label>
          <select
            id="filter-locked"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={boolSelectValue(filters.activeLocked)}
            onchange={(e) => filters.setLocked(parseBoolSelect((e.target as HTMLSelectElement).value))}
          >
            {#each boolOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="space-y-1.5">
          <label for="filter-has-cover" class="text-xs font-medium text-muted-foreground">Has Cover</label>
          <select
            id="filter-has-cover"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={boolSelectValue(filters.activeHasCover)}
            onchange={(e) =>
              filters.setHasCover(parseBoolSelect((e.target as HTMLSelectElement).value))}
          >
            {#each boolOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="space-y-1.5">
          <label for="filter-has-description" class="text-xs font-medium text-muted-foreground">Has Description</label>
          <select
            id="filter-has-description"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={boolSelectValue(filters.activeHasDescription)}
            onchange={(e) =>
              filters.setHasDescription(parseBoolSelect((e.target as HTMLSelectElement).value))}
          >
            {#each boolOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="space-y-1.5">
          <label for="filter-has-identifiers" class="text-xs font-medium text-muted-foreground">Has Identifiers</label>
          <select
            id="filter-has-identifiers"
            class="h-8 w-full rounded-md border border-input bg-background px-2 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            value={boolSelectValue(filters.activeHasIdentifiers)}
            onchange={(e) =>
              filters.setHasIdentifiers(parseBoolSelect((e.target as HTMLSelectElement).value))}
          >
            {#each boolOptions as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>
      </div>

      {#if filters.hasActiveFilters}
        <div class="mt-3 flex justify-end border-t border-border pt-3">
          <Button
            variant="ghost"
            size="sm"
            onclick={() => {
              filters.clearFilters();
              yearMinEditing = null;
              yearMaxEditing = null;
              idValueEditing = null;
              idTypeEditing = null;
            }}
          >
            Clear All Filters
          </Button>
        </div>
      {/if}
    </div>
{/if}
