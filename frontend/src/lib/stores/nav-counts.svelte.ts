import { api, onCountsChanged } from '$lib/api/index.js';

function createNavCounts() {
  let duplicateCount = $state<number | null>(null);
  let needsReviewCount = $state<number | null>(null);
  let unidentifiedCount = $state<number | null>(null);
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  async function refreshAll() {
    try {
      const counts = await api.ui.sidebarCounts();
      duplicateCount = counts.duplicates;
      needsReviewCount = counts.needs_review;
      unidentifiedCount = counts.unidentified;
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
    duplicateCount = needsReviewCount = unidentifiedCount = null;
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
    refresh,
    invalidate,
    reset
  };
}

export const navCounts = createNavCounts();
