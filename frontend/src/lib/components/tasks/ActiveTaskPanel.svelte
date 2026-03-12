<script lang="ts">
  import type { TaskProgressEvent, TaskResponse, ChildrenSummary } from '$lib/api/index.js';
  import { api, getSessionToken } from '$lib/api/index.js';
  import { Card, CardContent, CardHeader, CardTitle } from '$lib/components/ui/card/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import TaskCard from './TaskCard.svelte';
  import TaskStatusBadge from './TaskStatusBadge.svelte';
  import { isTerminalStatus, progressBarColor, taskTypeStageName } from './task-utils.js';
  import XIcon from '@lucide/svelte/icons/x';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  interface Props {
    /** Task IDs being tracked (from import/upload actions). */
    taskIds: string[];
    /** Called when all tracked tasks are done/dismissed. */
    onAllDone?: () => void;
  }

  let { taskIds = $bindable(), onAllDone }: Props = $props();

  // SSE progress state
  let taskProgressMap = $state<Record<string, TaskProgressEvent>>({});
  let taskDataMap = $state<Record<string, TaskResponse>>({});
  let childrenSummaryMap = $state<Record<string, ChildrenSummary>>({});
  let childProgressMap = $state<Record<string, Record<string, TaskProgressEvent>>>({});
  let cancellingTaskIds = $state(new Set<string>());
  let eventSource: EventSource | null = null;

  const hasActiveTasks = $derived(taskIds.length > 0);
  const taskIdSet = $derived(new Set(taskIds));

  // In-flight fetch dedup: tracks pending `fetchTaskData` promises per task ID
  // (not reactive — only used inside imperative handlers, not in the template)
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const inFlightFetches = new Set<string>();
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const needsRefetch = new Set<string>();

  // Start SSE when taskIds change
  $effect(() => {
    if (taskIds.length > 0) {
      startSSE();
      // Fetch initial task data
      for (const id of taskIds) {
        fetchTaskData(id);
      }
    }
    return () => {
      if (eventSource) {
        eventSource.close();
        eventSource = null;
      }
    };
  });

  async function fetchTaskData(taskId: string) {
    if (inFlightFetches.has(taskId)) {
      needsRefetch.add(taskId);
      return;
    }
    inFlightFetches.add(taskId);
    try {
      const task = await api.tasks.get(taskId);
      taskDataMap[taskId] = task;
      if (task.children_summary) {
        childrenSummaryMap[taskId] = task.children_summary;
      }
      // Check if root is terminal and all children are done
      if (isTerminalStatus(task.status)) {
        const cs = task.children_summary;
        if (!cs || (cs.running === 0 && cs.pending === 0)) {
          // Clear cancelling state — root is fully done
          if (cancellingTaskIds.has(taskId)) {
            cancellingTaskIds = new Set([...cancellingTaskIds].filter((id) => id !== taskId));
          }
          onAllDone?.();
        }
      }
    } catch {
      // Task may not exist yet or be unavailable
    } finally {
      inFlightFetches.delete(taskId);
      if (needsRefetch.has(taskId)) {
        needsRefetch.delete(taskId);
        fetchTaskData(taskId);
      }
    }
  }

  function startSSE() {
    if (eventSource) {
      eventSource.close();
    }

    const token = getSessionToken();
    const url = `/api/tasks/active${token ? `?token=${encodeURIComponent(token)}` : ''}`;
    eventSource = new EventSource(url);

    const handleEvent = (event: MessageEvent) => {
      try {
        const data = JSON.parse(event.data) as TaskProgressEvent;

        // Route direct child events to their parent (flat 2-level hierarchy)
        if (data.parent_task_id && taskIdSet.has(data.parent_task_id)) {
          const rootTaskId = data.parent_task_id;
          if (!childProgressMap[rootTaskId]) childProgressMap[rootTaskId] = {};
          childProgressMap[rootTaskId] = {
            ...childProgressMap[rootTaskId],
            [data.task_id]: data
          };
          // When child reaches terminal status, re-fetch parent task data
          if (isTerminalStatus(data.status)) {
            fetchTaskData(rootTaskId);
          }
          return;
        }

        // Only track events for our task IDs
        if (!taskIdSet.has(data.task_id)) return;

        // Don't let a late non-terminal event overwrite a terminal status
        // (fire-and-forget progress broadcasts can race with the completion event)
        const existing = taskProgressMap[data.task_id];
        if (existing && isTerminalStatus(existing.status) && !isTerminalStatus(data.status)) {
          return;
        }

        taskProgressMap[data.task_id] = data;

        if (isTerminalStatus(data.status)) {
          // Clear cancelling state if this task was in the cancelling set
          if (cancellingTaskIds.has(data.task_id)) {
            cancellingTaskIds = new Set([...cancellingTaskIds].filter((id) => id !== data.task_id));
          }
          // Refresh task data on completion to get final state and children_summary
          fetchTaskData(data.task_id);
          onAllDone?.();
        }
      } catch {
        // Ignore malformed events
      }
    };

    eventSource.addEventListener('task:progress', handleEvent);
    eventSource.addEventListener('task:complete', handleEvent);
    eventSource.addEventListener('task:error', handleEvent);
    eventSource.addEventListener('task:cancelled', handleEvent);

    eventSource.onerror = () => {
      // Reconnect handled by EventSource automatically
    };
  }

  function dismissTask(taskId: string) {
    taskIds = taskIds.filter((id) => id !== taskId);
    const updatedProgress = { ...taskProgressMap };
    delete updatedProgress[taskId];
    taskProgressMap = updatedProgress;
    const updatedData = { ...taskDataMap };
    delete updatedData[taskId];
    taskDataMap = updatedData;
    const updatedChildProgress = { ...childProgressMap };
    delete updatedChildProgress[taskId];
    childProgressMap = updatedChildProgress;
    if (cancellingTaskIds.has(taskId)) {
      cancellingTaskIds = new Set([...cancellingTaskIds].filter((id) => id !== taskId));
    }
  }

  function handleTaskCancelling(taskId: string) {
    cancellingTaskIds = new Set([...cancellingTaskIds, taskId]);
    onAllDone?.();
  }

  // --- Helpers to resolve the task type for a tracked ID ---
  function getTaskType(id: string): string | undefined {
    return taskDataMap[id]?.task_type ?? taskProgressMap[id]?.task_type;
  }

  function getStatus(id: string): string {
    return taskProgressMap[id]?.status ?? taskDataMap[id]?.status ?? 'pending';
  }

  // --- Effective status: accounts for active children on terminal roots ---
  function getEffectiveStatus(id: string): string {
    const rootStatus = getStatus(id);
    if (isTerminalStatus(rootStatus)) {
      const cs = childrenSummaryMap[id];
      if (cs && (cs.running > 0 || cs.pending > 0)) return 'processing';
    }
    return rootStatus;
  }

  // --- Separate file-import tasks for batch display ---
  function sortByTerminalFirst(a: string, b: string): number {
    const effA = getEffectiveStatus(a);
    const effB = getEffectiveStatus(b);
    return (isTerminalStatus(effA) ? 1 : 0) - (isTerminalStatus(effB) ? 1 : 0);
  }

  const fileImportIds = $derived(
    taskIds.filter((id) => getTaskType(id) === 'import_file')
  );
  const otherTaskIds = $derived.by(() => {
    const fileSet = new Set(fileImportIds);
    return taskIds.filter((id) => !fileSet.has(id)).sort(sortByTerminalFirst);
  });
  const showBatchCard = $derived(fileImportIds.length > 1);

  // Batch aggregate stats with stage-aware counting
  const batchStageCounts = $derived.by(() => {
    let done = 0,
      scanning = 0,
      resolving = 0,
      importing = 0,
      processing = 0,
      failed = 0,
      pending = 0;
    for (const id of fileImportIds) {
      const s = getEffectiveStatus(id);
      if (s === 'completed') { done++; continue; }
      if (s === 'failed') { failed++; continue; }
      if (s === 'pending') { pending++; continue; }
      // For running/processing, try to resolve stage from child SSE data
      const children = childProgressMap[id];
      if (children) {
        let stage = 'importing';
        for (const child of Object.values(children)) {
          if (!isTerminalStatus(child.status)) {
            stage = taskTypeStageName(child.task_type ?? '');
            break;
          }
        }
        if (stage === 'scanning') scanning++;
        else if (stage === 'resolving') resolving++;
        else importing++;
      } else if (s === 'processing') {
        processing++;
      } else {
        importing++;
      }
    }
    const total = fileImportIds.length;
    const allTerminal = done + failed === total && total > 0;
    return { done, scanning, resolving, importing, processing, failed, pending, total, allTerminal };
  });
  const batchPct = $derived(
    batchStageCounts.total > 0
      ? Math.round(((batchStageCounts.done + batchStageCounts.failed) / batchStageCounts.total) * 100)
      : 0
  );
  const batchStatus = $derived(
    batchStageCounts.allTerminal
      ? batchStageCounts.failed > 0
        ? 'failed'
        : 'completed'
      : batchStageCounts.importing > 0
        ? 'running'
        : batchStageCounts.scanning > 0 || batchStageCounts.resolving > 0 || batchStageCounts.processing > 0
          ? 'processing'
          : 'pending'
  );

  // Task IDs rendered as individual TaskCards (everything when no batch, or non-file-import when batching)
  const individualTaskIds = $derived(
    showBatchCard ? otherTaskIds : [...taskIds].sort(sortByTerminalFirst)
  );

  let batchExpanded = $state(false);

  function dismissBatch() {
    const ids = [...fileImportIds];
    for (const id of ids) {
      dismissTask(id);
    }
    batchExpanded = false;
  }
</script>

{#if hasActiveTasks}
  <Card>
    <CardHeader>
      <CardTitle>Task Progress</CardTitle>
    </CardHeader>
    <CardContent class="space-y-3">
      <!-- Batch card for multi-file uploads -->
      {#if showBatchCard}
        <div class="rounded-lg border border-border p-3">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium">File Import</span>
              <span class="text-xs text-muted-foreground">({batchStageCounts.total} files)</span>
              <TaskStatusBadge status={batchStatus} />
            </div>
            <div class="flex items-center gap-1">
              {#if batchStageCounts.allTerminal}
                <Button
                  variant="ghost"
                  size="icon-sm"
                  class="size-6"
                  onclick={dismissBatch}
                  aria-label="Dismiss all"
                >
                  <XIcon class="size-3" />
                </Button>
              {/if}
              {#if batchStageCounts.failed > 0 || !batchStageCounts.allTerminal}
                <Button
                  variant="ghost"
                  size="icon-sm"
                  class="size-6"
                  onclick={() => (batchExpanded = !batchExpanded)}
                  aria-label={batchExpanded ? 'Collapse' : 'Expand'}
                >
                  <ChevronDownIcon class="size-3 transition-transform {batchExpanded ? 'rotate-180' : ''}" />
                </Button>
              {/if}
            </div>
          </div>

          {#if !batchStageCounts.allTerminal}
            <div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
              <div
                class="h-full rounded-full transition-all duration-300 {progressBarColor(batchStatus)}"
                style="width: {batchPct}%"
              ></div>
            </div>
          {/if}

          <div class="mt-1 flex flex-wrap gap-2 text-xs">
            {#if batchStageCounts.done > 0}
              <span class="text-green-600 dark:text-green-400">{batchStageCounts.done} done</span>
            {/if}
            {#if batchStageCounts.resolving > 0}
              <span class="text-blue-600 dark:text-blue-400">{batchStageCounts.resolving} resolving</span>
            {/if}
            {#if batchStageCounts.scanning > 0}
              <span class="text-blue-600 dark:text-blue-400">{batchStageCounts.scanning} scanning</span>
            {/if}
            {#if batchStageCounts.importing > 0}
              <span class="text-blue-600 dark:text-blue-400">{batchStageCounts.importing} importing</span>
            {/if}
            {#if batchStageCounts.processing > 0}
              <span class="text-blue-600 dark:text-blue-400">{batchStageCounts.processing} processing</span>
            {/if}
            {#if batchStageCounts.pending > 0}
              <span class="text-muted-foreground">{batchStageCounts.pending} queued</span>
            {/if}
            {#if batchStageCounts.failed > 0}
              <span class="text-destructive">{batchStageCounts.failed} failed</span>
            {/if}
          </div>

          <!-- Expanded: show per-file rows with stage + message -->
          {#if batchExpanded}
            <div class="mt-2 space-y-1 border-t border-border pt-2">
              {#each fileImportIds as fid (fid)}
                {@const fStatus = getEffectiveStatus(fid)}
                {@const fTask = taskDataMap[fid]}
                {@const fProgress = taskProgressMap[fid]}
                {@const fChildren = childProgressMap[fid]}
                {@const fFilename = (() => {
                  const msg = fProgress?.message ?? fTask?.message ?? '';
                  const match = msg.match(/[^/\\]+$/);
                  return match ? match[0] : fTask?.id ?? fid;
                })()}
                {@const fStage = (() => {
                  if (fStatus === 'completed') return 'done';
                  if (fStatus === 'failed') return 'failed';
                  if (fStatus === 'pending') return 'queued';
                  if (fChildren) {
                    for (const child of Object.values(fChildren)) {
                      if (!isTerminalStatus(child.status)) {
                        return taskTypeStageName(child.task_type ?? '');
                      }
                    }
                  }
                  if (fStatus === 'running') return 'importing';
                  return 'processing';
                })()}
                {@const fChildMsg = (() => {
                  if (!fChildren) return null;
                  for (const child of Object.values(fChildren)) {
                    if (!isTerminalStatus(child.status)) return child.message;
                  }
                  return null;
                })()}
                <div class="flex items-center gap-2 text-xs">
                  {#if fStatus === 'completed'}
                    <span class="text-green-600 dark:text-green-400">&#10003;</span>
                  {:else if fStatus === 'failed'}
                    <span class="text-destructive">&#10007;</span>
                  {:else}
                    <span class="text-blue-600 dark:text-blue-400 animate-spin-slow">&#10227;</span>
                  {/if}
                  <span class="w-32 truncate font-medium" title={fFilename}>{fFilename}</span>
                  <span class="capitalize text-muted-foreground">{fStage}</span>
                  {#if fChildMsg}
                    <span class="flex-1 truncate text-muted-foreground">{fChildMsg}</span>
                  {:else if fStatus === 'failed'}
                    <span class="flex-1 truncate text-destructive">
                      {fProgress?.error ?? fTask?.error_message ?? 'Unknown error'}
                    </span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      <!-- Individual task cards -->
      {#each individualTaskIds as taskId (taskId)}
        <TaskCard
          task={taskDataMap[taskId] ?? null}
          progress={taskProgressMap[taskId] ?? null}
          childrenSummary={childrenSummaryMap[taskId] ?? null}
          childProgressEntries={childProgressMap[taskId] ?? null}
          cancelling={cancellingTaskIds.has(taskId)}
          onDismiss={() => dismissTask(taskId)}
          onCancelling={() => handleTaskCancelling(taskId)}
        />
      {/each}
    </CardContent>
  </Card>
{/if}
