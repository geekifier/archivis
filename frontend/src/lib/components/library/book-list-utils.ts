import type { BookSummary, SortField } from '$lib/api/types.js';

/** Maps TanStack column IDs to API sort fields. */
export const columnToSortField: Record<string, SortField> = {
	title: 'title',
	added_at: 'added_at',
	metadata_status: 'metadata_status',
	authors: 'author',
	series: 'series'
};

/** Status badge configuration for each metadata status. */
export const statusConfig: Record<string, { label: string; class: string }> = {
	identified: {
		label: 'Identified',
		class: 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
	},
	needs_review: {
		label: 'Needs Review',
		class: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400'
	},
	unidentified: {
		label: 'Unidentified',
		class: 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400'
	}
};

/** Format an ISO date string for table display. */
export function formatDate(iso: string): string {
	return new Date(iso).toLocaleDateString(undefined, {
		year: 'numeric',
		month: 'short',
		day: 'numeric'
	});
}

/** Join a book's author names into a comma-separated string. */
export function formatAuthors(book: BookSummary): string {
	return book.authors?.map((a) => a.name).join(', ') ?? '';
}

/** Format the primary series info for display. */
export function formatSeries(book: BookSummary): string {
	if (!book.series || book.series.length === 0) return '';
	const s = book.series[0];
	return s.position != null ? `${s.name} #${s.position}` : s.name;
}

/** Extract uppercase format labels from a book's files. */
export function formatFormats(book: BookSummary): string[] {
	return book.files?.map((f) => f.format.toUpperCase()) ?? [];
}
