<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '$lib/api/index.js';
  import type { BookDetail, CandidateResponse } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { providerColorClass, providerLabel } from '$lib/display.js';
  import {
    scoreColor,
    formatScore,
    hasChange,
    namesMatch,
    getExcludedFields,
    tierColorClass,
    tierLabel,
    extractErrorMessage,
    isWarningReason,
    warningFields,
    type CandidateFieldName
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

  /** Tracks which candidates have their description expanded. */
  let expandedDescs = $state<Record<string, boolean>>({});

  /** Per-candidate field selections: candidateId -> fieldName -> included. */
  let fieldSelections = $state<Record<string, Partial<Record<CandidateFieldName, boolean>>>>({});

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
        const sel: Partial<Record<CandidateFieldName, boolean>> = {};
        if (candidate.title != null) sel.title = true;
        if (candidate.subtitle != null) sel.subtitle = true;
        if (candidate.authors.length > 0) sel.authors = true;
        if (candidate.publication_year != null) sel.publication_year = true;
        if (candidate.language != null) sel.language = true;
        if (candidate.page_count != null) sel.page_count = true;
        if (candidate.isbn != null) sel.identifiers = true;
        if (candidate.series != null) sel.series = true;
        if (candidate.publisher != null) sel.publisher = true;
        if (candidate.description != null)
          sel.description = !warningFields(candidate.match_reasons).has('description');
        if (candidate.cover_url != null) sel.cover = true;
        fieldSelections[candidate.id] = sel;
      }
    }
  });

  function isFieldIncluded(candidateId: string, field: CandidateFieldName): boolean {
    return fieldSelections[candidateId]?.[field] ?? true;
  }

  function toggleField(candidateId: string, field: CandidateFieldName) {
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
      delete fieldSelections[candidateId];
    } catch (err) {
      actionError = extractErrorMessage(err, 'Failed to apply candidate');
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
      delete fieldSelections[candidateId];
    } catch (err) {
      actionError = extractErrorMessage(err, 'Failed to reject candidate');
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
      actionError = extractErrorMessage(err, 'Failed to undo candidate');
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
      {@const warned = warningFields(candidate.match_reasons)}
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
              {providerLabel(candidate.provider_name)}
            </span>
            {#if candidate.is_composite}
              <span class="inline-flex rounded-full bg-purple-100 px-2 py-0.5 text-xs font-medium text-purple-700 dark:bg-purple-900/30 dark:text-purple-300">
                Merged
              </span>
            {/if}
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
                  class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {isWarningReason(reason) ? 'bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400' : 'bg-primary/10 text-primary'}"
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
            {#snippet scalarField(
              candidateId: string,
              fieldKey: CandidateFieldName,
              label: string,
              currentDisplay: string | number | null,
              candidateDisplay: string | number | null,
              fieldMatch: boolean,
              showCheckbox: boolean,
              isIncluded: boolean,
              isWarned: boolean
            )}
              <tr class="{fieldMatch ? 'opacity-40' : showCheckbox && !isIncluded ? 'opacity-40' : ''} {isWarned ? 'bg-amber-50 dark:bg-amber-900/10' : ''}">
                <td class="py-1.5 pr-1">
                  {#if showCheckbox && !fieldMatch}
                    <input type="checkbox" checked={isIncluded}
                      onchange={() => toggleField(candidateId, fieldKey)}
                      class="h-3.5 w-3.5 rounded border-border" />
                  {/if}
                </td>
                <td class="py-1.5 pr-3 text-xs font-medium {isWarned ? 'text-amber-700 dark:text-amber-400' : 'text-muted-foreground'}">{label}</td>
                <td class="py-1.5 pr-3 text-xs">{currentDisplay ?? '--'}</td>
                <td class="py-1.5 text-xs {fieldMatch ? 'text-muted-foreground' : isWarned ? 'font-medium text-amber-700 dark:text-amber-400' : 'font-medium text-primary'}">
                  {candidateDisplay ?? '--'}
                </td>
              </tr>
            {/snippet}
            <table class="w-full text-sm">
              <thead>
                <tr class="border-b border-border text-left text-xs text-muted-foreground">
                  <th class="w-6 pb-2 pr-1"></th>
                  <th class="pb-2 pr-3 font-medium">Field</th>
                  <th class="w-[40%] pb-2 pr-3 font-medium">Current</th>
                  <th class="w-[40%] pb-2 font-medium">Candidate</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-border/50">
                <!-- Title -->
                {#if candidate.title != null}
                  {@const m = !hasChange(candidate.title, book.title)}
                  {@render scalarField(candidate.id, 'title', 'Title', book.title, candidate.title, m, true, isFieldIncluded(candidate.id, 'title'), warned.has('title'))}
                {/if}
                <!-- Subtitle -->
                {#if candidate.subtitle != null}
                  {@const m = !hasChange(candidate.subtitle ?? null, book.subtitle ?? null)}
                  {@render scalarField(candidate.id, 'subtitle', 'Subtitle', book.subtitle, candidate.subtitle, m, candidate.subtitle != null, isFieldIncluded(candidate.id, 'subtitle'), false)}
                {/if}
                <!-- Authors (split by role) -->
                {#if candidate.authors.length > 0}
                  {@const candidateAuthors = candidate.authors.filter((a) => isAuthorRole(a.role))}
                  {@const bookAuthors = book.authors.filter((a) => isAuthorRole(a.role))}
                  {@const candidateAuthorNames = candidateAuthors.map((a) => a.name).join(', ')}
                  {@const bookAuthorNames = bookAuthors.map((a) => a.name).join(', ')}
                  {@const authorRowMatch = namesMatch(candidateAuthors.map((a) => a.name), bookAuthors.map((a) => a.name))}
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
                    {@const contribMatch = namesMatch(
                      candidate.authors.filter((a) => a.role === role).map((a) => a.name),
                      book.authors.filter((a) => a.role === role).map((a) => a.name)
                    )}
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
                  {@const m = !hasChange(candidate.publisher, book.publisher_name)}
                  {@render scalarField(candidate.id, 'publisher', 'Publisher', book.publisher_name ?? null, candidate.publisher ?? null, m, !!candidate.publisher, isFieldIncluded(candidate.id, 'publisher'), false)}
                {/if}
                <!-- Publication Year -->
                {#if candidate.publication_year != null || book.publication_year != null}
                  {@const m = candidate.publication_year === book.publication_year}
                  {@render scalarField(candidate.id, 'publication_year', 'Published', book.publication_year ?? null, candidate.publication_year ?? null, m, candidate.publication_year != null, isFieldIncluded(candidate.id, 'publication_year'), false)}
                {/if}
                <!-- Language -->
                {#if candidate.language != null || book.language != null}
                  {@const m = (candidate.language ?? null) === (book.language ?? null)}
                  {@render scalarField(candidate.id, 'language', 'Language', book.language_label ?? book.language ?? null, candidate.language_label ?? candidate.language ?? null, m, candidate.language != null, isFieldIncluded(candidate.id, 'language'), false)}
                {/if}
                <!-- Page Count -->
                {#if candidate.page_count != null || book.page_count != null}
                  {@const m = candidate.page_count === book.page_count}
                  {@render scalarField(candidate.id, 'page_count', 'Pages', book.page_count ?? null, candidate.page_count ?? null, m, candidate.page_count != null, isFieldIncluded(candidate.id, 'page_count'), false)}
                {/if}
                <!-- ISBN (additive — backend merges, never replaces) -->
                {#if candidate.isbn}
                  {@const existingIsbns = book.identifiers.filter(
                    (i) => i.identifier_type === 'isbn13' || i.identifier_type === 'isbn10'
                  )}
                  {@const alreadyHas = existingIsbns.some((i) => i.value === candidate.isbn)}
                  {@const identifiersIncluded = isFieldIncluded(candidate.id, 'identifiers')}
                  <tr class={alreadyHas ? 'opacity-40' : !identifiersIncluded ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if !alreadyHas}
                        <input
                          type="checkbox"
                          checked={identifiersIncluded}
                          onchange={() => toggleField(candidate.id, 'identifiers')}
                          class="h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">ISBN</td>
                    <td class="py-1.5 pr-3 font-mono text-xs">
                      {#if existingIsbns.length > 0}
                        {#each existingIsbns as ident, i (i)}
                          {ident.value}{#if i < existingIsbns.length - 1}<br />{/if}
                        {/each}
                      {:else}
                        --
                      {/if}
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
                  {@const seriesIncluded = isFieldIncluded(candidate.id, 'series')}
                  <tr class={seriesMatch ? 'opacity-40' : !seriesIncluded ? 'opacity-40' : ''}>
                    <td class="py-1.5 pr-1">
                      {#if !seriesMatch}
                        <input
                          type="checkbox"
                          checked={seriesIncluded}
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
                <!-- Description (expandable) -->
                {#if candidate.description}
                  {@const descMatch = !hasChange(candidate.description, book.description)}
                  {@const descIncluded = isFieldIncluded(candidate.id, 'description')}
                  {@const descWarned = warned.has('description')}
                  {@const descExpanded = expandedDescs[candidate.id] ?? false}
                  <tr class="{descMatch ? 'opacity-40' : !descIncluded ? 'opacity-40' : ''} {descWarned ? 'bg-amber-50 dark:bg-amber-900/10' : ''}">
                    <td class="py-1.5 pr-1 align-top">
                      {#if !descMatch}
                        <input
                          type="checkbox"
                          checked={descIncluded}
                          onchange={() => toggleField(candidate.id, 'description')}
                          class="mt-0.5 h-3.5 w-3.5 rounded border-border"
                        />
                      {/if}
                    </td>
                    <td class="py-1.5 pr-3 align-top text-xs font-medium {descWarned ? 'text-amber-700 dark:text-amber-400' : 'text-muted-foreground'}"
                      >Description</td
                    >
                    <td
                      class="cursor-pointer py-1.5 pr-3 align-top text-xs"
                      onclick={() => expandedDescs[candidate.id] = !descExpanded}
                    >
                      {#if book.description}
                        <span class={descExpanded ? '' : 'line-clamp-2'}>{book.description}</span>
                      {:else}
                        --
                      {/if}
                    </td>
                    <td
                      class="cursor-pointer py-1.5 align-top text-xs {descMatch ? 'text-muted-foreground' : descWarned ? 'font-medium text-amber-700 dark:text-amber-400' : 'font-medium text-primary'}"
                      onclick={() => expandedDescs[candidate.id] = !descExpanded}
                    >
                      <span class={descExpanded ? '' : 'line-clamp-2'}>{candidate.description}</span>
                    </td>
                  </tr>
                {/if}
                <!-- Cover -->
                {#if candidate.cover_url || book.has_cover}
                  {@const coverIncluded = isFieldIncluded(candidate.id, 'cover')}
                  <tr class={candidate.cover_url && !coverIncluded ? 'opacity-40' : ''}>
                    <td class="py-2 pr-1 align-top">
                      {#if candidate.cover_url}
                        <input
                          type="checkbox"
                          checked={coverIncluded}
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
                  {providerLabel(candidate.provider_name)}
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
                {providerLabel(candidate.provider_name)}
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
  <AlertDialog.Root
    open={confirmApplyId !== null}
    onOpenChange={(open) => { if (!open) confirmApplyId = null; }}
  >
    <AlertDialog.Content>
      <AlertDialog.Header>
        <AlertDialog.Title>Confirm Apply</AlertDialog.Title>
        <AlertDialog.Description>
          Applying this candidate will replace the previous apply and make it permanent. Continue?
        </AlertDialog.Description>
      </AlertDialog.Header>
      <AlertDialog.Footer>
        <AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
        <AlertDialog.Action onclick={() => { if (confirmApplyId) handleApply(confirmApplyId); }}>
          Apply
        </AlertDialog.Action>
      </AlertDialog.Footer>
    </AlertDialog.Content>
  </AlertDialog.Root>

  <!-- Cover comparison modal -->
  <Dialog.Root
    open={coverCompare !== null}
    onOpenChange={(open) => { if (!open) coverCompare = null; }}
  >
    <Dialog.Content class="max-w-fit">
      <Dialog.Header>
        <Dialog.Title>Cover Comparison</Dialog.Title>
      </Dialog.Header>
      {#if coverCompare}
        <div class="flex items-end justify-center gap-6">
          <div class="flex flex-col items-center gap-2">
            {#if coverCompare.currentUrl}
              <img
                src={coverCompare.currentUrl}
                alt="Current cover"
                class="max-h-[70vh] rounded object-contain"
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
                class="max-h-[70vh] rounded object-contain"
              />
            {:else}
              <div class="flex h-48 w-32 items-center justify-center rounded bg-muted text-xs text-muted-foreground">
                No cover
              </div>
            {/if}
            <span class="text-sm font-medium text-muted-foreground">Candidate</span>
          </div>
        </div>
      {/if}
    </Dialog.Content>
  </Dialog.Root>
</div>
