import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { navCounts } from './nav-counts.svelte.js';

// Shared ref for the hook callback — vi.hoisted lifts this above mock hoisting
// so both the mock factory and test code can access it without fake exports.
const hookRef = vi.hoisted(() => ({ current: null as (() => void) | null }));

// Mock the API module
vi.mock('$lib/api/index.js', () => ({
  api: {
    ui: {
      sidebarCounts: vi.fn()
    }
  },
  onCountsChanged: (fn: () => void) => {
    hookRef.current = fn;
    return () => {
      if (hookRef.current === fn) hookRef.current = null;
    };
  }
}));

// Import after mock is set up
import { api } from '$lib/api/index.js';

const mockSidebarCounts = vi.mocked(api.ui.sidebarCounts);

describe('navCounts store', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    navCounts.reset();
    mockSidebarCounts.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('refresh()', () => {
    it('calls api.ui.sidebarCounts() and updates all three counts', async () => {
      mockSidebarCounts.mockResolvedValueOnce({
        duplicates: 3,
        needs_review: 7,
        unidentified: 12,
        active_tasks: 0
      });

      navCounts.refresh();
      await vi.runAllTimersAsync();

      expect(mockSidebarCounts).toHaveBeenCalledOnce();
      expect(navCounts.duplicateCount).toBe(3);
      expect(navCounts.needsReviewCount).toBe(7);
      expect(navCounts.unidentifiedCount).toBe(12);
    });

    it('keeps previous counts on API error', async () => {
      // First set some counts
      mockSidebarCounts.mockResolvedValueOnce({
        duplicates: 5,
        needs_review: 2,
        unidentified: 1,
        active_tasks: 0
      });
      navCounts.refresh();
      await vi.runAllTimersAsync();

      expect(navCounts.duplicateCount).toBe(5);

      // Now fail
      mockSidebarCounts.mockRejectedValueOnce(new Error('Network error'));
      navCounts.refresh();
      await vi.runAllTimersAsync();

      // Counts should be unchanged
      expect(navCounts.duplicateCount).toBe(5);
      expect(navCounts.needsReviewCount).toBe(2);
      expect(navCounts.unidentifiedCount).toBe(1);
    });

    it('cancels pending invalidate() timer', async () => {
      mockSidebarCounts.mockResolvedValue({
        duplicates: 1,
        needs_review: 1,
        unidentified: 1,
        active_tasks: 0
      });

      // Start a debounced invalidate
      navCounts.invalidate();

      // Before debounce fires, call refresh
      navCounts.refresh();
      await vi.runAllTimersAsync();

      // Should only have been called once (the refresh), not twice
      expect(mockSidebarCounts).toHaveBeenCalledOnce();
    });
  });

  describe('invalidate()', () => {
    it('debounces: 3 rapid calls result in 1 API call after 300ms', async () => {
      mockSidebarCounts.mockResolvedValue({
        duplicates: 1,
        needs_review: 1,
        unidentified: 1,
        active_tasks: 0
      });

      navCounts.invalidate();
      navCounts.invalidate();
      navCounts.invalidate();

      // Not called yet
      expect(mockSidebarCounts).not.toHaveBeenCalled();

      // Advance past debounce
      await vi.advanceTimersByTimeAsync(300);

      expect(mockSidebarCounts).toHaveBeenCalledOnce();
    });

    it('responds to counts-changed hook by invalidating', async () => {
      mockSidebarCounts.mockResolvedValue({
        duplicates: 2,
        needs_review: 3,
        unidentified: 4,
        active_tasks: 0
      });

      expect(hookRef.current).toBeTypeOf('function');

      hookRef.current?.();
      await vi.advanceTimersByTimeAsync(300);

      expect(mockSidebarCounts).toHaveBeenCalledOnce();
      expect(navCounts.duplicateCount).toBe(2);
    });
  });

  describe('reset()', () => {
    it('clears all counts to null', async () => {
      mockSidebarCounts.mockResolvedValueOnce({
        duplicates: 5,
        needs_review: 3,
        unidentified: 8,
        active_tasks: 0
      });
      navCounts.refresh();
      await vi.runAllTimersAsync();

      expect(navCounts.duplicateCount).toBe(5);

      navCounts.reset();

      expect(navCounts.duplicateCount).toBeNull();
      expect(navCounts.needsReviewCount).toBeNull();
      expect(navCounts.unidentifiedCount).toBeNull();
    });
  });
});
