<script lang="ts">
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import { api, type PaginatedTags } from '$lib/api/index.js';
  import type { SortOrder } from '$lib/api/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import Pagination from '$lib/components/library/Pagination.svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';

  const PER_PAGE = 30;
  const DEBOUNCE_MS = 300;

  type SortOption = { label: string; field: 'name' | 'category' | 'book_count'; order: SortOrder };

  const sortOptions: SortOption[] = [
    { label: 'Most Books', field: 'book_count', order: 'desc' },
    { label: 'Fewest Books', field: 'book_count', order: 'asc' },
    { label: 'Name A\u2013Z', field: 'name', order: 'asc' },
    { label: 'Name Z\u2013A', field: 'name', order: 'desc' },
    { label: 'Category A\u2013Z', field: 'category', order: 'asc' },
    { label: 'Category Z\u2013A', field: 'category', order: 'desc' }
  ];

  // Restore state from URL search params
  const _params = page.url.searchParams;
  const _initPage = Math.max(1, parseInt(_params.get('page') || '1', 10) || 1);
  const _initQuery = _params.get('q') || '';
  const _initCategory = _params.get('category') || '';
  const _initSort = _params.get('sort') as 'name' | 'category' | 'book_count' | null;
  const _initOrder = _params.get('order') as SortOrder | null;
  const _initSortIdx =
    _initSort && _initOrder
      ? sortOptions.findIndex((o) => o.field === _initSort && o.order === _initOrder)
      : -1;

  let searchInput = $state(_initQuery);
  let activeQuery = $state(_initQuery);
  let activeCategory = $state(_initCategory);
  let sortIndex = $state(_initSortIdx >= 0 ? _initSortIdx : 0);
  let currentPage = $state(_initPage);
  let loading = $state(true);
  let data = $state<PaginatedTags | null>(null);
  let error = $state<string | null>(null);
  let categories = $state<string[]>([]);
  let retryCount = $state(0);
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  let activeSortBy = $state<'name' | 'category' | 'book_count'>(
    _initSort || sortOptions[0].field
  );
  let activeSortOrder = $state<SortOrder>(_initOrder || sortOptions[0].order);

  // Fetch categories on mount
  $effect(() => {
    api.tags.categories().then((result) => {
      categories = result;
    });
  });

  function handleSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    searchInput = value;
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      activeQuery = value.trim();
    }, DEBOUNCE_MS);
  }

  function handleSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      clearTimeout(debounceTimer);
      activeQuery = searchInput.trim();
    }
  }

  const columnSortField: Record<string, 'name' | 'category' | 'book_count'> = {
    name: 'name',
    category: 'category',
    book_count: 'book_count'
  };

  function handleSortChange(e: Event) {
    sortIndex = Number((e.target as HTMLSelectElement).value);
    activeSortBy = sortOptions[sortIndex].field;
    activeSortOrder = sortOptions[sortIndex].order;
  }

  function handleCategoryChange(e: Event) {
    activeCategory = (e.target as HTMLSelectElement).value;
  }

  function handleHeaderSort(columnId: string) {
    const field = columnSortField[columnId];
    if (!field) return;
    if (activeSortBy === field) {
      activeSortOrder = activeSortOrder === 'asc' ? 'desc' : 'asc';
    } else {
      activeSortBy = field;
      activeSortOrder = 'asc';
    }
    const idx = sortOptions.findIndex(
      (o) => o.field === activeSortBy && o.order === activeSortOrder
    );
    sortIndex = idx >= 0 ? idx : -1;
  }

  function sortIndicator(columnId: string): string {
    const field = columnSortField[columnId];
    if (!field || activeSortBy !== field) return '';
    return activeSortOrder === 'asc' ? ' \u2191' : ' \u2193';
  }

  function handlePageChange(p: number) {
    currentPage = p;
  }

  // Reset to page 1 when search, category, or sort changes
  let _prevQuery = _initQuery;
  let _prevCategory = _initCategory;
  let _prevSortBy = _initSort || sortOptions[0].field;
  let _prevSortOrder = _initOrder || sortOptions[0].order;

  $effect(() => {
    const q = activeQuery;
    const cat = activeCategory;
    const sb = activeSortBy;
    const so = activeSortOrder;

    const changed =
      q !== _prevQuery ||
      cat !== _prevCategory ||
      sb !== _prevSortBy ||
      so !== _prevSortOrder;

    _prevQuery = q;
    _prevCategory = cat;
    _prevSortBy = sb;
    _prevSortOrder = so;

    if (changed) {
      currentPage = 1;
    }
  });

  // Fetch tags when params change
  $effect(() => {
    const p = currentPage;
    const field = activeSortBy;
    const order = activeSortOrder;
    const q = activeQuery;
    const cat = activeCategory;
    void retryCount;

    loading = true;
    error = null;

    api.tags
      .list({
        page: p,
        per_page: PER_PAGE,
        sort_by: field,
        sort_order: order,
        q: q || undefined,
        category: cat || undefined
      })
      .then((result) => {
        data = result;
      })
      .catch((err) => {
        error = err instanceof Error ? err.message : 'Failed to load tags';
      })
      .finally(() => {
        loading = false;
      });
  });

  // Sync state to URL for back-navigation support
  $effect(() => {
    const params = new SvelteURLSearchParams();
    if (currentPage > 1) params.set('page', String(currentPage));
    if (activeQuery) params.set('q', activeQuery);
    if (activeCategory) params.set('category', activeCategory);
    if (activeSortBy !== sortOptions[0].field || activeSortOrder !== sortOptions[0].order) {
      params.set('sort', activeSortBy);
      params.set('order', activeSortOrder);
    }

    const search = params.toString();
    const url = search ? `/tags?${search}` : '/tags';
    goto(url, { replaceState: true, noScroll: true, keepFocus: true });
  });

  function tagHref(tag: { id: string; name: string }): string {
    return `/?tags=${tag.id}&tag_names=${encodeURIComponent(tag.name)}`;
  }

  const skeletonRows = Array.from({ length: 10 }, (_, i) => i);
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-3xl font-bold tracking-tight">Tags</h1>
    <p class="text-muted-foreground">Browse all tags in your library</p>
  </div>

  <!-- Controls bar -->
  <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
    <div class="flex flex-1 gap-2">
      <div class="relative w-full sm:max-w-xs">
        <svg
          class="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <Input
          type="search"
          placeholder="Search tags..."
          value={searchInput}
          oninput={handleSearchInput}
          onkeydown={handleSearchKeydown}
          class="pl-9"
        />
      </div>

      {#if categories.length > 0}
        <select
          class="h-9 rounded-md border border-input bg-background px-3 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
          value={activeCategory}
          onchange={handleCategoryChange}
        >
          <option value="">All categories</option>
          {#each categories as cat (cat)}
            <option value={cat}>{cat}</option>
          {/each}
        </select>
      {/if}
    </div>

    <select
      class="h-9 rounded-md border border-input bg-background px-3 text-sm shadow-xs focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
      value={sortIndex}
      onchange={handleSortChange}
    >
      {#each sortOptions as option, i (option.field + option.order)}
        <option value={i}>{option.label}</option>
      {/each}
    </select>
  </div>

  <!-- Content area -->
  {#if loading}
    <div class="overflow-hidden rounded-lg border border-border">
      {#each skeletonRows as id (id)}
        <div class="flex items-center gap-4 border-b border-border px-4 py-3 last:border-b-0">
          <div class="h-4 w-40 animate-pulse rounded bg-muted"></div>
          <div class="hidden h-4 w-24 animate-pulse rounded bg-muted sm:block"></div>
          <div class="ml-auto h-4 w-8 animate-pulse rounded bg-muted"></div>
        </div>
      {/each}
    </div>
  {:else if error}
    <div
      class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
    >
      <div class="text-center">
        <p class="text-destructive">{error}</p>
        <Button variant="outline" class="mt-4" onclick={() => retryCount++}>
          Retry
        </Button>
      </div>
    </div>
  {:else if data && data.items.length > 0}
    <p class="text-sm text-muted-foreground">
      {data.total}
      {data.total === 1 ? 'tag' : 'tags'}
    </p>

    <div class="overflow-hidden rounded-lg border border-border">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-border bg-muted/50">
            <th
              class="cursor-pointer select-none px-4 py-2.5 text-left font-medium text-muted-foreground hover:text-foreground"
              onclick={() => handleHeaderSort('name')}
            >
              Name<span class="ml-1 text-xs">{sortIndicator('name')}</span>
            </th>
            <th
              class="hidden cursor-pointer select-none px-4 py-2.5 text-left font-medium text-muted-foreground hover:text-foreground sm:table-cell"
              onclick={() => handleHeaderSort('category')}
            >
              Category<span class="ml-1 text-xs">{sortIndicator('category')}</span>
            </th>
            <th
              class="cursor-pointer select-none px-4 py-2.5 text-right font-medium text-muted-foreground hover:text-foreground"
              onclick={() => handleHeaderSort('book_count')}
            >
              Books<span class="ml-1 text-xs">{sortIndicator('book_count')}</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {#each data.items as tag (tag.id)}
            <tr class="border-b border-border transition-colors last:border-b-0 hover:bg-muted/30">
              <td class="px-4 py-2.5">
                <a
                  href={tagHref(tag)}
                  class="font-medium text-foreground transition-colors hover:text-primary"
                >
                  {tag.name}
                </a>
              </td>
              <td class="hidden px-4 py-2.5 text-muted-foreground sm:table-cell">
                {#if tag.category}
                  {tag.category}
                {:else}
                  <span class="text-muted-foreground/40">&mdash;</span>
                {/if}
              </td>
              <td class="px-4 py-2.5 text-right text-muted-foreground">{tag.book_count}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <Pagination page={data.page} totalPages={data.total_pages} onPageChange={handlePageChange} />
  {:else}
    <div
      class="flex items-center justify-center rounded-lg border border-dashed border-border p-12"
    >
      <div class="text-center">
        {#if activeQuery || activeCategory}
          <svg
            class="mx-auto mb-3 size-10 text-muted-foreground/50"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="11" cy="11" r="8" />
            <path d="m21 21-4.3-4.3" />
          </svg>
          <p class="font-medium text-foreground">No tags found</p>
          <p class="mt-1 text-sm text-muted-foreground">No tags match your search.</p>
          <Button
            variant="outline"
            class="mt-4"
            onclick={() => {
              searchInput = '';
              activeQuery = '';
              activeCategory = '';
            }}
          >
            Clear filters
          </Button>
        {:else}
          <svg
            class="mx-auto mb-3 size-10 text-muted-foreground/50"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="m15 5 6.3 6.3a2.4 2.4 0 0 1 0 3.4L17 19" />
            <path
              d="M9.586 5.586A2 2 0 0 0 8.172 5H3a1 1 0 0 0-1 1v5.172a2 2 0 0 0 .586 1.414L8.29 18.29a2.426 2.426 0 0 0 3.42 0l3.58-3.58a2.426 2.426 0 0 0 0-3.42z"
            />
            <circle cx="6.5" cy="9.5" r=".5" fill="currentColor" />
          </svg>
          <p class="font-medium text-foreground">No tags yet</p>
          <p class="mt-1 text-sm text-muted-foreground">
            Tags will appear here once books are imported.
          </p>
        {/if}
      </div>
    </div>
  {/if}
</div>
