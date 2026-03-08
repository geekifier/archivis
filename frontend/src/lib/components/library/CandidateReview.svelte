<script lang="ts">
  import { untrack } from 'svelte';
  import { api, ApiError } from '$lib/api/index.js';
  import type { BookDetail, CandidateResponse } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import {
    scoreColor,
    formatScore,
    providerColorClass,
    hasChange,
    getExcludedFields,
    tierColorClass,
    tierLabel
  } from './candidate-utils.js';

  interface Props {
    book: BookDetail;
    candidates: CandidateResponse[];
    coverVersion?: number;
    onapply: (updated: BookDetail) => void;
    onreject: (candidateId: string) => void;
    onundo: (updated: BookDetail) => void;
  }

  let { book, candidates, coverVersion = 0, onapply, onreject, onundo }: Props = $props();

  const coverSuffix = $derived(coverVersion ? `&t=${coverVersion}` : '');
  const coverQuery = $derived(coverVersion ? `?t=${coverVersion}` : '');

  let applyingId = $state<string | null>(null);
  let rejectingId = $state<string | null>(null);
  let undoingId = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let coverCompare = $state<{ currentUrl: string | null; candidateUrl: string | null } | null>(null);
  let confirmApplyId = $state<string | null>(null);

  /** Per-candidate field selections: candidateId -> fieldName -> included. */
  let fieldSelections = $state<Record<string, Record<string, boolean>>>({});

  const pendingCandidates = $derived(candidates.filter((c) => c.status === 'pending'));
  const rejectedCandidates = $derived(candidates.filter((c) => c.status === 'rejected'));
  const appliedCandidates = $derived(candidates.filter((c) => c.status === 'applied'));
  const hasExistingApply = $derived(appliedCandidates.length > 0);

  function isAuthorRole(role: string | undefined | null): boolean {
    return !role || role === 'author';
  }

  function titleCase(s: string): string {
    return s.charAt(0).toUpperCase() + s.slice(1);
  }

  /** Initialize default selections for any new pending candidate. */
  $effect(() => {
    for (const candidate of pendingCandidates) {
      if (!untrack(() => fieldSelections[candidate.id])) {
        const sel: Record<string, boolean> = {};
        if (candidate.title != null) sel.title = true;
        if (candidate.subtitle != null) sel.subtitle = true;
        if (candidate.authors.length > 0) sel.authors = true;
        if (candidate.publication_year != null) sel.publication_year = true;
        if (candidate.isbn != null) sel.identifiers = true;
        if (candidate.series != null) sel.series = true;
        if (candidate.publisher != null) sel.publisher = true;
        if (candidate.description != null) sel.description = true;
        if (candidate.cover_url != null) sel.cover = true;
        fieldSelections[candidate.id] = sel;
      }
    }
  });

  function isFieldIncluded(candidateId: string, field: string): boolean {
    return fieldSelections[candidateId]?.[field] ?? true;
  }

  function toggleField(candidateId: string, field: string) {
    if (!fieldSelections[candidateId]) return;
    fieldSelections[candidateId][field] = !fieldSelections[candidateId][field];
  }

  function requestApply(candidateId: string) {
    if (hasExistingApply) {
      confirmApplyId = candidateId;
    } else {
      handleApply(candidateId);
    }
  }

  async function handleApply(candidateId: string) {
    confirmApplyId = null;
    applyingId = candidateId;
    actionError = null;
    try {
      const excluded = getExcludedFields(fieldSelections, candidateId);
      const updated = await api.resolution.applyCandidate(
        book.id,
        candidateId,
        excluded.length > 0 ? excluded : undefined
      );
      onapply(updated);
    } catch (err) {
      actionError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to apply candidate';
    } finally {
      applyingId = null;
    }
  }

  async function handleReject(candidateId: string) {
    rejectingId = candidateId;
    actionError = null;
    try {
      await api.resolution.rejectCandidate(book.id, candidateId);
      onreject(candidateId);
    } catch (err) {
      actionError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to reject candidate';
    } finally {
      rejectingId = null;
    }
  }

  async function handleUndo(candidateId: string) {
    undoingId = candidateId;
    actionError = null;
    try {
      const updated = await api.resolution.undoCandidate(book.id, candidateId);
      onundo(updated);
    } catch (err) {
      actionError =
        err instanceof ApiError
          ? err.userMessage
          : err instanceof Error
            ? err.message
            : 'Failed to undo candidate';
    } finally {
      undoingId = null;
    }
  }

</script>

<div class="space-y-4">
  {#if book.metadata_locked}
    <div
      class="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-900 dark:text-amber-200"
    >
      Metadata is locked. Refreshes stay in review-only mode until you unlock the book.
    </div>
  {/if}

  {#if actionError}
    <div
      class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {actionError}
    </div>
  {/if}

  {#if candidates.length === 0}
    <div class="rounded-lg border border-dashed border-border p-6 text-center">
      <p class="text-sm text-muted-foreground">No candidates found for this book.</p>
    </div>
  {:else}
    <!-- Pending candidates -->
    {#each pendingCandidates as candidate (candidate.id)}
      <div
        class="rounded-lg border border-border bg-card shadow-sm"
      >
        <!-- Candidate header -->
        <div class="flex items-center justify-between border-b border-border px-4 py-3">
          <div class="flex items-center gap-2">
            <span
              class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {providerColorClass(
                candidate.provider_name
              )}"
            >
              {candidate.provider_name}
            </span>
            {#if candidate.tier}
              <span class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {tierColorClass(candidate.tier)}">
                {tierLabel(candidate.tier)}
              </span>
            {/if}
            <div class="flex items-center gap-2">
              <span class="text-xs font-medium text-muted-foreground">Score:</span>
              <div class="flex items-center gap-1.5">
                <div class="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
                  <div
                    class="h-full rounded-full transition-all {scoreColor(candidate.score)}"
                    style="width: {candidate.score * 100}%"
                  ></div>
                </div>
                <span class="text-xs font-semibold">{formatScore(candidate.score)}</span>
              </div>
            </div>
          </div>
          <div class="flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              class="h-7 px-2 text-xs"
              disabled={rejectingId === candidate.id || applyingId !== null}
              onclick={() => handleReject(candidate.id)}
            >
              {#if rejectingId === candidate.id}
                Rejecting...
              {:else}
                Reject
              {/if}
            </Button>
            <Button
              size="sm"
              class="h-7 px-2 text-xs"
              disabled={applyingId === candidate.id || rejectingId !== null}
              onclick={() => requestApply(candidate.id)}
            >
              {#if applyingId === candidate.id}
                Applying...
              {:else}
                Apply
              {/if}
            </Button>
          </div>
        </div>

        <!-- Candidate metadata comparison -->
        <div class="px-4 py-3">
          <!-- Match reasons -->
          {#if candidate.match_reasons.length > 0}
            <div class="mb-3 flex flex-wrap gap-1.5">
              {#each candidate.match_reasons.filter((r) => !r.startsWith('Tier: ')) as reason, i (i)}
                <span
                  class="inline-flex rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
                >
                  {reason}
                </span>
              {/each}
            </div>
          {/if}

          {#if candidate.disputes.length > 0}
            <div class="mb-3 flex flex-wrap gap-1.5">
              {#each candidate.disputes as dispute, i (i)}
                <span
                  class="inline-flex rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive"
                >
                  {dispute}
                </span>
              {/each}
            </div>
          {/if}

          <!-- Side-by-side comparison -->
          <div class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead>
                <tr class="border-b border-border text-left text-xs text-muted-foreground">
                  <th class="w-6 pb-2 pr-1"></th>
                  <th class="pb-2 pr-3 font-medium">Field</th>
                  <th class="pb-2 pr-3 font-medium">Current</th>
                  <th class="pb-2 font-medium">Candidate</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-border/50">
                <!-- Title -->
                {#if candidate.title != null}
                  {@const titleMatch = !hasChange(candidate.title, book.title)}
                  <tr class={titleMatch ? 'opacity-40' : !isFieldIncluded(candidate.id, 'title') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if !titleMatch}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'title')}
                          onchange={() => toggleField(candidate.id, 'title')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Title</td>
                    <td class="py-1.5 pr-3 text-xs">{book.title}</td>
                    <td class="py-1.5 text-xs {titleMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {candidate.title}
                    </td>
                  </tr>
                {/if}
                <!-- Subtitle -->
                {#if candidate.subtitle != null}
                  {@const subtitleMatch = !hasChange(candidate.subtitle ?? null, book.subtitle ?? null)}
                  <tr class={subtitleMatch ? 'opacity-40' : candidate.subtitle != null && !isFieldIncluded(candidate.id, 'subtitle') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if candidate.subtitle != null && !subtitleMatch}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'subtitle')}
                          onchange={() => toggleField(candidate.id, 'subtitle')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Subtitle</td>
                    <td class="py-1.5 pr-3 text-xs">{book.subtitle ?? '--'}</td>
                    <td class="py-1.5 text-xs {subtitleMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {candidate.subtitle ?? '--'}
                    </td>
                  </tr>
                {/if}
                <!-- Authors (split by role) -->
                {#if candidate.authors.length > 0}
                  {@const candidateAuthors = candidate.authors.filter((a) => isAuthorRole(a.role))}
                  {@const bookAuthors = book.authors.filter((a) => isAuthorRole(a.role))}
                  {@const candidateAuthorNames = candidateAuthors.map((a) => a.name).join(', ')}
                  {@const bookAuthorNames = bookAuthors.map((a) => a.name).join(', ')}
                  {@const authorRowMatch = candidateAuthorNames === bookAuthorNames}
                  {@const authorsIncluded = isFieldIncluded(candidate.id, 'authors')}
                  <!-- Author-role row -->
                  <tr class={authorRowMatch ? 'opacity-40' : !authorsIncluded ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if !authorRowMatch}
                        <input
                          type="checkbox"
                          checked={authorsIncluded}
                          onchange={() => toggleField(candidate.id, 'authors')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Authors</td>
                    <td class="py-1.5 pr-3 text-xs">
                      {bookAuthorNames || '--'}
                    </td>
                    <td class="py-1.5 text-xs {authorRowMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {candidateAuthorNames || '--'}
                    </td>
                  </tr>
                  <!-- Non-author contributor rows -->
                  {@const nonAuthorRoles = [...new Set([
                    ...candidate.authors.filter((a) => !isAuthorRole(a.role)).map((a) => a.role!),
                    ...book.authors.filter((a) => !isAuthorRole(a.role)).map((a) => a.role)
                  ])]}
                  {#each nonAuthorRoles as role (role)}
                    {@const candidateNames = candidate.authors.filter((a) => a.role === role).map((a) => a.name).join(', ')}
                    {@const bookNames = book.authors.filter((a) => a.role === role).map((a) => a.name).join(', ')}
                    {@const contribMatch = candidateNames === bookNames}
                    <tr class={contribMatch ? 'opacity-40' : !authorsIncluded ? 'opacity-40' : ''}>
                      <td class="py-1.5 pr-1">
                        {#if !contribMatch}
                          <input
                            type="checkbox"
                            checked={authorsIncluded}
                            onchange={() => toggleField(candidate.id, 'authors')}
                            class="h-3.5 w-3.5 rounded border-border"
                          />
                        {/if}
                      </td>
                      <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">{titleCase(role)}</td>
                      <td class="py-1.5 pr-3 text-xs">{bookNames || '--'}</td>
                      <td class="py-1.5 text-xs {contribMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                        {candidateNames || '--'}
                      </td>
                    </tr>
                  {/each}
                {/if}
                <!-- Publisher -->
                {#if candidate.publisher || book.publisher_name}
                  {@const publisherMatch = !hasChange(candidate.publisher, book.publisher_name)}
                  <tr class={publisherMatch ? 'opacity-40' : candidate.publisher && !isFieldIncluded(candidate.id, 'publisher') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if candidate.publisher && !publisherMatch}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'publisher')}
                          onchange={() => toggleField(candidate.id, 'publisher')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Publisher</td>
                    <td class="py-1.5 pr-3 text-xs">{book.publisher_name ?? '--'}</td>
                    <td class="py-1.5 text-xs {publisherMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {candidate.publisher ?? '--'}
                    </td>
                  </tr>
                {/if}
                <!-- Publication Date -->
                {#if candidate.publication_year != null || book.publication_year != null}
                  {@const pubYearMatch = candidate.publication_year === book.publication_year}
                  <tr class={pubYearMatch ? 'opacity-40' : candidate.publication_year != null && !isFieldIncluded(candidate.id, 'publication_year') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if candidate.publication_year != null && !pubYearMatch}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'publication_year')}
                          onchange={() => toggleField(candidate.id, 'publication_year')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Published</td>
                    <td class="py-1.5 pr-3 text-xs">{book.publication_year ?? '--'}</td>
                    <td class="py-1.5 text-xs {pubYearMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {candidate.publication_year ?? '--'}
                    </td>
                  </tr>
                {/if}
                <!-- ISBN (additive — backend merges, never replaces) -->
                {#if candidate.isbn}
                  {@const existingIsbns = book.identifiers.filter(
                    (i) => i.identifier_type === 'isbn13' || i.identifier_type === 'isbn10'
                  )}
                  {@const alreadyHas = existingIsbns.some((i) => i.value === candidate.isbn)}
                  <tr class={alreadyHas ? 'opacity-40' : !isFieldIncluded(candidate.id, 'identifiers') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if !alreadyHas}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'identifiers')}
                          onchange={() => toggleField(candidate.id, 'identifiers')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">ISBN</td>
                    <td class="py-1.5 pr-3 font-mono text-xs">
                      {existingIsbns.map((i) => i.value).join(', ') || '--'}
                    </td>
                    <td class="py-1.5 font-mono text-xs {alreadyHas ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {#if alreadyHas}
                        {candidate.isbn} <span class="font-sans text-muted-foreground">(already present)</span>
                      {:else}
                        + {candidate.isbn}
                      {/if}
                    </td>
                  </tr>
                {/if}
                <!-- Series -->
                {#if candidate.series}
                  {@const currentSeriesText = book.series.length > 0
                    ? book.series.map((s) => (s.position != null ? `${s.name} #${s.position}` : s.name)).join(', ')
                    : ''}
                  {@const candidateSeriesText = `${candidate.series.name}${candidate.series.position != null ? ` #${candidate.series.position}` : ''}`}
                  {@const seriesMatch = currentSeriesText === candidateSeriesText}
                  <tr class={seriesMatch ? 'opacity-40' : !isFieldIncluded(candidate.id, 'series') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if !seriesMatch}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'series')}
                          onchange={() => toggleField(candidate.id, 'series')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Series</td>
                    <td class="py-1.5 pr-3 text-xs">
                      {currentSeriesText || '--'}
                    </td>
                    <td class="py-1.5 text-xs {seriesMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      {candidateSeriesText}
                    </td>
                  </tr>
                {/if}
                <!-- Description (show truncated if present) -->
                {#if candidate.description}
                  {@const descMatch = !hasChange(candidate.description, book.description)}
                  <tr class={descMatch ? 'opacity-40' : !isFieldIncluded(candidate.id, 'description') ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1 align-top">
                      {#if !descMatch}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'description')}
                          onchange={() => toggleField(candidate.id, 'description')}
                          class="mt-0.5 h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 align-top text-xs font-medium text-muted-foreground"
                      >Description</td
                    >
                    <td class="py-1.5 pr-3 text-xs">
                      {#if book.description}
                        <span class="line-clamp-2">{book.description}</span>
                      {:else}
                        --
                      {/if}
                    </td>
                    <td class="py-1.5 text-xs {descMatch ? 'text-muted-foreground' : 'font-medium text-primary'}">
                      <span class="line-clamp-2">{candidate.description}</span>
                    </td>
                  </tr>
                {/if}
                <!-- Cover -->
                {#if candidate.cover_url || book.has_cover}
                  <tr class={candidate.cover_url && !isFieldIncluded(candidate.id, 'cover') ? 'opacity-40' : ''}>
                    <td class="py-2 pr-1 align-top">
                      {#if candidate.cover_url}
                        <input
                          type="checkbox"
                          checked={isFieldIncluded(candidate.id, 'cover')}
                          onchange={() => toggleField(candidate.id, 'cover')}
                          class="mt-0.5 h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-2 pr-3 align-top text-xs font-medium text-muted-foreground">Cover</td>
                    <td class="py-2 pr-3">
                      {#if book.has_cover}
                        <button
                          type="button"
                          class="cursor-pointer border-0 bg-transparent p-0"
                          onclick={() => coverCompare = { currentUrl: `/api/books/${book.id}/cover${coverQuery}`, candidateUrl: candidate.cover_url ?? null }}
                        >
                          <img
                            src="/api/books/{book.id}/cover?size=sm{coverSuffix}"
                            alt="Current cover"
                            class="h-20 rounded object-contain shadow-sm"
                          />
                        </button>
                      {:else}
                        <span class="text-xs text-muted-foreground">--</span>
                      {/if}
                    </td>
                    <td class="py-2">
                      {#if candidate.cover_url}
                        <button
                          type="button"
                          class="cursor-pointer border-0 bg-transparent p-0"
                          onclick={() => coverCompare = { currentUrl: book.has_cover ? `/api/books/${book.id}/cover${coverQuery}` : null, candidateUrl: candidate.cover_url ?? null }}
                        >
                          <img
                            src={candidate.cover_url}
                            alt="Candidate cover"
                            class="h-20 rounded object-contain shadow-sm"
                          />
                        </button>
                      {:else}
                        <span class="text-xs text-muted-foreground">--</span>
                      {/if}
                    </td>
                  </tr>
                {/if}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    {/each}

    <!-- Applied candidates -->
    {#if appliedCandidates.length > 0}
      <div class="space-y-2">
        <h4 class="text-xs font-medium text-muted-foreground">Applied</h4>
        {#each appliedCandidates as candidate (candidate.id)}
          <div
            class="rounded-lg border border-green-200 bg-green-50/50 px-4 py-2.5 dark:border-green-900/30 dark:bg-green-900/10"
          >
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2">
                <span
                  class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {providerColorClass(
                    candidate.provider_name
                  )}"
                >
                  {candidate.provider_name}
                </span>
                <span class="text-xs text-muted-foreground">
                  {formatScore(candidate.score)} score
                </span>
                <span
                  class="inline-flex rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-800 dark:bg-green-900/30 dark:text-green-400"
                >
                  Applied
                </span>
              </div>
              <Button
                size="sm"
                variant="outline"
                class="h-7 px-2 text-xs"
                disabled={undoingId === candidate.id}
                onclick={() => handleUndo(candidate.id)}
              >
                {#if undoingId === candidate.id}
                  Undoing...
                {:else}
                  Undo
                {/if}
              </Button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    <!-- Rejected candidates (collapsed) -->
    {#if rejectedCandidates.length > 0}
      <div class="space-y-2">
        <h4 class="text-xs font-medium text-muted-foreground">
          Rejected ({rejectedCandidates.length})
        </h4>
        {#each rejectedCandidates as candidate (candidate.id)}
          <div class="rounded-lg border border-border bg-muted/30 px-4 py-2.5 opacity-60">
            <div class="flex items-center gap-2">
              <span
                class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {providerColorClass(
                  candidate.provider_name
                )}"
              >
                {candidate.provider_name}
              </span>
              <span class="text-xs text-muted-foreground">
                {candidate.title ?? 'No title'}
              </span>
              <span class="text-xs text-muted-foreground">
                {formatScore(candidate.score)}
              </span>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  <!-- Confirmation dialog: applying over an existing apply -->
  {#if confirmApplyId}
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/70"
      role="dialog"
      aria-modal="true"
      aria-label="Confirm apply"
      onclick={() => confirmApplyId = null}
      onkeydown={(e) => { if (e.key === 'Escape') confirmApplyId = null; }}
    >
      <div
        class="mx-4 max-w-sm rounded-xl bg-popover p-6 shadow-2xl"
        onclick={(e) => e.stopPropagation()}
      >
        <p class="mb-4 text-sm">
          Applying this candidate will replace the previous apply and make it permanent. Continue?
        </p>
        <div class="flex justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            onclick={() => confirmApplyId = null}
          >
            Cancel
          </Button>
          <Button
            size="sm"
            onclick={() => { if (confirmApplyId) handleApply(confirmApplyId); }}
          >
            Apply
          </Button>
        </div>
      </div>
    </div>
  {/if}

  <!-- Cover comparison modal -->
  {#if coverCompare}
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/70"
      role="dialog"
      aria-modal="true"
      aria-label="Cover comparison"
      onclick={() => coverCompare = null}
      onkeydown={(e) => { if (e.key === 'Escape') coverCompare = null; }}
    >
      <div
        class="mx-4 flex max-h-[90vh] items-end gap-6 rounded-xl bg-popover p-6 shadow-2xl"
        onclick={(e) => e.stopPropagation()}
      >
        <div class="flex flex-col items-center gap-2">
          {#if coverCompare.currentUrl}
            <img
              src={coverCompare.currentUrl}
              alt="Current cover"
              class="max-h-[80vh] rounded object-contain"
            />
          {:else}
            <div class="flex h-48 w-32 items-center justify-center rounded bg-muted text-xs text-muted-foreground">
              No cover
            </div>
          {/if}
          <span class="text-sm font-medium text-muted-foreground">Current</span>
        </div>
        <div class="flex flex-col items-center gap-2">
          {#if coverCompare.candidateUrl}
            <img
              src={coverCompare.candidateUrl}
              alt="Candidate cover"
              class="max-h-[80vh] rounded object-contain"
            />
          {:else}
            <div class="flex h-48 w-32 items-center justify-center rounded bg-muted text-xs text-muted-foreground">
              No cover
            </div>
          {/if}
          <span class="text-sm font-medium text-muted-foreground">Candidate</span>
        </div>
      </div>
    </div>
  {/if}
</div>
