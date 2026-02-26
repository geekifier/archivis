<script lang="ts">
	import type { TaskProgressEvent, TaskResponse, ChildrenSummary } from '$lib/api/index.js';
	import { api, getSessionToken } from '$lib/api/index.js';
	import {
		Card,
		CardContent,
		CardHeader,
		CardTitle
	} from '$lib/components/ui/card/index.js';
	import TaskCard from './TaskCard.svelte';
	import { isTerminalStatus } from './task-utils.js';

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
	let cancellingTaskIds = $state(new Set<string>());
	let eventSource: EventSource | null = null;

	const hasActiveTasks = $derived(taskIds.length > 0);

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
		try {
			const task = await api.tasks.get(taskId);
			taskDataMap = { ...taskDataMap, [taskId]: task };
			if (task.children_summary) {
				childrenSummaryMap = { ...childrenSummaryMap, [taskId]: task.children_summary };
			}
		} catch {
			// Task may not exist yet or be unavailable
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
				// Only track events for our task IDs
				if (!taskIds.includes(data.task_id)) return;

				// Don't let a late non-terminal event overwrite a terminal status
				// (fire-and-forget progress broadcasts can race with the completion event)
				const existing = taskProgressMap[data.task_id];
				if (existing && isTerminalStatus(existing.status) && !isTerminalStatus(data.status)) {
					return;
				}

				taskProgressMap = { ...taskProgressMap, [data.task_id]: data };

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
		if (cancellingTaskIds.has(taskId)) {
			cancellingTaskIds = new Set([...cancellingTaskIds].filter((id) => id !== taskId));
		}
	}

	function handleTaskCancelling(taskId: string) {
		cancellingTaskIds = new Set([...cancellingTaskIds, taskId]);
		onAllDone?.();
	}

	// Sort tasks: running/pending first, then terminal
	const sortedTaskIds = $derived(
		[...taskIds].sort((a, b) => {
			const statusA = taskProgressMap[a]?.status ?? taskDataMap[a]?.status ?? 'pending';
			const statusB = taskProgressMap[b]?.status ?? taskDataMap[b]?.status ?? 'pending';
			const termA = isTerminalStatus(statusA) ? 1 : 0;
			const termB = isTerminalStatus(statusB) ? 1 : 0;
			return termA - termB;
		})
	);
</script>

{#if hasActiveTasks}
	<Card>
		<CardHeader>
			<CardTitle>Import Progress</CardTitle>
		</CardHeader>
		<CardContent class="space-y-3">
			{#each sortedTaskIds as taskId (taskId)}
				<TaskCard
					task={taskDataMap[taskId] ?? null}
					progress={taskProgressMap[taskId] ?? null}
					childrenSummary={childrenSummaryMap[taskId] ?? null}
					cancelling={cancellingTaskIds.has(taskId)}
					onDismiss={() => dismissTask(taskId)}
					onCancelling={() => handleTaskCancelling(taskId)}
				/>
			{/each}
		</CardContent>
	</Card>
{/if}
