<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import type {
    BookDetail,
    CandidateResponse,
    DuplicateLinkResponse,
    TaskProgressEvent,
    TaskStatus
  } from '$lib/api/index.js';
  import { api, ApiError } from '$lib/api/index.js';
  import AutocompleteInput from '$lib/components/library/AutocompleteInput.svelte';
  import BookEditForm from '$lib/components/library/BookEditForm.svelte';
  import CandidateReview from '$lib/components/library/CandidateReview.svelte';
  import CoverImage from '$lib/components/library/CoverImage.svelte';
  import CoverUploadDialog from '$lib/components/library/CoverUploadDialog.svelte';
  import IdentifierEditor from '$lib/components/library/IdentifierEditor.svelte';
  import MergeDialog from '$lib/components/library/MergeDialog.svelte';
  import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { navCounts } from '$lib/stores/nav-counts.svelte.js';
  import { formatFileSize, placeholderHue } from '$lib/utils.js';

  let book = $state<BookDetail | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let notFound = $state(false);
  let coverError = $state(false);
  let editing = $state(false);
  let deleteDialogOpen = $state(false);
  let deleting = $state(false);
  let deleteError = $state<string | null>(null);
  let coverCacheBust = $state(0);
  let coverDialogOpen = $state(false);
  let metadataAction = $state<'lock' | 'unlock' | null>(null);
  let metadataActionError = $state<string | null>(null);
  let descriptionExpanded = $state(false);

  // --- Metadata refresh state ---
  let refreshingMetadata = $state(false);
  let refreshError = $state<string | null>(null);
  let refreshProgress = $state<TaskProgressEvent | null>(null);
  let refreshEventSource: EventSource | null = null;
  let candidates = $state<CandidateResponse[]>([]);
  let candidatesError = $state<string | null>(null);
  let candidatesExpanded = $state(false);
  let trustingMetadata = $state(false);
  let rejectingAllFromBanner = $state(false);

  // --- ISBN content scan state ---
  let scanning = $state(false);
  let scanError = $state<string | null>(null);
  let scanProgress = $state<TaskProgressEvent | null>(null);
  let scanEventSource: EventSource | null = null;

  // --- Duplicate state ---
  let duplicateLinks = $state<DuplicateLinkResponse[]>([]);
  let dismissingDupId = $state<string | null>(null);
  let mergeDialogOpen = $state(false);
  let selectedDupLink = $state<DuplicateLinkResponse | null>(null);
  let flagDialogOpen = $state(false);
  let flagging = $state(false);
  let flagError = $state<string | null>(null);

  const bookId = $derived(page.params.id ?? '');
  const hue = $derived(placeholderHue(bookId));
  const coverVersion = $derived(
    coverCacheBust || (book?.updated_at ? Date.parse(book.updated_at) : 0)
  );
  const coverUrl = $derived(
    `/api/books/${bookId}/cover?size=lg${coverVersion ? `&t=${coverVersion}` : ''}`
  );

  const authors = $derived(book?.authors ?? []);
  const primaryAuthors = $derived(authors.filter((a) => a.role === 'author'));
  const otherContributors = $derived(authors.filter((a) => a.role !== 'author'));

  const pendingCandidates = $derived(candidates.filter((c) => c.status === 'pending'));
  const needsTruncation = $derived(
    book?.description != null &&
      (book.description.length > 300 || (book.description.match(/\n/g)?.length ?? 0) >= 4)
  );

  function fetchBook() {
    loading = true;
    error = null;
    notFound = false;
    coverError = false;

    api.books
      .get(bookId)
      .then((result) => {
        book = result;
      })
      .catch((err) => {
        if (err instanceof ApiError && err.isNotFound) {
          notFound = true;
        } else {
          error = err instanceof Error ? err.message : 'Failed to load book';
        }
      })
      .finally(() => {
        loading = false;
      });
  }

  $effect(() => {
    void bookId;
    fetchBook();
  });

  function enterEditMode() {
    editing = true;
  }

  function cancelEdit() {
    editing = false;
  }

  function handleSave(updated: BookDetail) {
    const wasTrusted = book?.metadata_user_trusted;
    book = updated;
    editing = false;

    // Trust via edit-form rejects all pending candidates server-side;
    // core-identity edits supersede them. Sync local state.
    if (updated.metadata_user_trusted && !wasTrusted) {
      candidates = [];
      candidatesExpanded = false;
      candidatesError = null;
    } else {
      loadCandidates();
    }
  }

  async function handleDelete() {
    deleting = true;
    deleteError = null;
    try {
      await api.books.delete(bookId);
      deleteDialogOpen = false;
      goto('/');
    } catch (err) {
      deleteError = err instanceof Error ? err.message : 'Failed to delete book';
    } finally {
      deleting = false;
    }
  }

  function handleCoverUpdate(updated: BookDetail) {
    book = updated;
    coverError = false;
    coverCacheBust = Date.now();
  }

  async function handleToggleMetadataLock() {
    if (!book) return;

    metadataAction = book.metadata_locked ? 'unlock' : 'lock';
    metadataActionError = null;
    try {
      book = book.metadata_locked
        ? await api.books.unlockMetadata(bookId)
        : await api.books.lockMetadata(bookId);
    } catch (err) {
      metadataActionError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to update metadata lock';
    } finally {
      metadataAction = null;
    }
  }

  function handleProtectionChange(updated: BookDetail) {
    book = updated;
  }

  // --- Metadata refresh ---

  async function handleRefreshMetadata() {
    refreshingMetadata = true;
    refreshError = null;
    refreshProgress = null;

    try {
      const response = await api.books.refreshMetadata(bookId);
      subscribeToRefreshProgress(response.task_id);
    } catch (err) {
      refreshError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to start metadata refresh';
      refreshingMetadata = false;
    }
  }

  function subscribeToRefreshProgress(taskId: string) {
    if (refreshEventSource) {
      refreshEventSource.close();
    }

    const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);
    refreshEventSource = es;

    es.addEventListener('task:progress', (event: MessageEvent) => {
      try {
        refreshProgress = JSON.parse(event.data) as TaskProgressEvent;
      } catch {
        // Ignore malformed events
      }
    });

    es.addEventListener('task:complete', (event: MessageEvent) => {
      try {
        const data = JSON.parse(event.data) as TaskProgressEvent;
        refreshProgress = { ...data, status: 'completed' as TaskStatus, progress: 100 };
      } catch {
        // Ignore malformed events
      }
      es.close();
      refreshEventSource = null;
      refreshingMetadata = false;
      // Reload book and candidates
      fetchBook();
      loadCandidates().then(() => {
        if (candidates.length > 0) {
          candidatesExpanded = true;
        }
      });
      navCounts.invalidate();
    });

    es.addEventListener('task:error', (event: MessageEvent) => {
      try {
        const data = JSON.parse(event.data) as TaskProgressEvent;
        refreshProgress = { ...data, status: 'failed' as TaskStatus };
        refreshError = data.error ?? 'Metadata refresh failed';
      } catch {
        refreshError = 'Metadata refresh failed';
      }
      es.close();
      refreshEventSource = null;
      refreshingMetadata = false;
    });

    es.onerror = () => {
      es.close();
      refreshEventSource = null;
      refreshingMetadata = false;
    };
  }

  // --- ISBN Content Scan ---

  async function handleScanIsbn() {
    scanning = true;
    scanError = null;
    scanProgress = null;

    try {
      const response = await api.isbnScan.scanBook(bookId);
      subscribeToScanProgress(response.task_id);
    } catch (err) {
      scanError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to start ISBN scan';
      scanning = false;
    }
  }

  function subscribeToScanProgress(taskId: string) {
    if (scanEventSource) {
      scanEventSource.close();
    }

    const es = new EventSource(`/api/tasks/${encodeURIComponent(taskId)}/progress`);
    scanEventSource = es;

    es.addEventListener('task:progress', (event: MessageEvent) => {
      try {
        scanProgress = JSON.parse(event.data) as TaskProgressEvent;
      } catch {
        // Ignore malformed events
      }
    });

    es.addEventListener('task:complete', (event: MessageEvent) => {
      try {
        const data = JSON.parse(event.data) as TaskProgressEvent;
        scanProgress = { ...data, status: 'completed' as TaskStatus, progress: 100 };
      } catch {
        // Ignore malformed events
      }
      es.close();
      scanEventSource = null;
      scanning = false;
      // Reload book to pick up newly found identifiers
      fetchBook();
      navCounts.invalidate();
    });

    es.addEventListener('task:error', (event: MessageEvent) => {
      try {
        const data = JSON.parse(event.data) as TaskProgressEvent;
        scanProgress = { ...data, status: 'failed' as TaskStatus };
        scanError = data.error ?? 'ISBN scan failed';
      } catch {
        scanError = 'ISBN scan failed';
      }
      es.close();
      scanEventSource = null;
      scanning = false;
    });

    es.onerror = () => {
      es.close();
      scanEventSource = null;
      scanning = false;
    };
  }

  async function loadCandidates() {
    candidatesError = null;
    try {
      candidates = await api.resolution.candidates(bookId);
    } catch (err) {
      candidatesError = err instanceof Error ? err.message : 'Failed to load candidates';
    }
  }

  function handleCandidateApplied(updated: BookDetail) {
    book = updated;
    coverError = false;
    coverCacheBust = Date.now();
    loadCandidates().then(() => {
      if (pendingCandidates.length === 0) {
        candidatesExpanded = false;
      }
    });
  }

  function handleCandidateRejected(candidateId: string) {
    candidatesError = null;
    candidates = candidates.map((c) =>
      c.id === candidateId ? { ...c, status: 'rejected' as const } : c
    );
    fetchBook();
  }

  function handleCandidateUndone(updated: BookDetail) {
    book = updated;
    loadCandidates();
  }

  async function handleTrustMetadata() {
    trustingMetadata = true;
    candidatesError = null;
    try {
      const updated = await api.resolution.trustMetadata(bookId);
      book = updated;
      candidates = [];
      candidatesExpanded = false;
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        candidatesError = 'Cannot trust while a metadata refresh is in progress';
      } else {
        candidatesError = err instanceof Error ? err.message : 'Failed to trust metadata';
      }
    } finally {
      trustingMetadata = false;
    }
  }

  async function handleUntrustMetadata() {
    trustingMetadata = true;
    candidatesError = null;
    try {
      const updated = await api.resolution.untrustMetadata(bookId);
      book = updated;
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        candidatesError = 'Cannot untrust while a metadata refresh is in progress';
      } else {
        candidatesError = err instanceof Error ? err.message : 'Failed to untrust metadata';
      }
    } finally {
      trustingMetadata = false;
    }
  }

  async function handleRejectAllFromBanner() {
    if (pendingCandidates.length === 0) return;
    rejectingAllFromBanner = true;
    candidatesError = null;
    try {
      const ids = pendingCandidates.map((c) => c.id);
      const updated = await api.resolution.rejectCandidates(bookId, ids);
      book = updated;
      candidates = candidates.map((c) =>
        ids.includes(c.id) ? { ...c, status: 'rejected' as const } : c
      );
    } catch (err) {
      candidatesError = err instanceof Error ? err.message : 'Failed to reject candidates';
    } finally {
      rejectingAllFromBanner = false;
    }
  }

  function handleIdentifierUpdate(updated: BookDetail) {
    book = updated;
  }

  // --- Duplicate functions ---

  async function loadDuplicateLinks() {
    try {
      duplicateLinks = await api.duplicates.forBook(bookId);
    } catch {
      // Silently ignore — duplicates are supplemental info
      duplicateLinks = [];
    }
  }

  async function handleDismissDuplicate(linkId: string) {
    dismissingDupId = linkId;
    try {
      await api.duplicates.dismiss(linkId);
      duplicateLinks = duplicateLinks.filter((l) => l.id !== linkId);
    } catch {
      // Silently ignore
    } finally {
      dismissingDupId = null;
    }
  }

  function openDupMergeDialog(link: DuplicateLinkResponse) {
    selectedDupLink = link;
    mergeDialogOpen = true;
  }

  function handleDupMergeComplete(merged: BookDetail) {
    mergeDialogOpen = false;
    selectedDupLink = null;
    goto(`/books/${merged.id}`);
  }

  function handleDupMergeCancel() {
    mergeDialogOpen = false;
    selectedDupLink = null;
  }

  function otherBookInLink(link: DuplicateLinkResponse): { id: string; title: string } {
    if (link.book_a.id === bookId) {
      return { id: link.book_b.id, title: link.book_b.title };
    }
    return { id: link.book_a.id, title: link.book_a.title };
  }

  async function handleFlagDuplicate(item: { id: string; label: string }) {
    flagging = true;
    flagError = null;
    flagDialogOpen = false;
    try {
      const newLink = await api.duplicates.flag(bookId, item.id);
      duplicateLinks = [...duplicateLinks, newLink];
    } catch (err) {
      flagError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to flag duplicate';
    } finally {
      flagging = false;
    }
  }

  async function searchBooksForFlag(
    query: string
  ): Promise<{ id: string; label: string; sublabel?: string }[]> {
    try {
      const result = await api.books.list({ q: query, per_page: 10 });
      return result.items
        .filter((b) => b.id !== bookId)
        .map((b) => ({
          id: b.id,
          label: b.title,
          sublabel: b.authors?.map((a) => a.name).join(', ')
        }));
    } catch {
      return [];
    }
  }

  // Load candidates and duplicate links when book loads
  $effect(() => {
    if (book && !loading) {
      loadCandidates();
      loadDuplicateLinks();
    }

    return () => {
      // Cleanup SSE on unmount
      if (refreshEventSource) {
        refreshEventSource.close();
        refreshEventSource = null;
      }
      if (scanEventSource) {
        scanEventSource.close();
        scanEventSource = null;
      }
    };
  });

  function statusLabel(status: string): string {
    switch (status) {
      case 'identified':
        return 'Identified';
      case 'needs_review':
        return 'Needs Review';
      case 'unidentified':
        return 'Unidentified';
      default:
        return status;
    }
  }

  function primaryMetadataLabel(detail: BookDetail): string {
    // Active work always takes priority
    if (detail.resolution_state === 'running') {
      return 'Refreshing metadata';
    }

    // Trusted + identified takes priority over generic outcomes
    if (detail.metadata_user_trusted && detail.metadata_status === 'identified') {
      return 'Metadata trusted';
    }

    // Review states
    if (detail.metadata_status === 'needs_review' || detail.resolution_outcome === 'disputed') {
      return 'Review needed';
    }

    // Show the meaningful outcome when available
    // For `ambiguous`, only show "Review suggested" if there are actually pending candidates
    switch (detail.resolution_outcome) {
      case 'confirmed':
        return 'Metadata confirmed';
      case 'enriched':
        return 'Metadata enriched';
      case 'ambiguous':
        if (pendingCandidates.length > 0) return 'Review suggested';
        break;
      case 'unmatched':
        return 'No provider match';
      default:
        break;
    }

    // No outcome yet or resolved ambiguity — fall back to state
    switch (detail.resolution_state) {
      case 'pending':
        return 'Refresh pending';
      case 'failed':
        return 'Refresh failed';
      default:
        return statusLabel(detail.metadata_status);
    }
  }

  function primaryMetadataClasses(detail: BookDetail): string {
    // Active work always takes priority
    if (detail.resolution_state === 'running') {
      return 'bg-sky-100 text-sky-800 dark:bg-sky-900/30 dark:text-sky-400';
    }

    // Trusted + identified
    if (detail.metadata_user_trusted && detail.metadata_status === 'identified') {
      return 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/30 dark:text-emerald-400';
    }

    // Review states
    if (detail.metadata_status === 'needs_review' || detail.resolution_outcome === 'disputed') {
      return 'bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400';
    }

    // Show the meaningful outcome when available
    switch (detail.resolution_outcome) {
      case 'confirmed':
        return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400';
      case 'enriched':
        return 'bg-cyan-100 text-cyan-800 dark:bg-cyan-900/30 dark:text-cyan-400';
      case 'ambiguous':
        if (pendingCandidates.length > 0) {
          return 'bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-400';
        }
        break;
      case 'unmatched':
        return 'bg-slate-200 text-slate-700 dark:bg-slate-800 dark:text-slate-300';
      default:
        break;
    }

    // No outcome yet or resolved ambiguity — fall back to state
    switch (detail.resolution_state) {
      case 'pending':
        return 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400';
      case 'failed':
        return 'bg-destructive/10 text-destructive';
      default:
        return detail.metadata_status === 'unidentified'
          ? 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400'
          : 'bg-muted text-muted-foreground';
    }
  }

  function metadataWorkflowSummary(detail: BookDetail): string {
    switch (detail.resolution_state) {
      case 'running':
        return 'Archivis is checking metadata providers right now.';
      case 'pending':
        return 'A metadata refresh is queued for this book.';
      case 'failed':
        return 'The last metadata refresh failed. Run it again to retry.';
      default:
        break;
    }

    if (detail.metadata_status === 'needs_review' || detail.resolution_outcome === 'disputed') {
      return candidates.length > 0
        ? 'Review the candidates below before changing anything else.'
        : 'The latest metadata refresh needs manual review.';
    }

    switch (detail.resolution_outcome) {
      case 'confirmed':
        return 'The latest refresh confirmed the current metadata.';
      case 'enriched':
        return 'The latest refresh added better supporting metadata.';
      case 'ambiguous':
        if (pendingCandidates.length > 0) {
          return 'The latest refresh found possible matches, but they need review.';
        }
        return 'Use Refresh Metadata whenever identifiers or core book details change.';
      case 'unmatched':
        return 'The latest refresh finished without a usable provider match.';
      default:
        return 'Use Refresh Metadata whenever identifiers or core book details change.';
    }
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'long',
      day: 'numeric'
    });
  }

  function formatRating(rating: number): string {
    return `${rating.toFixed(1)} / 5`;
  }

  function formatFormatBadge(format: string): string {
    return format.toUpperCase();
  }

  const READABLE_FORMATS = new Set(['epub', 'pdf', 'mobi', 'azw3', 'fb2', 'cbz']);
  function isReadableFormat(format: string): boolean {
    return READABLE_FORMATS.has(format);
  }
</script>

<div class="mx-auto max-w-5xl space-y-6">
  <!-- Back navigation -->
  <button
    onclick={() => history.back()}
    class="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
  >
    <svg
      class="size-4"
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <path d="m15 18-6-6 6-6" />
    </svg>
    Back to Library
  </button>

  {#if loading}
    <!-- Loading skeleton -->
    <div class="grid gap-8 md:grid-cols-[280px_1fr]">
      <div class="aspect-[2/3] w-full animate-pulse rounded-lg bg-muted"></div>
      <div class="space-y-4">
        <div class="h-8 w-3/4 animate-pulse rounded bg-muted"></div>
        <div class="h-5 w-1/3 animate-pulse rounded bg-muted"></div>
        <div class="h-4 w-1/4 animate-pulse rounded bg-muted"></div>
        <div class="mt-6 space-y-2">
          <div class="h-4 w-full animate-pulse rounded bg-muted"></div>
          <div class="h-4 w-full animate-pulse rounded bg-muted"></div>
          <div class="h-4 w-2/3 animate-pulse rounded bg-muted"></div>
        </div>
      </div>
    </div>
  {:else if notFound}
    <div
      class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
    >
      <div class="text-center">
        <p class="text-lg font-medium text-destructive">Book not found</p>
        <p class="mt-1 text-sm text-muted-foreground">
          The book you're looking for doesn't exist or has been removed.
        </p>
        <Button variant="outline" class="mt-4" onclick={() => history.back()}
          >Back to Library</Button
        >
      </div>
    </div>
  {:else if error}
    <div
      class="flex items-center justify-center rounded-lg border border-dashed border-destructive/50 p-12"
    >
      <div class="text-center">
        <p class="text-destructive">{error}</p>
        <Button variant="outline" class="mt-4" onclick={fetchBook}>Retry</Button>
      </div>
    </div>
  {:else if book}
    <!-- Duplicate alert banners -->
    {#if duplicateLinks.length > 0}
      <div class="space-y-2">
        {#each duplicateLinks as dupLink (dupLink.id)}
          {@const other = otherBookInLink(dupLink)}
          <div
            class="flex items-center justify-between rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 dark:border-amber-800 dark:bg-amber-900/20"
          >
            <div class="flex items-center gap-2 text-sm">
              <svg
                class="size-4 flex-shrink-0 text-amber-600 dark:text-amber-400"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"
                />
                <line x1="12" x2="12" y1="9" y2="13" />
                <line x1="12" x2="12.01" y1="17" y2="17" />
              </svg>
              <span class="text-amber-800 dark:text-amber-300">
                This book may be a duplicate of
                <a
                  href="/books/{other.id}"
                  class="font-medium underline transition-colors hover:text-amber-900 dark:hover:text-amber-200"
                >
                  {other.title}
                </a>
              </span>
            </div>
            <div class="flex items-center gap-2">
              <Button
                size="sm"
                variant="outline"
                class="h-7 px-2 text-xs"
                disabled={dismissingDupId === dupLink.id}
                onclick={() => handleDismissDuplicate(dupLink.id)}
              >
                {#if dismissingDupId === dupLink.id}
                  Dismissing...
                {:else}
                  Dismiss
                {/if}
              </Button>
              <Button
                size="sm"
                class="h-7 px-2 text-xs"
                onclick={() => openDupMergeDialog(dupLink)}
              >
                Review
              </Button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if flagError}
      <div
        class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        {flagError}
      </div>
    {/if}

    <div class="grid gap-8 md:grid-cols-[280px_1fr]">
      <!-- Left column: Cover -->
      <div>
        <div
          class="group relative w-full overflow-hidden rounded-lg bg-muted shadow-md ring-1 ring-black/5 dark:ring-white/5"
        >
          {#if book.has_cover && !coverError}
            <CoverImage
              src={coverUrl}
              alt="Cover of {book.title}"
              fadeIn={true}
              onerror={() => (coverError = true)}
            />
            <div
              class="absolute inset-x-0 bottom-0 flex items-center justify-center bg-black/60 p-2 opacity-0 transition-opacity group-hover:opacity-100"
            >
              <button
                type="button"
                class="inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-white/20"
                onclick={() => (coverDialogOpen = true)}
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
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="17 8 12 3 7 8" />
                  <line x1="12" x2="12" y1="3" y2="15" />
                </svg>
                Change Cover
              </button>
            </div>
          {:else}
            <div
              class="flex aspect-[2/3] w-full flex-col items-center justify-center gap-3 p-6"
              style="background-color: hsl({hue}, 30%, 25%);"
            >
              <span class="line-clamp-6 text-center text-lg font-medium text-white/80">
                {book.title}
              </span>
              <button
                type="button"
                class="inline-flex items-center gap-1.5 rounded-md border border-white/30 px-3 py-1.5 text-xs font-medium text-white/90 transition-colors hover:bg-white/20"
                onclick={() => (coverDialogOpen = true)}
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
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="17 8 12 3 7 8" />
                  <line x1="12" x2="12" y1="3" y2="15" />
                </svg>
                Add Cover
              </button>
            </div>
          {/if}
        </div>
        <!-- Files section (below cover on desktop) -->
        {#if book.files.length > 0}
          <div class="mt-4 space-y-2">
            <h3 class="text-sm font-semibold text-muted-foreground">Files</h3>
            {#each book.files as file (file.id)}
              {@const fileFormatLabel = formatFormatBadge(file.format)}
              {@const fileSizeLabel = formatFileSize(file.file_size)}
              <div class="flex items-center gap-2 rounded-md border border-border p-2.5 text-sm">
                <div class="flex shrink-0 items-center gap-2">
                  <span
                    class="inline-flex rounded bg-primary/10 px-1.5 py-0.5 text-xs font-semibold text-primary"
                  >
                    {fileFormatLabel}
                  </span>
                </div>
                <div class="ml-auto flex min-w-0 flex-1 flex-wrap items-center justify-end gap-2">
                  {#if isReadableFormat(file.format)}
                    <a
                      href="/read/{book.id}/{file.id}"
                      target="_blank"
                      class="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90"
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
                        <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z" />
                        <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z" />
                      </svg>
                      Read
                    </a>
                  {/if}
                  <a
                    href="/api/books/{book.id}/files/{file.id}/download"
                    class="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted"
                    title={`Download (${fileSizeLabel})`}
                    aria-label={`Download ${fileFormatLabel} file (${fileSizeLabel})`}
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
                      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                      <polyline points="7 10 12 15 17 10" />
                      <line x1="12" x2="12" y1="15" y2="3" />
                    </svg>
                    Download
                  </a>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <!-- Right column: Metadata or Edit Form -->
      <div class="space-y-6">
        {#if editing}
          <BookEditForm
            {book}
            oncancel={cancelEdit}
            onsave={handleSave}
            oncoverupdate={handleCoverUpdate}
            metadata_provenance={book.metadata_provenance}
            onprotectionchange={handleProtectionChange}
          />
        {:else}
          <!-- Header: Title, status badges, actions -->
          <div>
            <div class="flex items-start justify-between gap-4">
              <h1 class="text-2xl font-bold tracking-tight md:text-3xl">{book.title}</h1>
              <div class="flex items-center gap-2">
                <Button size="sm" variant="outline" onclick={enterEditMode}>
                  <svg
                    class="size-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path
                      d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"
                    />
                    <path d="m15 5 4 4" />
                  </svg>
                  Edit
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onclick={() => (flagDialogOpen = !flagDialogOpen)}
                  disabled={flagging}
                  aria-label="Flag as duplicate"
                  title="Flag as duplicate"
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
                    <path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z" />
                    <line x1="4" x2="4" y1="22" y2="15" />
                  </svg>
                  Duplicate
                </Button>
                <Button size="sm" variant="destructive" onclick={() => (deleteDialogOpen = true)}>
                  <svg
                    class="size-4"
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
                  Delete
                </Button>
              </div>
            </div>
            {#if book.subtitle}
              <p class="text-lg text-muted-foreground">{book.subtitle}</p>
            {/if}
            {#if primaryAuthors.length > 0}
              <p class="mt-1 text-lg text-muted-foreground">
                {#each primaryAuthors as a, i (a.id)}<a
                    href="/authors/{a.id}"
                    class="transition-colors hover:text-foreground hover:underline">{a.name}</a
                  >{#if i < primaryAuthors.length - 1},&nbsp;{/if}{/each}
              </p>
            {:else if authors.length > 0}
              <p class="mt-1 text-lg text-muted-foreground">
                {#each authors as a, i (a.id)}<a
                    href="/authors/{a.id}"
                    class="transition-colors hover:text-foreground hover:underline">{a.name}</a
                  >{#if i < authors.length - 1},&nbsp;{/if}{/each}
              </p>
            {/if}
            <!-- Inline metadata status bar -->
            <div class="mt-3 space-y-2">
              <div class="flex flex-wrap items-center gap-3">
                <span
                  class="inline-flex rounded-full px-2.5 py-0.5 text-xs font-medium {primaryMetadataClasses(
                    book
                  )}"
                  title={metadataWorkflowSummary(book)}
                >
                  {primaryMetadataLabel(book)}
                </span>
                <button
                  type="button"
                  class="inline-flex items-center rounded p-1 transition-colors {book.metadata_locked
                    ? 'text-foreground'
                    : 'text-muted-foreground/50 hover:text-muted-foreground'} disabled:opacity-40"
                  title={book.metadata_locked
                    ? 'Metadata locked — click to unlock'
                    : 'Metadata unlocked — click to lock'}
                  disabled={metadataAction !== null || refreshingMetadata}
                  onclick={handleToggleMetadataLock}
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
                    {#if book.metadata_locked}
                      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
                      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
                    {:else}
                      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
                      <path d="M7 11V7a5 5 0 0 1 9.9-1" />
                    {/if}
                  </svg>
                </button>
                {#if !editing}
                  <button
                    type="button"
                    class="inline-flex items-center rounded p-1 transition-colors {book.metadata_user_trusted
                      ? 'text-emerald-600 dark:text-emerald-400'
                      : 'text-muted-foreground/50 hover:text-muted-foreground'} disabled:opacity-40"
                    title={book.metadata_user_trusted
                      ? 'Metadata trusted — click to remove trust'
                      : 'Click to trust this metadata'}
                    disabled={trustingMetadata ||
                      refreshingMetadata ||
                      book.resolution_state === 'running'}
                    onclick={book.metadata_user_trusted
                      ? handleUntrustMetadata
                      : handleTrustMetadata}
                  >
                    <svg
                      class="size-4"
                      viewBox="0 0 24 24"
                      fill={book.metadata_user_trusted ? 'currentColor' : 'none'}
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    >
                      <path
                        d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"
                      />
                      {#if book.metadata_user_trusted}
                        <path d="m9 12 2 2 4-4" stroke="white" fill="none" />
                      {/if}
                    </svg>
                  </button>
                {/if}
                <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
                  <span
                    >{Math.round(
                      (book.metadata_quality_score ?? book.ingest_quality_score) * 100
                    )}%</span
                  >
                  <div class="h-1.5 w-10 overflow-hidden rounded-full bg-muted">
                    <div
                      class="h-full rounded-full bg-primary transition-all"
                      style="width: {(book.metadata_quality_score ?? book.ingest_quality_score) *
                        100}%"
                    ></div>
                  </div>
                </div>
                <div class="ml-auto">
                  <Button
                    size="sm"
                    variant="ghost"
                    onclick={handleRefreshMetadata}
                    disabled={refreshingMetadata || metadataAction !== null}
                  >
                    {#if refreshingMetadata}
                      <svg
                        class="size-3 animate-spin"
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
                        class="size-3"
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
                </div>
              </div>

              {#if metadataActionError}
                <p class="text-xs text-destructive">{metadataActionError}</p>
              {/if}

              {#if refreshError}
                <p class="text-xs text-destructive">{refreshError}</p>
              {/if}

              {#if refreshingMetadata && refreshProgress}
                <div class="space-y-1">
                  <div class="flex items-center justify-between text-xs text-muted-foreground">
                    <span>Refreshing metadata...</span>
                    <span>{refreshProgress.progress}%</span>
                  </div>
                  <div class="h-1 w-full overflow-hidden rounded-full bg-muted">
                    <div
                      class="h-full rounded-full bg-primary transition-all duration-300"
                      style="width: {refreshProgress.progress}%"
                    ></div>
                  </div>
                  {#if refreshProgress.message}
                    <p class="text-xs text-muted-foreground">{refreshProgress.message}</p>
                  {/if}
                </div>
              {/if}

              {#if pendingCandidates.length > 0 || candidatesExpanded}
                <div
                  class="flex flex-col gap-2 rounded-lg border px-3 py-2 sm:flex-row sm:items-center sm:justify-between {pendingCandidates.length >
                    0 &&
                  (book.resolution_outcome === 'disputed' ||
                    book.metadata_status === 'needs_review')
                    ? 'border-amber-300 bg-amber-50/80 dark:border-amber-800 dark:bg-amber-900/20'
                    : 'border-border bg-muted/40'}"
                >
                  <p class="text-sm text-muted-foreground">
                    {#if pendingCandidates.length > 0}
                      {pendingCandidates.length} candidate{pendingCandidates.length === 1
                        ? ''
                        : 's'} ready for review
                    {:else}
                      All candidates dismissed
                    {/if}
                  </p>
                  <div class="flex items-center gap-2">
                    {#if !candidatesExpanded && pendingCandidates.length > 0}
                      <Button
                        size="sm"
                        class="h-7 px-2 text-xs"
                        onclick={() => (candidatesExpanded = true)}
                      >
                        Review Candidates
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        class="h-7 px-2 text-xs"
                        disabled={trustingMetadata || refreshingMetadata || book.resolution_state === 'running'}
                        onclick={handleTrustMetadata}
                      >
                        {trustingMetadata ? 'Trusting...' : 'Trust Metadata'}
                      </Button>
                    {:else if candidatesExpanded && pendingCandidates.length > 0}
                      {#if pendingCandidates.length > 1}
                        <Button
                          size="sm"
                          variant="destructive"
                          class="h-7 px-2 text-xs"
                          disabled={rejectingAllFromBanner}
                          onclick={handleRejectAllFromBanner}
                        >
                          {rejectingAllFromBanner ? 'Rejecting...' : 'Reject All'}
                        </Button>
                      {/if}
                      <Button
                        size="sm"
                        variant="outline"
                        class="h-7 px-2 text-xs"
                        disabled={trustingMetadata || refreshingMetadata || book.resolution_state === 'running'}
                        onclick={handleTrustMetadata}
                      >
                        {trustingMetadata ? 'Trusting...' : 'Trust Metadata'}
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        class="h-7 px-2 text-xs"
                        onclick={() => (candidatesExpanded = false)}
                      >
                        Hide
                      </Button>
                    {:else}
                      <Button
                        size="sm"
                        variant="outline"
                        class="h-7 px-2 text-xs"
                        disabled={trustingMetadata || refreshingMetadata || book.resolution_state === 'running'}
                        onclick={handleTrustMetadata}
                      >
                        {trustingMetadata ? 'Trusting...' : 'Trust Metadata'}
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        class="h-7 px-2 text-xs"
                        onclick={() => (candidatesExpanded = false)}
                      >
                        Hide
                      </Button>
                    {/if}
                  </div>
                </div>
              {/if}

              {#if candidatesExpanded && candidates.length > 0 && !editing}
                <CandidateReview
                  {book}
                  {candidates}
                  {coverVersion}
                  onapply={handleCandidateApplied}
                  onreject={handleCandidateRejected}
                  onundo={handleCandidateUndone}
                />
              {/if}
            </div>
          </div>

          {#if candidatesError}
            <p class="text-sm text-destructive">{candidatesError}</p>
          {/if}

          <!-- Description -->
          {#if book.description}
            <div>
              <h3 class="text-sm font-semibold text-muted-foreground">Description</h3>
              <p
                class="mt-1 whitespace-pre-line text-sm leading-relaxed {descriptionExpanded
                  ? ''
                  : 'line-clamp-4'}"
              >
                {book.description}
              </p>
              {#if needsTruncation}
                <button
                  type="button"
                  class="mt-1 text-xs font-medium text-primary hover:underline"
                  onclick={() => (descriptionExpanded = !descriptionExpanded)}
                >
                  {descriptionExpanded ? 'Show less' : 'Show more'}
                </button>
              {/if}
            </div>
          {/if}

          <!-- Details grid -->
          <div>
            <h3 class="text-sm font-semibold text-muted-foreground">Details</h3>
            <dl class="mt-2 grid grid-cols-2 gap-x-6 gap-y-3 text-sm">
              {#if book.publisher_name}
                <div>
                  <dt class="text-muted-foreground">Publisher</dt>
                  <dd class="font-medium">{book.publisher_name}</dd>
                </div>
              {/if}
              {#if book.publication_year != null}
                <div>
                  <dt class="text-muted-foreground">Published</dt>
                  <dd class="font-medium">{book.publication_year}</dd>
                </div>
              {/if}
              {#if book.language}
                <div>
                  <dt class="text-muted-foreground">Language</dt>
                  <dd class="font-medium">{book.language_label ?? book.language}</dd>
                </div>
              {/if}
              {#if book.page_count != null}
                <div>
                  <dt class="text-muted-foreground">Pages</dt>
                  <dd class="font-medium">{book.page_count}</dd>
                </div>
              {/if}
              {#if book.rating != null}
                <div>
                  <dt class="text-muted-foreground">Rating</dt>
                  <dd class="font-medium">{formatRating(book.rating)}</dd>
                </div>
              {/if}
              <div>
                <dt class="text-muted-foreground">Added</dt>
                <dd class="font-medium">{formatDate(book.added_at)}</dd>
              </div>

              {#if otherContributors.length > 0}
                <div>
                  <dt class="text-muted-foreground">Contributors</dt>
                  {#each otherContributors as contributor (contributor.id)}
                    <dd class="text-sm">
                      <a
                        href="/authors/{contributor.id}"
                        class="transition-colors hover:text-primary hover:underline"
                        >{contributor.name}</a
                      >
                      <span class="text-muted-foreground">({contributor.role})</span>
                    </dd>
                  {/each}
                </div>
              {/if}
            </dl>
          </div>

          <!-- Series -->
          {#if book.series.length > 0}
            <div>
              <h3 class="text-sm font-semibold text-muted-foreground">Series</h3>
              <div class="mt-1 space-y-1">
                {#each book.series as s (s.id)}
                  <p class="text-sm font-medium">
                    {#if s.position != null}
                      Book {Number.isInteger(s.position)
                        ? s.position.toString()
                        : s.position.toFixed(1)} in
                    {/if}
                    <a
                      href="/series/{s.id}"
                      class="transition-colors hover:text-primary hover:underline">{s.name}</a
                    >
                  </p>
                {/each}
              </div>
            </div>
          {/if}

          <!-- Tags -->
          {#if book.tags.length > 0}
            <div>
              <h3 class="text-sm font-semibold text-muted-foreground">Tags</h3>
              <div class="mt-1.5 flex flex-wrap gap-1.5">
                {#each book.tags as tag (tag.id)}
                  <span
                    class="inline-flex rounded-full border border-border bg-muted px-2.5 py-0.5 text-xs font-medium"
                  >
                    {#if tag.category}
                      <span class="mr-1 text-muted-foreground">{tag.category}:</span>
                    {/if}
                    {tag.name}
                  </span>
                {/each}
              </div>
            </div>
          {/if}

          <!-- Identifiers (editable) + ISBN scan -->
          <div class="space-y-4">
            <div class="flex items-center justify-between">
              <h3 class="text-sm font-semibold text-muted-foreground">Identifiers</h3>
              <Button
                size="sm"
                variant="outline"
                onclick={handleScanIsbn}
                disabled={scanning || refreshingMetadata}
                class="h-7 px-2 text-xs"
              >
                {#if scanning}
                  <svg
                    class="size-3 animate-spin"
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
                  Scanning...
                {:else}
                  <svg
                    class="size-3"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
                    <path d="M14 2v4a2 2 0 0 0 2 2h4" />
                    <circle cx="11.5" cy="14.5" r="2.5" />
                    <path d="M13.3 16.3 15 18" />
                  </svg>
                  Scan for ISBNs
                {/if}
              </Button>
            </div>

            {#if scanning && scanProgress}
              <div class="rounded-lg border border-border p-3">
                <div class="flex items-center justify-between text-sm">
                  <span class="font-medium">Scanning book content...</span>
                  <span class="text-xs text-muted-foreground">{scanProgress.progress}%</span>
                </div>
                <div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
                  <div
                    class="h-full rounded-full bg-primary transition-all duration-300"
                    style="width: {scanProgress.progress}%"
                  ></div>
                </div>
                {#if scanProgress.message}
                  <p class="mt-1.5 text-xs text-muted-foreground">{scanProgress.message}</p>
                {/if}
              </div>
            {/if}

            {#if scanError}
              <div
                class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive"
              >
                {scanError}
              </div>
            {/if}
          </div>

          <IdentifierEditor {book} onupdate={handleIdentifierUpdate} />
        {/if}
      </div>
    </div>
  {/if}
</div>

<!-- Cover upload dialog -->
{#if book}
  <CoverUploadDialog
    bookId={book.id}
    hasCover={book.has_cover}
    bind:open={coverDialogOpen}
    onupdate={handleCoverUpdate}
  />
{/if}

<!-- Delete confirmation dialog -->
{#if book}
  <AlertDialog.Root bind:open={deleteDialogOpen}>
    <AlertDialog.Content>
      <AlertDialog.Header>
        <AlertDialog.Title>Delete Book</AlertDialog.Title>
        <AlertDialog.Description>
          Are you sure you want to delete <strong>{book.title}</strong>? This will permanently
          remove the book, all associated files, and cover images. This action cannot be undone.
        </AlertDialog.Description>
      </AlertDialog.Header>
      {#if deleteError}
        <p class="text-sm text-destructive">{deleteError}</p>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel disabled={deleting}>Cancel</AlertDialog.Cancel>
        <AlertDialog.Action
          class="bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90"
          onclick={handleDelete}
          disabled={deleting}
        >
          {#if deleting}
            Deleting...
          {:else}
            Delete
          {/if}
        </AlertDialog.Action>
      </AlertDialog.Footer>
    </AlertDialog.Content>
  </AlertDialog.Root>
{/if}

<!-- Flag as duplicate dialog -->
{#if book}
  <Dialog.Root bind:open={flagDialogOpen}>
    <Dialog.Content class="sm:max-w-md">
      <Dialog.Header>
        <Dialog.Title>Flag as Duplicate</Dialog.Title>
        <Dialog.Description>
          Search for the other book that is a duplicate of <strong>{book.title}</strong>.
        </Dialog.Description>
      </Dialog.Header>
      <div class="py-2">
        <AutocompleteInput
          placeholder="Search for a book..."
          search={searchBooksForFlag}
          onselect={handleFlagDuplicate}
        />
      </div>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<!-- Merge dialog for duplicate review -->
{#if selectedDupLink}
  <MergeDialog
    link={selectedDupLink}
    bind:open={mergeDialogOpen}
    onmerge={handleDupMergeComplete}
    oncancel={handleDupMergeCancel}
  />
{/if}
