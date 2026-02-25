import type { TaskStatus, TaskType } from '$lib/api/index.js';

export function taskStatusLabel(status: TaskStatus): string {
	switch (status) {
		case 'pending':
			return 'Pending';
		case 'running':
			return 'Running';
		case 'completed':
			return 'Completed';
		case 'failed':
			return 'Failed';
		case 'cancelled':
			return 'Cancelled';
		default:
			return status;
	}
}

export function taskTypeLabel(taskType: TaskType | string): string {
	switch (taskType) {
		case 'import_file':
			return 'File Import';
		case 'import_directory':
			return 'Directory Import';
		case 'scan_isbn':
			return 'ISBN Scan';
		case 'identify_book':
			return 'Identification';
		default:
			return taskType;
	}
}

export function statusColorClass(status: TaskStatus | string): string {
	switch (status) {
		case 'completed':
			return 'text-green-600 dark:text-green-400';
		case 'failed':
			return 'text-destructive';
		case 'running':
			return 'text-blue-600 dark:text-blue-400';
		case 'cancelled':
			return 'text-amber-600 dark:text-amber-400';
		default:
			return 'text-muted-foreground';
	}
}

export function statusBgClass(status: TaskStatus | string): string {
	switch (status) {
		case 'completed':
			return 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400';
		case 'failed':
			return 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400';
		case 'running':
			return 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400';
		case 'cancelled':
			return 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400';
		default:
			return 'bg-muted text-muted-foreground';
	}
}

export function progressBarColor(status: TaskStatus | string): string {
	switch (status) {
		case 'failed':
			return 'bg-destructive';
		case 'cancelled':
			return 'bg-amber-500';
		default:
			return 'bg-primary';
	}
}

export function formatRelativeTime(dateStr: string): string {
	const date = new Date(dateStr);
	const now = new Date();
	const diffMs = now.getTime() - date.getTime();
	const diffSec = Math.floor(diffMs / 1000);

	if (diffSec < 60) return 'just now';
	const diffMin = Math.floor(diffSec / 60);
	if (diffMin < 60) return `${diffMin}m ago`;
	const diffHr = Math.floor(diffMin / 60);
	if (diffHr < 24) return `${diffHr}h ago`;
	const diffDay = Math.floor(diffHr / 24);
	return `${diffDay}d ago`;
}

export function formatElapsedTime(startedAt: string): string {
	const start = new Date(startedAt);
	const now = new Date();
	const diffSec = Math.floor((now.getTime() - start.getTime()) / 1000);

	if (diffSec < 60) return `${diffSec}s`;
	const min = Math.floor(diffSec / 60);
	const sec = diffSec % 60;
	if (min < 60) return `${min}m ${sec}s`;
	const hr = Math.floor(min / 60);
	const remMin = min % 60;
	return `${hr}h ${remMin}m`;
}

export function isTerminalStatus(status: TaskStatus): boolean {
	return status === 'completed' || status === 'failed' || status === 'cancelled';
}
