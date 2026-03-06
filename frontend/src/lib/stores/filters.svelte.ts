import type { BookFormat, MetadataStatus } from '$lib/api/types.js';

function createFilterStore() {
  let activeFormat = $state<BookFormat | null>(null);
  let activeStatus = $state<MetadataStatus | null>(null);

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
    setFormat,
    setStatus,
    clearFilters
  };
}

export const filters = createFilterStore();
