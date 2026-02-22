import type { BookFormat, MetadataStatus } from '$lib/api/types.js';

function createFilterStore() {
	let activeFormat = $state<BookFormat | null>(null);
	let activeStatus = $state<MetadataStatus | null>(null);
	let needsReviewCount = $state<number | null>(null);
	let unidentifiedCount = $state<number | null>(null);

	const hasActiveFilters = $derived(activeFormat !== null || activeStatus !== null);

	function setFormat(format: BookFormat) {
		activeFormat = activeFormat === format ? null : format;
	}

	function setStatus(status: MetadataStatus) {
		activeStatus = activeStatus === status ? null : status;
	}

	function clearFilters() {
		activeFormat = null;
		activeStatus = null;
	}

	function setNeedsReviewCount(count: number) {
		needsReviewCount = count;
	}

	function setUnidentifiedCount(count: number) {
		unidentifiedCount = count;
	}

	return {
		get activeFormat() {
			return activeFormat;
		},
		get activeStatus() {
			return activeStatus;
		},
		get hasActiveFilters() {
			return hasActiveFilters;
		},
		get needsReviewCount() {
			return needsReviewCount;
		},
		get unidentifiedCount() {
			return unidentifiedCount;
		},
		setFormat,
		setStatus,
		clearFilters,
		setNeedsReviewCount,
		setUnidentifiedCount
	};
}

export const filters = createFilterStore();
