<script lang="ts">
  import { SvelteSet, SvelteURLSearchParams } from 'svelte/reactivity';
  import { api, ApiError, type PaginatedBooks, type SelectionSpec } from '$lib/api/index.js';
  import { navCounts } from '$lib/stores/nav-counts.svelte.js';
  import type { SortField, SortOrder } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
  import { filters } from '$lib/stores/filters.svelte.js';
  import BookCard from '$lib/components/library/BookCard.svelte';
  import BookListView from '$lib/components/library/BookListView.svelte';
  import Pagination from '$lib/components/library/Pagination.svelte';
  import BatchEditPanel from '$lib/components/library/BatchEditPanel.svelte';
  import FilterPanel from '$lib/components/library/FilterPanel.svelte';
  import DslSearchBox from '$lib/components/library/DslSearchBox.svelte';
  import {
    findTokenByFieldAndQuery,
    replaceTokenInQuery
  } from '$lib/components/library/search-dsl.js';
  import ActiveTaskPanel from '$lib/components/tasks/ActiveTaskPanel.svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';

  const PER_PAGE = 24;
  const DEBOUNCE_MS = 300;
  const VIEW_STORAGE_KEY = 'archivis-library-view';

  type ViewMode = 'grid' | 'list';
  type SortOption = { label: string; field: SortField; order: SortOrder };

  const sortOptions: SortOption[] = [
    { label: 'Recently Added', field: 'added_at', order: 'desc' },
    { label: 'Title A\u2013Z', field: 'title', order: 'asc' },
    { label: 'Title Z\u2013A', field: 'title', order: 'desc' },
    { label: 'Highest Rated', field: 'rating', order: 'desc' },
    { label: 'Relevance', field: 'relevance', order: 'asc' }
  ];

  function loadViewPreference(): ViewMode {
    if (typeof localStorage === 'undefined') return 'grid';
    const stored = localStorage.getItem(VIEW_STORAGE_KEY);
    return stored === 'list' ? 'list' : 'grid';
  }

  // Restore state from URL search params (supports browser back-navigation)
  const _params = page.url.searchParams;
  const _initPage = Math.max(1, parseInt(_params.get('page') || '1', 10) || 1);
  const _initQuery = _params.get('q') || '';
  const _initSort = _params.get('sort') as SortField | null;
  const _initOrder = _params.get('order') as SortOrder | null;
  const _initSortIdx =
    _initSort && _initOrder
      ? sortOptions.findIndex((o) => o.field === _initSort && o.order === _initOrder)
      : -1;

  // Restore filters from URL
  filters.fromURLParams(_params);

  let viewMode = $state<ViewMode>(loadViewPreference());
  let searchInput = $state(_initQuery);
  let activeQuery = $state(_initQuery);
  let sortIndex = $state(_initSortIdx >= 0 ? _initSortIdx : 0);
  let currentPage = $state(_initPage);
  let loading = $state(true);
  let data = $state<PaginatedBooks | null>(null);
  let error = $state<string | null>(null);
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  let activeSortBy = $state<SortField>(_initSort || sortOptions[0].field);
  let activeSortOrder = $state<SortOrder>(_initOrder || sortOptions[0].order);
  let filtersOpen = $state(false);

  // --- Refresh metadata state ---
  let refreshMetadataDialogOpen = $state(false);
  let refreshingAllMetadata = $state(false);
  let refreshMetadataError = $state<string | null>(null);
  let refreshMetadataCompleted = $state(0);
  let refreshMetadataTotal = $state(0);
  let refreshMetadataEventSources: EventSource[] = [];

  // --- Selection mode state ---
  type SelectionMode = 'none' | 'page' | 'scope';
  let selectionMode = $state<SelectionMode>('none');
  let selectedIds = new SvelteSet<string>();
  let lastClickedId = $state<string | null>(null);
  let batchEditOpen = $state(false);

  // Scope mode state
  let scopeToken = $state('');
  let scopeExcludedIds = new SvelteSet<string>();
  let scopeTotalMatching = $state(0);
  let promotingScope = $state(false);

  // Background bulk task tracking (async 202 responses)
  let bulkTaskIds = $state<string[]>([]);

  const selectionActive = $derived(selectionMode !== 'none');

  const selectedCount = $derived(
    selectionMode === 'scope'
      ? scopeTotalMatching - scopeExcludedIds.size
      : selectedIds.size
  );

  /** Whether all items on the current page are selected (triggers promote banner). */
  const allPageSelected = $derived(
    selectionMode === 'page' &&
      data !== null &&
      data.items.length > 0 &&
      data.items.every((b) => selectedIds.has(b.id))
  );

  /** Whether there are more matching books beyond this page (scope promotion is useful). */
  const canPromoteToScope = $derived(
    allPageSelected && data !== null && data.total > data.items.length
  );

  /** Build a `SelectionSpec` for batch API calls. */
  function buildSelectionSpec(): SelectionSpec {
    if (selectionMode === 'scope') {
      return {
        mode: 'scope',
        scope_token: scopeToken,
        excluded_ids: Array.from(scopeExcludedIds)
      };
    }
    return { mode: 'ids', ids: Array.from(selectedIds) };
  }

  const includeParam = $derived(viewMode === 'list' ? 'authors,series,files' : 'authors,files');

  const showRefreshMetadata = $derived(
    filters.activeStatus === 'needs_review' || filters.activeStatus === 'unidentified'
  );

  function setViewMode(mode: ViewMode) {
    viewMode = mode;
    localStorage.setItem(VIEW_STORAGE_KEY, mode);
  }

  function handleSearchInput(val: string) {
    searchInput = val;
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      activeQuery = val.trim();
    }, DEBOUNCE_MS);
  }

  function handleSearchSubmit() {
    clearTimeout(debounceTimer);
    activeQuery = searchInput.trim();
  }

  // Track whether the user has made an explicit sort choice during the current search.
  // When false and a search is active, we auto-apply relevance sort.
  let userExplicitSort = $state(false);

  function handleSortChange(e: Event) {
    sortIndex = Number((e.target as HTMLSelectElement).value);
    activeSortBy = sortOptions[sortIndex].field;
    activeSortOrder = sortOptions[sortIndex].order;
    userExplicitSort = true;
  }

  function handlePageChange(page: number) {
    currentPage = page;
  }

  function handleListSort(field: SortField, order: SortOrder) {
    activeSortBy = field;
    activeSortOrder = order;
    userExplicitSort = true;
    // Sync dropdown if it matches a preset
    const idx = sortOptions.findIndex((o) => o.field === field && o.order === order);
    if (idx >= 0) sortIndex = idx;
  }

  // --- Selection mode ---

  function toggleSelectionMode() {
    if (selectionMode !== 'none') {
      exitSelectionMode();
    } else {
      selectionMode = 'page';
    }
  }

  function exitSelectionMode() {
    selectionMode = 'none';
    selectedIds.clear();
    lastClickedId = null;
    scopeToken = '';
    scopeExcludedIds.clear();
    scopeTotalMatching = 0;
  }

  function handleBookSelect(bookId: string, event?: MouseEvent) {
    if (selectionMode === 'scope') {
      // In scope mode, clicking toggles exclusion
      if (scopeExcludedIds.has(bookId)) {
        scopeExcludedIds.delete(bookId);
      } else {
        scopeExcludedIds.add(bookId);
      }
      return;
    }

    if (event?.shiftKey && lastClickedId && data) {
      // Range selection: select all books between lastClickedId and bookId
      const items = data.items;
      const lastIdx = items.findIndex((b) => b.id === lastClickedId);
      const currentIdx = items.findIndex((b) => b.id === bookId);
      if (lastIdx >= 0 && currentIdx >= 0) {
        const start = Math.min(lastIdx, currentIdx);
        const end = Math.max(lastIdx, currentIdx);
        for (let i = start; i <= end; i++) {
          selectedIds.add(items[i].id);
        }
      }
    } else {
      // Toggle single selection
      if (selectedIds.has(bookId)) {
        selectedIds.delete(bookId);
      } else {
        selectedIds.add(bookId);
      }
    }

    lastClickedId = bookId;
  }

  function handleCardSelect(bookId: string, event: MouseEvent) {
    handleBookSelect(bookId, event);
  }

  function selectAll() {
    if (!data) return;
    for (const book of data.items) {
      selectedIds.add(book.id);
    }
  }

  function deselectAll() {
    if (selectionMode === 'scope') {
      // In scope mode, "deselect all" fully exits selection
      exitSelectionMode();
    } else {
      selectedIds.clear();
    }
  }

  async function promoteToScope() {
    promotingScope = true;
    try {
      const filterState = filters.toFilterState(activeQuery);
      const result = await api.books.issueSelectionScope({ filters: filterState });
      selectionMode = 'scope';
      scopeToken = result.scope_token;
      scopeTotalMatching = result.matching_count;
      scopeExcludedIds.clear();
      selectedIds.clear();
    } catch (err) {
      error =
        err instanceof ApiError
          ? err.message
          : err instanceof Error
            ? err.message
            : 'Failed to issue selection scope';
    } finally {
      promotingScope = false;
    }
  }

  function handleBatchApply() {
    // Refresh the book list and exit selection mode
    exitSelectionMode();
    // Trigger refresh by reassigning currentPage
    currentPage = currentPage;
  }

  function handleBatchAsync(taskIds: string[]) {
    bulkTaskIds = [...bulkTaskIds, ...taskIds];
  }

  function handleBulkTasksDone() {
    // All background bulk tasks finished — refresh list and counts
    currentPage = currentPage;
    navCounts.invalidate();
  }

  // Clear scope when filters or search change (scope was frozen at promotion time).
  // We track the snapshot that was active when scope was entered, and compare on each tick.
  let _scopeEntryFilterSnapshot = '';
  let _scopeEntryQuery = '';

  // Capture the snapshot when entering scope mode
  $effect(() => {
    if (selectionMode === 'scope' && _scopeEntryFilterSnapshot === '') {
      _scopeEntryFilterSnapshot = filters.snapshotKey();
      _scopeEntryQuery = activeQuery;
    } else if (selectionMode !== 'scope') {
      _scopeEntryFilterSnapshot = '';
      _scopeEntryQuery = '';
    }
  });

  // Detect if filters/query changed while in scope mode
  $effect(() => {
    if (selectionMode !== 'scope') return;
    const snap = filters.snapshotKey();
    const q = activeQuery;
    if (
      _scopeEntryFilterSnapshot !== '' &&
      (snap !== _scopeEntryFilterSnapshot || q !== _scopeEntryQuery)
    ) {
      // Filters changed while in scope mode — fully exit selection
      exitSelectionMode();
    }
  });

  // Reset to page 1 when search, sort, or filters change (but not on initial mount)
  let _prevQuery = _initQuery;
  let _prevSortBy: SortField = _initSort || sortOptions[0].field;
  let _prevSortOrder: SortOrder = _initOrder || sortOptions[0].order;
  let _prevFilterSnapshot = filters.snapshotKey();

  $effect(() => {
    const q = activeQuery;
    const sb = activeSortBy;
    const so = activeSortOrder;
    const snap = filters.snapshotKey();

    const changed =
      q !== _prevQuery || sb !== _prevSortBy || so !== _prevSortOrder || snap !== _prevFilterSnapshot;

    _prevQuery = q;
    _prevSortBy = sb;
    _prevSortOrder = so;
    _prevFilterSnapshot = snap;

    if (changed) {
      currentPage = 1;
    }
  });

  // Sync sort dropdown with implicit relevance when searching
  const RELEVANCE_SORT_IDX = sortOptions.findIndex((o) => o.field === 'relevance');

  // Whether relevance was auto-applied (not user-chosen) — used to decide revert on clear
  let autoRelevanceApplied = false;

  $effect(() => {
    const q = activeQuery;

    if (q && !userExplicitSort && sortIndex !== RELEVANCE_SORT_IDX) {
      // Search became active, user hasn't chosen a sort — auto-switch to relevance
      autoRelevanceApplied = true;
      sortIndex = RELEVANCE_SORT_IDX;
      activeSortBy = sortOptions[RELEVANCE_SORT_IDX].field;
      activeSortOrder = sortOptions[RELEVANCE_SORT_IDX].order;
    } else if (!q) {
      // Search cleared — revert if relevance was auto-applied, otherwise keep user's choice
      if (autoRelevanceApplied && sortIndex === RELEVANCE_SORT_IDX) {
        sortIndex = 0;
        activeSortBy = sortOptions[0].field;
        activeSortOrder = sortOptions[0].order;
      }
      autoRelevanceApplied = false;
      userExplicitSort = false;
    }
  });

  // Stale-response protection for book fetches
  let fetchSeqId = 0;

  // Fetch books when query params change
  $effect(() => {
    const pg = currentPage;
    const field = activeSortBy;
    const order = activeSortOrder;
    const q = activeQuery;
    const include = includeParam;
    // Access filter snapshot to track reactivity (void to suppress unused lint)
    void filters.snapshotKey();
    const filterParams = filters.toListParams();

    loading = true;
    error = null;

    const thisSeqId = ++fetchSeqId;

    api.books
      .list({
        page: pg,
        per_page: PER_PAGE,
        sort_by: field,
        sort_order: order,
        q: q || undefined,
        include,
        ...filterParams
      })
      .then((result) => {
        if (thisSeqId !== fetchSeqId) return;
        data = result;
        warningsSuppressed = false;
      })
      .catch((err) => {
        if (thisSeqId !== fetchSeqId) return;
        error = err instanceof Error ? err.message : 'Failed to load books';
      })
      .finally(() => {
        if (thisSeqId !== fetchSeqId) return;
        loading = false;
      });
  });

  // Sync state to URL for back-navigation support
  $effect(() => {
    const params = new SvelteURLSearchParams();
    if (currentPage > 1) params.set('page', String(currentPage));
    if (activeQuery) params.set('q', activeQuery);
    if (activeSortBy !== sortOptions[0].field || activeSortOrder !== sortOptions[0].order) {
      params.set('sort', activeSortBy);
      params.set('order', activeSortOrder);
    }
    filters.toURLParams(params);

    const search = params.toString();
    const url = search ? `/?${search}` : '/';
    goto(url, { replaceState: true, noScroll: true, keepFocus: true });
  });

  // Cleanup SSE on unmount
  $effect(() => {
    return () => {
      for (const es of refreshMetadataEventSources) {
        es.close();
      }
      refreshMetadataEventSources = [];
    };
  });

  // --- Refresh Metadata ---

  async function handleRefreshAllMetadata() {
    refreshingAllMetadata = true;
    refreshMetadataError = null;
    refreshMetadataCompleted = 0;
    refreshMetadataDialogOpen = false;

    try {
      const response = await api.resolution.refreshAll();
      refreshMetadataTotal = response.count;

      if (response.count === 0) {
        refreshingAllMetadata = false;
        refreshMetadataError = 'No books found needing metadata refresh.';
        return;
      }

      // Subscribe to SSE for each task
      for (const taskId of response.task_ids) {
        subscribeToRefreshTask(taskId);
      }
    } catch (err) {
      refreshMetadataError =
        err instanceof Error ? err.message : 'Failed to start metadata refresh';
      refreshingAllMetadata = false;
    }
  }

  function subscribeToRefreshTask(taskId: string) {
    const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);
    refreshMetadataEventSources.push(es);

    es.addEventListener('task:complete', () => {
      refreshMetadataCompleted += 1;
      es.close();
      removeRefreshEventSource(es);
      checkRefreshAllDone();
    });

    es.addEventListener('task:error', () => {
      refreshMetadataCompleted += 1;
      es.close();
      removeRefreshEventSource(es);
      checkRefreshAllDone();
    });

    es.onerror = () => {
      refreshMetadataCompleted += 1;
      es.close();
      removeRefreshEventSource(es);
      checkRefreshAllDone();
    };
  }

  function removeRefreshEventSource(es: EventSource) {
    refreshMetadataEventSources = refreshMetadataEventSources.filter((e) => e !== es);
  }

  function checkRefreshAllDone() {
    if (refreshMetadataCompleted >= refreshMetadataTotal) {
      refreshingAllMetadata = false;
      // Refresh the book list
      currentPage = currentPage;
      navCounts.invalidate();
    }
  }

  function dismissRefreshMetadata() {
    refreshMetadataCompleted = 0;
    refreshMetadataTotal = 0;
    refreshMetadataError = null;
  }

  // --- Filter chip labels ---

  const resolutionStateLabels: Record<string, string> = {
    pending: 'Pending',
    running: 'Running',
    done: 'Done',
    failed: 'Failed'
  };

  const resolutionOutcomeLabels: Record<string, string> = {
    confirmed: 'Confirmed',
    enriched: 'Enriched',
    disputed: 'Disputed',
    ambiguous: 'Ambiguous',
    unmatched: 'Unmatched'
  };

  const statusLabels: Record<string, string> = {
    identified: 'Identified',
    needs_review: 'Needs Review',
    unidentified: 'Unidentified'
  };

  const identifierTypeLabels: Record<string, string> = {
    isbn: 'ISBN',
    isbn10: 'ISBN-10',
    isbn13: 'ISBN-13',
    asin: 'ASIN',
    google_books: 'Google Books',
    open_library: 'Open Library',
    hardcover: 'Hardcover',
    lccn: 'LCCN'
  };

  const skeletonIds = Array.from({ length: 12 }, (_, i) => i);

  // --- Search warnings ---

  let searchWarnings = $derived(data?.search_warnings ?? []);

  // Suppressed after a warning pick until the triggered fetch resolves with fresh data.
  let warningsSuppressed = $state(false);
  let showSearchWarnings = $derived(
    searchInput.trim() === activeQuery && !warningsSuppressed
  );

  function handleWarningPick(field: string, query: string, id: string, name: string) {
    // 1. ID-precise resolution via filter store (source of truth)
    switch (field) {
      case 'author':
        filters.setAuthor({ id, name });
        break;
      case 'series':
        filters.setSeries({ id, name });
        break;
      case 'publisher':
        filters.setPublisher({ id, name });
        break;
      case 'tag':
        filters.addTag({ id, name, category: null });
        break;
    }

    // 2. Cosmetic: remove the ambiguous token so it doesn't re-trigger the warning
    const token = findTokenByFieldAndQuery(searchInput, field, query);
    if (token) {
      const { newQuery } = replaceTokenInQuery(searchInput, token, '');
      searchInput = newQuery.trim();
      clearTimeout(debounceTimer);
      activeQuery = searchInput;
    }

    // 3. Suppress old warnings until the next accepted fetch resolves.
    warningsSuppressed = true;
  }
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-3xl font-bold tracking-tight">Library</h1>
    <p class="text-muted-foreground">Your e-book collection</p>
  </div>

  <!-- Controls bar -->
  <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
    <div class="flex min-w-0 flex-1 items-center gap-2">
      <div class="min-w-0 flex-1">
        <DslSearchBox
          bind:value={searchInput}
          placeholder="Search books..."
          warnings={searchWarnings}
          showWarnings={showSearchWarnings}
          onWarningPick={handleWarningPick}
          onchange={handleSearchInput}
          onsubmit={handleSearchSubmit}
        />
      </div>
      <Button
        variant="outline"
        size="sm"
        onclick={() => (filtersOpen = !filtersOpen)}
        class="flex-shrink-0 gap-1.5"
      >
        <svg
          class="size-4"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="4" y1="21" x2="4" y2="14" />
          <line x1="4" y1="10" x2="4" y2="3" />
          <line x1="12" y1="21" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12" y2="3" />
          <line x1="20" y1="21" x2="20" y2="16" />
          <line x1="20" y1="12" x2="20" y2="3" />
          <line x1="1" y1="14" x2="7" y2="14" />
          <line x1="9" y1="8" x2="15" y2="8" />
          <line x1="17" y1="16" x2="23" y2="16" />
        </svg>
        Filters
        {#if filters.activeFilterCount > 0}
          <span
            class="inline-flex size-5 items-center justify-center rounded-full bg-primary text-[10px] font-semibold text-primary-foreground"
          >
            {filters.activeFilterCount}
          </span>
        {/if}
      </Button>
    </div>

    <div class="flex flex-shrink-0 items-center gap-2">
      {#if showRefreshMetadata && data && data.total > 0}
        <Button
          size="sm"
          variant="outline"
          onclick={() => (refreshMetadataDialogOpen = true)}
          disabled={refreshingAllMetadata}
        >
          {#if refreshingAllMetadata}
            <svg
              class="size-4 animate-spin"
              xmlns="http://www.w3.org/2000/svg"
              fill="none"
              viewBox="0 0 24 24"
            >
              <circle
                class="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                stroke-width="4"
              ></circle>
              <path
                class="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              ></path>
            </svg>
            Refreshing...
          {:else}
            <svg
              class="size-4"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M21 12a9 9 0 1 1-3.2-6.9" />
              <path d="M21 3v6h-6" />
            </svg>
            Refresh Metadata
          {/if}
        </Button>
      {/if}

      <!-- Selection mode toggle -->
      <Button
        size="sm"
        variant={selectionActive ? 'default' : 'outline'}
        onclick={toggleSelectionMode}
      >
        {#if selectionActive}
          <svg
            class="size-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M18 6 6 18" />
            <path d="m6 6 12 12" />
          </svg>
          Exit Select
        {:else}
          <svg
            class="size-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <polyline points="9 11 12 14 22 4" />
            <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
          </svg>
          Select
        {/if}
      </Button>

      <!-- View toggle -->
      <div class="flex rounded-md border border-input shadow-xs">
        <Button
          variant={viewMode === 'grid' ? 'default' : 'ghost'}
          size="icon-sm"
          onclick={() => setViewMode('grid')}
          aria-label="Grid view"
          class="rounded-r-none border-0 shadow-none"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class="size-4"
          >
            <rect x="3" y="3" width="7" height="7" />
            <rect x="14" y="3" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" />
            <rect x="14" y="14" width="7" height="7" />
          </svg>
        </Button>
        <Button
          variant={viewMode === 'list' ? 'default' : 'ghost'}
          size="icon-sm"
          onclick={() => setViewMode('list')}
          aria-label="List view"
          class="rounded-l-none border-0 shadow-none"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class="size-4"
          >
            <line x1="3" y1="6" x2="21" y2="6" />
            <line x1="3" y1="12" x2="21" y2="12" />
            <line x1="3" y1="18" x2="21" y2="18" />
          </svg>
        </Button>
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
  </div>

  <FilterPanel open={filtersOpen} />

  <!-- Selection toolbar -->
  {#if selectionActive}
    <div class="space-y-0">
      <div class="flex items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-2">
        <span class="text-sm font-medium">
          {#if selectionMode === 'scope'}
            {selectedCount.toLocaleString()} of {scopeTotalMatching.toLocaleString()} books selected
          {:else}
            {selectedCount} selected
          {/if}
        </span>
        <div class="flex items-center gap-1.5">
          {#if selectionMode === 'page'}
            <Button size="sm" variant="ghost" class="h-7 text-xs" onclick={selectAll}>
              Select All
            </Button>
          {/if}
          <Button
            size="sm"
            variant="ghost"
            class="h-7 text-xs"
            onclick={deselectAll}
            disabled={selectedCount === 0}
          >
            {selectionMode === 'scope' ? 'Exit Selection' : 'Deselect All'}
          </Button>
        </div>
        <div class="flex-1"></div>
        <Button size="sm" onclick={() => (batchEditOpen = true)} disabled={selectedCount === 0}>
          <svg
            class="size-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
            <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
          </svg>
          Edit Selected
        </Button>
      </div>

      <!-- Promote banner: shown when all page items are selected and more exist -->
      {#if canPromoteToScope}
        <div class="flex items-center justify-center gap-2 rounded-b-lg border border-t-0 border-primary/20 bg-primary/5 px-4 py-1.5 text-sm">
          <span class="text-muted-foreground">
            All {data?.items.length} books on this page are selected.
          </span>
          <button
            class="font-medium text-primary underline-offset-4 hover:underline disabled:opacity-50"
            onclick={promoteToScope}
            disabled={promotingScope}
          >
            {#if promotingScope}
              Selecting...
            {:else}
              Select all {data?.total.toLocaleString()} matching books
            {/if}
          </button>
        </div>
      {/if}
    </div>
  {/if}

  <!-- Refresh metadata progress -->
  {#if refreshingAllMetadata || (refreshMetadataTotal > 0 && refreshMetadataCompleted > 0)}
    <div class="rounded-lg border border-border p-3">
      <div class="flex items-center justify-between text-sm">
        <span class="font-medium">
          {#if refreshingAllMetadata}
            Refreshing metadata...
          {:else}
            Metadata refresh complete
          {/if}
        </span>
        <div class="flex items-center gap-2">
          <span class="text-xs text-muted-foreground">
            {refreshMetadataCompleted} / {refreshMetadataTotal}
          </span>
          {#if !refreshingAllMetadata}
            <Button
              variant="ghost"
              size="icon-sm"
              class="size-6"
              onclick={dismissRefreshMetadata}
              aria-label="Dismiss"
            >
              <svg
                class="size-3"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M18 6 6 18" />
                <path d="m6 6 12 12" />
              </svg>
            </Button>
          {/if}
        </div>
      </div>
      <div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
        <div
          class="h-full rounded-full bg-primary transition-all duration-300"
          style="width: {refreshMetadataTotal > 0
            ? (refreshMetadataCompleted / refreshMetadataTotal) * 100
            : 0}%"
        ></div>
      </div>
    </div>
  {/if}

  {#if refreshMetadataError}
    <div
      class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {refreshMetadataError}
    </div>
  {/if}

  <!-- Background bulk task progress -->
  {#if bulkTaskIds.length > 0}
    <ActiveTaskPanel bind:taskIds={bulkTaskIds} onAllDone={handleBulkTasksDone} />
  {/if}

  <!-- Active filter chips -->
  {#if filters.hasActiveFilters}
    <div class="flex flex-wrap items-center gap-2">
      {#if filters.activeFormat}
        {@const chipLabel = filters.activeFormat.toUpperCase()}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {chipLabel}
          <button
            onclick={() => filters.setFormat(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove format filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeStatus}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {statusLabels[filters.activeStatus] ?? filters.activeStatus}
          <button
            onclick={() => filters.setStatus(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove status filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeAuthor}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Author: {filters.activeAuthor.name}
          <button
            onclick={() => filters.setAuthor(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove author filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeSeries}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Series: {filters.activeSeries.name}
          <button
            onclick={() => filters.setSeries(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove series filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activePublisher}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Publisher: {filters.activePublisher.name}
          <button
            onclick={() => filters.setPublisher(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove publisher filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#each filters.activeTags as tag (tag.id)}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Tag: {tag.name}
          <button
            onclick={() => filters.removeTag(tag.id)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove tag {tag.name}"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/each}
      {#if filters.activeResolutionState}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Resolution: {resolutionStateLabels[filters.activeResolutionState]}
          <button
            onclick={() => filters.setResolutionState(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove resolution state filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeResolutionOutcome}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Outcome: {resolutionOutcomeLabels[filters.activeResolutionOutcome]}
          <button
            onclick={() => filters.setResolutionOutcome(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove resolution outcome filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeTrusted !== null}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {filters.activeTrusted ? 'Trusted' : 'Not Trusted'}
          <button
            onclick={() => filters.setTrusted(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove trusted filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeLocked !== null}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {filters.activeLocked ? 'Locked' : 'Not Locked'}
          <button
            onclick={() => filters.setLocked(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove locked filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeLanguage}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Language: {filters.activeLanguage}
          <button
            onclick={() => filters.setLanguage(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove language filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeYearMin !== null || filters.activeYearMax !== null}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Year: {filters.activeYearMin ?? '...'}&ndash;{filters.activeYearMax ?? '...'}
          <button
            onclick={() => {
              filters.setYearMin(null);
              filters.setYearMax(null);
            }}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove year range filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeHasCover !== null}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {filters.activeHasCover ? 'Has Cover' : 'No Cover'}
          <button
            onclick={() => filters.setHasCover(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove cover filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeHasDescription !== null}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {filters.activeHasDescription ? 'Has Description' : 'No Description'}
          <button
            onclick={() => filters.setHasDescription(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove description filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeHasIdentifiers !== null}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {filters.activeHasIdentifiers ? 'Has Identifiers' : 'No Identifiers'}
          <button
            onclick={() => filters.setHasIdentifiers(null)}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove identifiers filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if filters.activeIdentifierType && filters.activeIdentifierValue}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          {identifierTypeLabels[filters.activeIdentifierType] ?? filters.activeIdentifierType}: {filters.activeIdentifierValue}
          <button
            onclick={() => filters.clearIdentifier()}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove identifier filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      {#if !filters.activeIdentifierType && filters.activeIdentifierValue}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary"
        >
          Identifier: {filters.activeIdentifierValue}
          <button
            onclick={() => filters.clearIdentifier()}
            class="ml-0.5 rounded-full p-0.5 transition-colors hover:bg-primary/20"
            aria-label="Remove identifier filter"
          >
            <svg class="size-3" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </span>
      {/if}
      <button
        onclick={() => {
          filters.clearFilters();
          searchInput = '';
          activeQuery = '';
        }}
        class="text-xs text-muted-foreground transition-colors hover:text-foreground"
      >
        Clear all
      </button>
    </div>
  {/if}

  <!-- Search warnings are now rendered inside DslSearchBox -->

  <!-- Content area -->
  {#if loading}
    {#if viewMode === 'grid'}
      <!-- Skeleton grid -->
      <div class="grid grid-cols-[repeat(auto-fill,minmax(150px,200px))] justify-center gap-4">
        {#each skeletonIds as id (id)}
          <div>
            <div class="aspect-[2/3] w-full animate-pulse rounded-lg bg-muted"></div>
            <div class="mt-1.5 space-y-1 px-0.5">
              <div class="h-4 w-3/4 animate-pulse rounded bg-muted"></div>
              <div class="h-3 w-1/2 animate-pulse rounded bg-muted"></div>
            </div>
          </div>
        {/each}
      </div>
    {:else}
      <!-- Skeleton list -->
      <div class="space-y-0 overflow-hidden rounded-lg border border-border">
        {#each skeletonIds.slice(0, 8) as id (id)}
          <div class="flex items-center gap-3 border-b border-border px-3 py-2 last:border-b-0">
            <div class="h-10 w-7 animate-pulse rounded bg-muted"></div>
            <div class="h-4 w-48 animate-pulse rounded bg-muted"></div>
            <div class="h-4 w-32 animate-pulse rounded bg-muted"></div>
            <div class="h-4 w-24 animate-pulse rounded bg-muted"></div>
          </div>
        {/each}
      </div>
    {/if}
  {:else if error}
    <div
      class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
    >
      <div class="text-center">
        <p class="text-destructive">{error}</p>
        <Button variant="outline" class="mt-4" onclick={() => (currentPage = currentPage)}>
          Retry
        </Button>
      </div>
    </div>
  {:else if data && data.items.length > 0}
    {#if viewMode === 'grid'}
      <div class="grid grid-cols-[repeat(auto-fill,minmax(150px,200px))] justify-center gap-4">
        {#each data.items as book (book.id)}
          <BookCard
            {book}
            selectionMode={selectionActive}
            selected={selectionMode === 'scope'
              ? !scopeExcludedIds.has(book.id)
              : selectedIds.has(book.id)}
            onselect={handleCardSelect}
          />
        {/each}
      </div>
    {:else}
      {@const effectiveSelectedIds =
        selectionMode === 'scope'
          ? new Set(data.items.filter((b) => !scopeExcludedIds.has(b.id)).map((b) => b.id))
          : selectedIds}
      <BookListView
        books={data.items}
        sortBy={activeSortBy}
        sortOrder={activeSortOrder}
        onSort={handleListSort}
        selectionMode={selectionActive}
        selectedIds={effectiveSelectedIds}
        onselect={(bookId, event) => handleBookSelect(bookId, event)}
      />
    {/if}

    <Pagination page={data.page} totalPages={data.total_pages} onPageChange={handlePageChange} />
  {:else}
    <!-- Empty state -->
    <div
      class="flex items-center justify-center rounded-lg border border-dashed border-border p-12"
    >
      <div class="text-center">
        {#if activeQuery || filters.hasActiveFilters}
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
          <p class="font-medium text-foreground">No books found</p>
          <p class="mt-1 text-sm text-muted-foreground">
            No books match your current {activeQuery && filters.hasActiveFilters
              ? 'search and filters'
              : activeQuery
                ? 'search'
                : 'filters'}.
          </p>
          <Button
            variant="outline"
            class="mt-4"
            onclick={() => {
              searchInput = '';
              activeQuery = '';
              filters.clearFilters();
            }}
          >
            Clear {activeQuery && filters.hasActiveFilters
              ? 'search & filters'
              : activeQuery
                ? 'search'
                : 'filters'}
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
            <path
              d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"
            />
          </svg>
          <p class="font-medium text-foreground">Welcome to Archivis</p>
          <p class="mt-1 text-sm text-muted-foreground">
            Your library is empty. Import your first e-books to get started.
          </p>
          <Button class="mt-4" href="/import">Import books</Button>
        {/if}
      </div>
    </div>
  {/if}
</div>

<!-- Refresh metadata confirmation dialog -->
<AlertDialog.Root bind:open={refreshMetadataDialogOpen}>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>Refresh Metadata</AlertDialog.Title>
      <AlertDialog.Description>
        Refresh metadata for all books that currently need review or enrichment. This will query
        configured providers like Open Library and Hardcover.
      </AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
      <AlertDialog.Action onclick={handleRefreshAllMetadata}>Refresh Metadata</AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>

<!-- Batch edit panel -->
{#if selectionActive && selectedCount > 0}
  <BatchEditPanel
    selection={buildSelectionSpec()}
    {selectedCount}
    bind:open={batchEditOpen}
    onclose={() => (batchEditOpen = false)}
    onapply={handleBatchApply}
    onasync={handleBatchAsync}
  />
{/if}
