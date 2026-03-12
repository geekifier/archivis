import { api, onCountsChanged } from '$lib/api/index.js';

function createNavCounts() {
  let duplicateCount = $state<number | null>(null);
  let needsReviewCount = $state<number | null>(null);
  let unidentifiedCount = $state<number | null>(null);
  let activeTaskCount = $state<number | null>(null);
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  async function refreshAll() {
    try {
      const counts = await api.ui.sidebarCounts();
      duplicateCount = counts.duplicates;
      needsReviewCount = counts.needs_review;
      unidentifiedCount = counts.unidentified;
      activeTaskCount = counts.active_tasks;
    } catch {
      // Silently ignore — keep previous counts
    }
  }

  function refresh() {
    clearTimeout(debounceTimer);
    refreshAll();
  }

  function invalidate() {
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(refreshAll, 300);
  }

  function reset() {
    clearTimeout(debounceTimer);
    duplicateCount = needsReviewCount = unidentifiedCount = activeTaskCount = null;
  }

  // Auto-register with API layer
  onCountsChanged(invalidate);

  return {
    get duplicateCount() {
      return duplicateCount;
    },
    get needsReviewCount() {
      return needsReviewCount;
    },
    get unidentifiedCount() {
      return unidentifiedCount;
    },
    get activeTaskCount() {
      return activeTaskCount;
    },
    refresh,
    invalidate,
    reset
  };
}

export const navCounts = createNavCounts();
