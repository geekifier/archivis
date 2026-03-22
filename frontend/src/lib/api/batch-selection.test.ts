import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { api } from './client.js';
import { isBatchAsync } from './types.js';
import type { BatchSyncResponse, BatchAsyncResponse } from './types.js';

function mockResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' }
  });
}

describe('batch selection API', () => {
  let fetchSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    fetchSpy = vi.spyOn(globalThis, 'fetch');
  });

  afterEach(() => {
    fetchSpy.mockRestore();
  });

  describe('isBatchAsync', () => {
    it('returns false for sync (200) response shape', () => {
      const sync: BatchSyncResponse = { updated_count: 5, errors: [] };
      expect(isBatchAsync(sync)).toBe(false);
    });

    it('returns true for async (202) response shape', () => {
      const async_: BatchAsyncResponse = {
        task_id: 'task-abc',
        task_type: 'bulk_update',
        matching_count: 500,
        message: 'Bulk update enqueued for 500 books'
      };
      expect(isBatchAsync(async_)).toBe(true);
    });
  });

  describe('batchUpdate', () => {
    it('returns status 200 and sync data for small batches', async () => {
      const syncBody = { updated_count: 3, errors: [] };
      fetchSpy.mockResolvedValueOnce(mockResponse(syncBody, 200));

      const result = await api.books.batchUpdate({
        selection: { mode: 'ids', ids: ['a', 'b', 'c'] },
        updates: { language: 'en' }
      });

      expect(result.status).toBe(200);
      expect(isBatchAsync(result.data)).toBe(false);
      if (!isBatchAsync(result.data)) {
        expect(result.data.updated_count).toBe(3);
      }
    });

    it('returns status 202 and async data for large scopes', async () => {
      const asyncBody = {
        task_id: 'task-123',
        task_type: 'bulk_update',
        matching_count: 1500,
        message: 'Bulk update enqueued for 1,500 books'
      };
      fetchSpy.mockResolvedValueOnce(mockResponse(asyncBody, 202));

      const result = await api.books.batchUpdate({
        selection: { mode: 'scope', scope_token: 'tok', excluded_ids: [] },
        updates: { language: 'fr' }
      });

      expect(result.status).toBe(202);
      expect(isBatchAsync(result.data)).toBe(true);
      if (isBatchAsync(result.data)) {
        expect(result.data.task_id).toBe('task-123');
        expect(result.data.matching_count).toBe(1500);
      }
    });

    it('sends scope selection spec in request body', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ updated_count: 0, errors: [] }));

      await api.books.batchUpdate({
        selection: { mode: 'scope', scope_token: 'my-token', excluded_ids: ['ex1'] },
        updates: { rating: 4.5 }
      });

      const [, init] = fetchSpy.mock.calls[0];
      const body = JSON.parse(init?.body as string);
      expect(body.selection).toEqual({
        mode: 'scope',
        scope_token: 'my-token',
        excluded_ids: ['ex1']
      });
    });
  });

  describe('batchTags', () => {
    it('returns status 200 for sync tag updates', async () => {
      const syncBody = { updated_count: 10, errors: [] };
      fetchSpy.mockResolvedValueOnce(mockResponse(syncBody, 200));

      const result = await api.books.batchTags({
        selection: { mode: 'ids', ids: ['a'] },
        tags: [{ tag_id: 't1' }],
        mode: 'add'
      });

      expect(result.status).toBe(200);
      expect(isBatchAsync(result.data)).toBe(false);
    });

    it('returns status 202 for async tag updates', async () => {
      const asyncBody = {
        task_id: 'task-tags',
        task_type: 'bulk_set_tags',
        matching_count: 200,
        message: 'Bulk tag update enqueued'
      };
      fetchSpy.mockResolvedValueOnce(mockResponse(asyncBody, 202));

      const result = await api.books.batchTags({
        selection: { mode: 'scope', scope_token: 'tok', excluded_ids: [] },
        tags: [{ tag_id: 't2' }],
        mode: 'replace'
      });

      expect(result.status).toBe(202);
      expect(isBatchAsync(result.data)).toBe(true);
      if (isBatchAsync(result.data)) {
        expect(result.data.task_id).toBe('task-tags');
      }
    });
  });

  describe('combined async scalar + tag apply preserves both task IDs', () => {
    it('collects task IDs from both 202 responses', async () => {
      const scalarAsync = {
        task_id: 'task-scalar',
        task_type: 'bulk_update',
        matching_count: 300,
        message: 'Bulk update enqueued'
      };
      const tagAsync = {
        task_id: 'task-tags',
        task_type: 'bulk_set_tags',
        matching_count: 300,
        message: 'Bulk tag update enqueued'
      };

      fetchSpy
        .mockResolvedValueOnce(mockResponse(scalarAsync, 202))
        .mockResolvedValueOnce(mockResponse(tagAsync, 202));

      // Simulate what BatchEditPanel.handleApply does
      const asyncTaskIds: string[] = [];
      const selection = { mode: 'scope' as const, scope_token: 'tok', excluded_ids: [] as string[] };

      const scalarResult = await api.books.batchUpdate({
        selection,
        updates: { language: 'de' }
      });
      if (scalarResult.status === 202 && isBatchAsync(scalarResult.data)) {
        asyncTaskIds.push(scalarResult.data.task_id);
      }

      const tagResult = await api.books.batchTags({
        selection,
        tags: [{ tag_id: 't1' }],
        mode: 'add'
      });
      if (tagResult.status === 202 && isBatchAsync(tagResult.data)) {
        asyncTaskIds.push(tagResult.data.task_id);
      }

      expect(asyncTaskIds).toEqual(['task-scalar', 'task-tags']);
      expect(asyncTaskIds).toHaveLength(2);
    });
  });

  describe('issueSelectionScope', () => {
    it('sends filter state and returns scope token', async () => {
      const responseBody = {
        scope_token: 'signed-token-abc',
        matching_count: 42,
        summary: '42 books matching current filters'
      };
      fetchSpy.mockResolvedValueOnce(mockResponse(responseBody));

      const result = await api.books.issueSelectionScope({
        filters: {
          text_query: 'sanderson',
          author_id: null,
          series_id: null,
          publisher_id: null,
          tag_ids: [],
          tag_match: 'any',
          format: null,
          metadata_status: null,
          resolution_state: null,
          resolution_outcome: null,
          trusted: null,
          locked: null,
          language: 'en',
          year_min: null,
          year_max: null,
          has_cover: null,
          has_description: null,
          has_identifiers: null,
          isbn: null,
          asin: null,
          open_library_id: null,
          hardcover_id: null
        }
      });

      expect(result.scope_token).toBe('signed-token-abc');
      expect(result.matching_count).toBe(42);

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/books/selection-scope');
      expect(init?.method).toBe('POST');
      const body = JSON.parse(init?.body as string);
      expect(body.filters.text_query).toBe('sanderson');
      expect(body.filters.language).toBe('en');
    });
  });
});
