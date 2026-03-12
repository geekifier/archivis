<script lang="ts">
  import type { TaskProgressEvent, TaskResponse, ChildrenSummary } from '$lib/api/index.js';
  import { api } from '$lib/api/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import TaskStatusBadge from './TaskStatusBadge.svelte';
  import {
    taskTypeLabel,
    taskStatsLabels,
    progressBarColor,
    formatElapsedTime,
    isTerminalStatus
  } from './task-utils.js';
  import XIcon from '@lucide/svelte/icons/x';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  interface Props {
    task?: TaskResponse | null;
    progress?: TaskProgressEvent | null;
    childrenSummary?: ChildrenSummary | null;
    childProgressEntries?: Record<string, TaskProgressEvent> | null;
    cancelling?: boolean;
    onDismiss?: () => void;
    onCancelling?: () => void;
  }

  let {
    task = null,
    progress = null,
    childrenSummary = null,
    childProgressEntries = null,
    cancelling = false,
    onDismiss,
    onCancelling
  }: Props = $props();

  let expanded = $state(false);
  let cancelApiInFlight = $state(false);
  let showCancelConfirm = $state(false);

  // Backend status from SSE or task data (the real status from the server)
  const backendStatus = $derived(progress?.status ?? task?.status ?? 'pending');
  // Display status: override with 'cancelling' or 'processing' as needed
  const hasActiveChildren = $derived(
    childrenSummary != null && (childrenSummary.running > 0 || childrenSummary.pending > 0)
  );
  const currentStatus = $derived.by(() => {
    const effectivelyInFlight = !isTerminalStatus(backendStatus) || hasActiveChildren;
    if (cancelling && effectivelyInFlight) return 'cancelling';
    if (isTerminalStatus(backendStatus) && hasActiveChildren) return 'processing';
    return backendStatus;
  });
  const currentProgress = $derived(progress?.progress ?? task?.progress ?? 0);
  const currentMessage = $derived(progress?.message ?? task?.message ?? null);
  const taskType = $derived(task?.task_type ?? 'import_directory');
  const taskId = $derived(progress?.task_id ?? task?.id ?? '');
  const isTerminal = $derived(isTerminalStatus(currentStatus));
  const canCancel = $derived(!isTerminal && !cancelling && !cancelApiInFlight);

  // Persist the last-seen structured data — not every SSE event carries it
  // (e.g. on_file_start sends progress without data), so we keep the most
  // recent non-null value to avoid flickering.
  let lastProgressData = $state<Record<string, unknown> | null>(null);
  $effect(() => {
    if (progress?.data) {
      lastProgressData = progress.data;
    }
  });
  const progressData = $derived(lastProgressData);
  const skippedCount = $derived(progressData ? Number(progressData.skipped ?? 0) : null);
  const failedCount = $derived(progressData ? Number(progressData.failed ?? 0) : null);
  const processedCount = $derived(progressData ? Number(progressData.processed ?? 0) : null);
  const totalCount = $derived(progressData ? Number(progressData.total ?? 0) : null);
  const statsLabels = $derived(taskStatsLabels(taskType));
  const successCount = $derived.by(() => {
    if (!progressData) return null;
    if (taskType === 'import_directory' && progressData.imported !== undefined) return Number(progressData.imported);
    if (taskType === 'resolve_book' && progressData.resolved !== undefined) return Number(progressData.resolved);
    if (taskType === 'scan_isbn' && progressData.scanned !== undefined) return Number(progressData.scanned);
    return null;
  });

  // Derive stages from child progress entries for multi-stage view
  const stages = $derived.by(() => {
    if (!childProgressEntries) return [];
    const byType: Record<string, TaskProgressEvent> = {};
    for (const entry of Object.values(childProgressEntries)) {
      const key = entry.task_type ?? '';
      const existing = byType[key];
      // Prefer non-terminal events; if both terminal, keep latest
      if (!existing || !isTerminalStatus(entry.status) || isTerminalStatus(existing.status)) {
        byType[key] = entry;
      }
    }
    // Fixed display order
    const result: TaskProgressEvent[] = [];
    for (const type of ['scan_isbn', 'resolve_book']) {
      const event = byType[type];
      if (event) result.push(event);
    }
    return result;
  });

  // Whether the expandable section has any visible content
  const hasExpandableContent = $derived.by(() => {
    const hasStats =
      processedCount !== null &&
      totalCount !== null &&
      !((currentStatus === 'completed' || currentStatus === 'processing') && progress?.result);
    const hasResult =
      (currentStatus === 'completed' || currentStatus === 'processing') &&
      progress?.result &&
      stages.length === 0;
    const hasError =
      currentStatus === 'failed' && (progress?.error || task?.error_message);
    const hasChildren =
      childrenSummary && childrenSummary.total > 0 && stages.length === 0;
    return !!(hasStats || hasResult || hasError || hasChildren);
  });

  // Elapsed time (auto-refreshing every second for running tasks)
  let elapsedTime = $state('');
  let elapsedInterval: ReturnType<typeof setInterval> | null = null;

  $effect(() => {
    if (backendStatus === 'running' && task?.started_at) {
      const startedAt = task.started_at;
      elapsedTime = formatElapsedTime(startedAt);
      elapsedInterval = setInterval(() => {
        elapsedTime = formatElapsedTime(startedAt);
      }, 1000);
    } else {
      if (elapsedInterval) {
        clearInterval(elapsedInterval);
        elapsedInterval = null;
      }
      elapsedTime = '';
    }

    return () => {
      if (elapsedInterval) clearInterval(elapsedInterval);
    };
  });

  async function handleCancel() {
    if (!taskId) return;
    cancelApiInFlight = true;
    try {
      await api.tasks.cancel(taskId);
      showCancelConfirm = false;
      onCancelling?.();
    } catch {
      // Error handling — the task may have finished between button click and request
    } finally {
      cancelApiInFlight = false;
    }
  }
</script>

<div class="rounded-lg border border-border p-3">
  <!-- Header row -->
  <div class="flex items-center justify-between">
    <div class="flex items-center gap-2">
      <span class="text-sm font-medium">{taskTypeLabel(taskType)}</span>
      <TaskStatusBadge status={currentStatus} />
      {#if elapsedTime}
        <span class="text-xs text-muted-foreground">{elapsedTime}</span>
      {/if}
    </div>
    <div class="flex items-center gap-1">
      {#if canCancel}
        {#if showCancelConfirm}
          <div class="flex items-center gap-1">
            <span class="text-xs text-muted-foreground">Cancel?</span>
            <Button
              variant="destructive"
              size="sm"
              class="h-6 px-2 text-xs"
              onclick={handleCancel}
              disabled={cancelApiInFlight}
            >
              {cancelApiInFlight ? 'Cancelling...' : 'Yes'}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              class="h-6 px-2 text-xs"
              onclick={() => (showCancelConfirm = false)}
            >
              No
            </Button>
          </div>
        {:else}
          <Button
            variant="ghost"
            size="sm"
            class="h-6 px-2 text-xs text-muted-foreground hover:text-destructive"
            onclick={() => (showCancelConfirm = true)}
          >
            Cancel
          </Button>
        {/if}
      {/if}
      {#if isTerminal && onDismiss}
        <Button
          variant="ghost"
          size="icon-sm"
          class="size-6"
          onclick={onDismiss}
          aria-label="Dismiss"
        >
          <XIcon class="size-3" />
        </Button>
      {/if}
      {#if hasExpandableContent}
        <Button
          variant="ghost"
          size="icon-sm"
          class="size-6"
          onclick={() => (expanded = !expanded)}
          aria-label={expanded ? 'Collapse details' : 'Expand details'}
        >
          <ChevronDownIcon class="size-3 transition-transform {expanded ? 'rotate-180' : ''}" />
        </Button>
      {/if}
    </div>
  </div>

  <!-- Progress bar (running / pending / cancelling / cancelled) -->
  {#if currentStatus === 'running' || currentStatus === 'pending' || currentStatus === 'cancelling' || currentStatus === 'cancelled'}
    <div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
      <div
        class="h-full rounded-full transition-all duration-300 {progressBarColor(currentStatus)}"
        class:animate-pulse={currentStatus === 'cancelling'}
        style="width: {currentProgress}%"
      ></div>
    </div>
    <div class="mt-1 flex items-center justify-between text-xs text-muted-foreground">
      <span>
        {#if currentStatus === 'cancelling'}
          Cancelling {taskTypeLabel(taskType).toLowerCase()}...
        {:else if currentMessage}
          {currentMessage}
        {:else if currentStatus === 'cancelled'}
          Import cancelled
        {:else}
          Waiting...
        {/if}
      </span>
      <span>{currentProgress}%</span>
    </div>
  {/if}

  <!-- Stage view (processing or terminal with child SSE data) -->
  {#if (currentStatus === 'processing' || isTerminal) && stages.length > 0}
    <div class="mt-2 space-y-2">
      <!-- Root stage row (always complete when `processing`) -->
      <div class="flex items-center gap-2 text-xs">
        <span class="text-green-600 dark:text-green-400">&#10003;</span>
        <span class="w-24 font-medium">Import</span>
        <span class="flex-1 text-muted-foreground">
          {#if progress?.result}
            {#if progress.result.imported !== undefined}
              {progress.result.imported} imported
              {#if Number(progress.result.skipped) > 0}
                &middot; {progress.result.skipped} skipped
              {/if}
            {:else}
              Done
            {/if}
          {:else}
            Done
          {/if}
        </span>
      </div>
      <!-- Child stage rows -->
      {#each stages as stage (stage.task_type)}
        {@const stageTerminal = isTerminalStatus(stage.status)}
        {@const stageData = stage.data}
        <div class="flex items-center gap-2 text-xs">
          {#if stage.status === 'cancelled'}
            <span class="text-amber-600 dark:text-amber-400">&#10007;</span>
          {:else if stage.status === 'failed'}
            <span class="text-destructive">&#10007;</span>
          {:else if stageTerminal}
            <span class="text-green-600 dark:text-green-400">&#10003;</span>
          {:else}
            <span class="text-blue-600 dark:text-blue-400 animate-spin-slow">&#10227;</span>
          {/if}
          <span class="w-24 font-medium">{taskTypeLabel(stage.task_type ?? '')}</span>
          {#if !stageTerminal && stageData}
            {@const sTotal = Number(stageData.total ?? 0)}
            {@const sProcessed = Number(stageData.processed ?? 0)}
            {@const sLabels = taskStatsLabels(stage.task_type ?? '')}
            <div class="flex flex-1 items-center gap-2">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                <div
                  class="h-full rounded-full transition-all duration-300 bg-primary"
                  style="width: {stage.progress}%"
                ></div>
              </div>
              <span class="text-muted-foreground whitespace-nowrap">
                {sProcessed}/{sTotal} {sLabels.successLabel.toLowerCase()}
              </span>
            </div>
          {:else if !stageTerminal}
            <span class="flex-1 text-muted-foreground">{stage.message ?? 'Processing...'}</span>
          {:else if stage.status === 'cancelled'}
            <span class="flex-1 text-amber-600 dark:text-amber-400">
              Cancelled{#if stageData}{@const sProcessed = Number(stageData.processed ?? 0)}{@const sTotal = Number(stageData.total ?? 0)}{#if sTotal > 0} · {sProcessed}/{sTotal}{/if}{/if}
            </span>
          {:else if stage.status === 'failed'}
            <span class="flex-1 text-destructive">
              Failed{#if stage.error} · {stage.error}{/if}
            </span>
          {:else}
            <span class="flex-1 text-muted-foreground">Done</span>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <!-- Degraded processing view (no child SSE data yet) -->
  {#if currentStatus === 'processing' && stages.length === 0 && childrenSummary && childrenSummary.total > 0}
    <div class="mt-2 rounded bg-muted/50 p-2 text-xs">
      <span class="font-medium">Subtasks:</span>
      <span class="ml-1">
        {#if childrenSummary.completed > 0}
          <span class="text-green-600 dark:text-green-400">{childrenSummary.completed} done</span>
        {/if}
        {#if childrenSummary.running > 0}
          <span class="ml-1 text-blue-600 dark:text-blue-400">{childrenSummary.running} running</span>
        {/if}
        {#if childrenSummary.pending > 0}
          <span class="ml-1 text-muted-foreground">{childrenSummary.pending} pending</span>
        {/if}
      </span>
    </div>
  {/if}

  <!-- Expanded detail view -->
  {#if expanded}
    <div class="mt-3 space-y-2 border-t border-border pt-2">
      <!-- Stats (from structured progress data) -->
      {#if processedCount !== null && totalCount !== null && !((currentStatus === 'completed' || currentStatus === 'processing') && progress?.result)}
        <div class="grid grid-cols-2 gap-2 text-xs">
          <div>
            <span class="text-muted-foreground">{statsLabels.countLabel}:</span>
            <span class="ml-1 font-medium">{processedCount} / {totalCount}</span>
          </div>
          {#if successCount !== null}
            <div>
              <span class="text-muted-foreground">{statsLabels.successLabel}:</span>
              <span class="ml-1 font-semibold text-green-600 dark:text-green-400"
                >{successCount}</span
              >
            </div>
          {/if}
          {#if skippedCount !== null && skippedCount > 0}
            <div>
              <span class="text-muted-foreground">Skipped:</span>
              <span class="ml-1 font-semibold text-amber-600 dark:text-amber-400"
                >{skippedCount}</span
              >
            </div>
          {/if}
          {#if failedCount !== null && failedCount > 0}
            <div>
              <span class="text-muted-foreground">Failed:</span>
              <span class="ml-1 font-semibold text-red-600 dark:text-red-400">{failedCount}</span>
            </div>
          {/if}
        </div>
      {/if}

      <!-- Completion result (hidden when stage rows already show this info) -->
      {#if (currentStatus === 'completed' || currentStatus === 'processing') && progress?.result && stages.length === 0}
        <div class="rounded bg-muted/50 p-2 text-xs">
          {#if progress.result.imported !== undefined}
            <div class="flex flex-wrap gap-3">
              <span>
                <span class="font-semibold text-green-600 dark:text-green-400"
                  >{progress.result.imported}</span
                >
                <span class="text-muted-foreground"> imported</span>
              </span>
              {#if Number(progress.result.skipped) > 0}
                <span>
                  <span class="font-semibold text-amber-600 dark:text-amber-400"
                    >{progress.result.skipped}</span
                  >
                  <span class="text-muted-foreground"> skipped</span>
                </span>
              {/if}
              {#if Number(progress.result.failed) > 0}
                <span>
                  <span class="font-semibold text-red-600 dark:text-red-400"
                    >{progress.result.failed}</span
                  >
                  <span class="text-muted-foreground"> failed</span>
                </span>
              {/if}
            </div>
          {:else if progress.result.resolved !== undefined}
            <div class="flex flex-wrap gap-3">
              <span>
                <span class="font-semibold text-green-600 dark:text-green-400"
                  >{progress.result.resolved}</span
                >
                <span class="text-muted-foreground"> resolved</span>
              </span>
              {#if Number(progress.result.failed) > 0}
                <span>
                  <span class="font-semibold text-red-600 dark:text-red-400"
                    >{progress.result.failed}</span
                  >
                  <span class="text-muted-foreground"> failed</span>
                </span>
              {/if}
              <span class="text-muted-foreground">of {progress.result.total} books</span>
            </div>
          {:else if progress.result.book_id}
            <div class="flex items-center justify-between">
              <span class="text-muted-foreground">Book imported successfully</span>
              <a
                href="/books/{progress.result.book_id}"
                class="font-medium text-primary hover:underline"
              >
                View Book
              </a>
            </div>
          {/if}
        </div>
        {#if currentStatus === 'completed'}
          <div class="mt-1">
            <a
              href="/"
              class="inline-flex items-center gap-1.5 text-xs font-medium text-primary hover:underline"
            >
              <svg
                class="size-3.5"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"
                />
              </svg>
              View Library
            </a>
          </div>
        {/if}
      {/if}

      <!-- Error details -->
      {#if currentStatus === 'failed' && (progress?.error || task?.error_message)}
        <p class="text-xs text-destructive">{progress?.error ?? task?.error_message}</p>
      {/if}

      <!-- Children summary (fallback context when stage rows aren't visible) -->
      {#if childrenSummary && childrenSummary.total > 0 && stages.length === 0}
        <div class="rounded bg-muted/50 p-2 text-xs">
          <span class="font-medium">Subtasks:</span>
          <div class="mt-1 flex flex-wrap gap-2">
            {#if childrenSummary.completed > 0}
              <span class="text-green-600 dark:text-green-400"
                >{childrenSummary.completed} done</span
              >
            {/if}
            {#if childrenSummary.running > 0}
              <span class="text-blue-600 dark:text-blue-400">{childrenSummary.running} running</span
              >
            {/if}
            {#if childrenSummary.pending > 0}
              <span class="text-muted-foreground">{childrenSummary.pending} pending</span>
            {/if}
            {#if childrenSummary.failed > 0}
              <span class="text-destructive">{childrenSummary.failed} failed</span>
            {/if}
            {#if childrenSummary.cancelled > 0}
              <span class="text-amber-600 dark:text-amber-400"
                >{childrenSummary.cancelled} cancelled</span
              >
            {/if}
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>
