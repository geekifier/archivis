import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { goto } from '$app/navigation';
import { setSessionToken, getSessionToken, api, onCountsChanged } from './client.js';
import { ApiError } from './errors.js';

function mockResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' }
  });
}

describe('API client', () => {
  let fetchSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    fetchSpy = vi.spyOn(globalThis, 'fetch');
    localStorage.clear();
    setSessionToken(null);
    vi.mocked(goto).mockReset();
  });

  afterEach(() => {
    fetchSpy.mockRestore();
  });

  describe('token management', () => {
    it('setSessionToken stores token in localStorage and in-memory', () => {
      setSessionToken('abc');
      expect(localStorage.getItem('archivis-session')).toBe('abc');
      expect(getSessionToken()).toBe('abc');
    });

    it('setSessionToken(null) removes from localStorage', () => {
      setSessionToken('abc');
      setSessionToken(null);
      expect(localStorage.getItem('archivis-session')).toBeNull();
      expect(getSessionToken()).toBeNull();
    });

    it('getSessionToken reads from localStorage if in-memory is null', () => {
      // Set directly in localStorage bypassing setSessionToken
      localStorage.setItem('archivis-session', 'from-storage');
      // Clear the in-memory token
      setSessionToken(null);
      // Now set it in localStorage again to simulate a pre-existing value
      localStorage.setItem('archivis-session', 'from-storage');
      expect(getSessionToken()).toBe('from-storage');
    });
  });

  describe('request()', () => {
    it('attaches Authorization header when token is set', async () => {
      setSessionToken('my-token');
      fetchSpy.mockResolvedValueOnce(mockResponse({ setup_required: false }));

      await api.auth.status();

      expect(fetchSpy).toHaveBeenCalledOnce();
      const [, init] = fetchSpy.mock.calls[0];
      expect((init?.headers as Record<string, string>)['Authorization']).toBe('Bearer my-token');
    });

    it('does NOT attach Authorization header when no token', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ setup_required: false }));

      await api.auth.status();

      const [, init] = fetchSpy.mock.calls[0];
      expect((init?.headers as Record<string, string>)['Authorization']).toBeUndefined();
    });

    it('sets Content-Type when body is provided', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({ token: 'tok', user: { id: '1', username: 'admin' } })
      );

      await api.auth.login({ username: 'admin', password: 'pass' });

      const [, init] = fetchSpy.mock.calls[0];
      expect((init?.headers as Record<string, string>)['Content-Type']).toBe('application/json');
    });

    it('does NOT set Content-Type when no body', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ setup_required: false }));

      await api.auth.status();

      const [, init] = fetchSpy.mock.calls[0];
      expect((init?.headers as Record<string, string>)['Content-Type']).toBeUndefined();
    });

    it('returns parsed JSON on success', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ setup_required: true }));

      const result = await api.auth.status();

      expect(result).toEqual({ setup_required: true });
    });

    it('handles 204 No Content', async () => {
      fetchSpy.mockResolvedValueOnce(new Response(null, { status: 204 }));

      const result = await api.auth.logout();

      expect(result).toBeUndefined();
    });

    it('throws ApiError on non-ok response', async () => {
      fetchSpy.mockResolvedValueOnce(
        new Response(JSON.stringify({ error: { status: 400, message: 'Bad input' } }), {
          status: 400,
          headers: { 'Content-Type': 'application/json' }
        })
      );

      await expect(api.auth.status()).rejects.toThrow(ApiError);
    });

    it('on 401: clears token and calls goto(/login)', async () => {
      setSessionToken('expired-token');

      // Set window.location.pathname to something other than /login
      Object.defineProperty(window, 'location', {
        value: { pathname: '/books', href: 'http://localhost/books' },
        writable: true,
        configurable: true
      });

      fetchSpy.mockResolvedValueOnce(
        new Response(JSON.stringify({ error: { status: 401, message: 'Unauthorized' } }), {
          status: 401,
          headers: { 'Content-Type': 'application/json' }
        })
      );

      await expect(api.auth.status()).rejects.toThrow(ApiError);

      expect(getSessionToken()).toBeNull();
      expect(goto).toHaveBeenCalledWith('/login');
    });

    it('on 401: does NOT call goto when already on /login', async () => {
      setSessionToken('expired-token');

      Object.defineProperty(window, 'location', {
        value: { pathname: '/login', href: 'http://localhost/login' },
        writable: true,
        configurable: true
      });

      fetchSpy.mockResolvedValueOnce(
        new Response(JSON.stringify({ error: { status: 401, message: 'Unauthorized' } }), {
          status: 401,
          headers: { 'Content-Type': 'application/json' }
        })
      );

      await expect(api.auth.status()).rejects.toThrow(ApiError);

      expect(getSessionToken()).toBeNull();
      expect(goto).not.toHaveBeenCalled();
    });
  });

  describe('api.books.list()', () => {
    it('calls GET /api/books with no params', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({ items: [], total: 0, page: 1, per_page: 20, total_pages: 0 })
      );

      await api.books.list();

      const [url] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/books');
    });

    it('builds query params correctly', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({ items: [], total: 0, page: 2, per_page: 20, total_pages: 1 })
      );

      await api.books.list({ page: 2, sort_by: 'title' });

      const [url] = fetchSpy.mock.calls[0];
      expect(url).toContain('/api/books?');
      const parsed = new URL(url as string, 'http://localhost');
      expect(parsed.searchParams.get('page')).toBe('2');
      expect(parsed.searchParams.get('sort_by')).toBe('title');
    });

    it('skips null/undefined/empty params', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({ items: [], total: 0, page: 1, per_page: 20, total_pages: 0 })
      );

      await api.books.list({ page: 1, q: '', format: undefined });

      const [url] = fetchSpy.mock.calls[0];
      const parsed = new URL(url as string, 'http://localhost');
      expect(parsed.searchParams.get('q')).toBeNull();
      expect(parsed.searchParams.get('format')).toBeNull();
      expect(parsed.searchParams.get('page')).toBe('1');
    });
  });

  describe('api.stats.get()', () => {
    it('calls GET /api/stats', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({
          generated_at: '2026-01-01T00:00:00Z',
          library: {
            books: 0,
            files: 0,
            total_file_size: 0,
            average_files_per_book: 0,
            files_by_format: [],
            metadata_status: []
          },
          usage: {
            tasks_total: 0,
            tasks_last_24h: 0,
            tasks_by_status: [],
            tasks_by_type: [],
            pending_duplicates: 0,
            pending_candidates: 0
          },
          db: null
        })
      );

      await api.stats.get();

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/stats');
      expect(init?.method).toBe('GET');
    });
  });

  describe('api.auth.login()', () => {
    it('calls POST /api/auth/login with body', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({
          token: 'test-token',
          user: {
            id: '1',
            username: 'admin',
            email: null,
            role: 'admin',
            is_active: true,
            created_at: '2024-01-01'
          }
        })
      );

      await api.auth.login({ username: 'admin', password: 'secret' });

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/auth/login');
      expect(init?.method).toBe('POST');
      expect(JSON.parse(init?.body as string)).toEqual({
        username: 'admin',
        password: 'secret'
      });
    });

    it('stores the returned token automatically', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({
          token: 'test-token',
          user: {
            id: '1',
            username: 'admin',
            email: null,
            role: 'admin',
            is_active: true,
            created_at: '2024-01-01'
          }
        })
      );

      await api.auth.login({ username: 'admin', password: 'secret' });

      expect(getSessionToken()).toBe('test-token');
    });
  });

  describe('api.auth.logout()', () => {
    it('calls POST /api/auth/logout', async () => {
      fetchSpy.mockResolvedValueOnce(new Response(null, { status: 204 }));

      await api.auth.logout();

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/auth/logout');
      expect(init?.method).toBe('POST');
    });

    it('clears token even if the request fails', async () => {
      setSessionToken('my-token');

      fetchSpy.mockResolvedValueOnce(
        new Response(JSON.stringify({ error: { status: 500, message: 'Server error' } }), {
          status: 500,
          headers: { 'Content-Type': 'application/json' }
        })
      );

      // logout catches the error internally in the finally block and still clears the token
      // However, the request() function will throw due to 500 -> but logout wraps in try/finally
      // Actually looking at the code: logout does try { await request } finally { setSessionToken(null) }
      // The request throws on 500, but logout's finally still runs. However, the throw propagates.
      await expect(api.auth.logout()).rejects.toThrow(ApiError);

      expect(getSessionToken()).toBeNull();
    });
  });

  describe('counts-changed hook', () => {
    it('onCountsChanged() supports multiple listeners', async () => {
      const hookA = vi.fn();
      const hookB = vi.fn();
      const unlistenA = onCountsChanged(hookA);
      const unlistenB = onCountsChanged(hookB);
      fetchSpy.mockResolvedValueOnce(new Response(null, { status: 204 }));

      await api.duplicates.dismiss('link-1');

      expect(hookA).toHaveBeenCalledOnce();
      expect(hookB).toHaveBeenCalledOnce();
      unlistenA();
      unlistenB();
    });

    it('onCountsChanged() unsubscribe removes only that listener', async () => {
      const hookA = vi.fn();
      const hookB = vi.fn();
      const unlistenA = onCountsChanged(hookA);
      const unlistenB = onCountsChanged(hookB);
      unlistenA();
      fetchSpy.mockResolvedValueOnce(new Response(null, { status: 204 }));

      await api.duplicates.dismiss('link-1');

      expect(hookA).not.toHaveBeenCalled();
      expect(hookB).toHaveBeenCalledOnce();
      unlistenB();
    });

    it('dismiss calls hook on success', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(new Response(null, { status: 204 }));

      await api.duplicates.dismiss('link-1');

      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('merge calls hook on success', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'merged-book' }));

      await api.duplicates.merge('link-1', {
        primary_id: 'a',
        secondary_id: 'b'
      });

      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('flag calls hook on success', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'new-link' }));

      await api.duplicates.flag('book-a', 'book-b');

      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('books.update calls hook for metadata edits', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'book-1' }));

      await api.books.update('book-1', { title: 'New Title' });

      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('books.refreshMetadata uses the refresh endpoint', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ task_id: 'task-1' }));

      await api.books.refreshMetadata('book-1');

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/books/book-1/refresh-metadata');
      expect(init?.method).toBe('POST');
      expect(hook).not.toHaveBeenCalled();
      unlisten();
    });

    it('books.protectFields sends the selected metadata fields and calls the hook', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'book-1' }));

      await api.books.protectFields('book-1', ['title', 'authors']);

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/books/book-1/protect-fields');
      expect(init?.method).toBe('POST');
      expect(JSON.parse(init?.body as string)).toEqual({ fields: ['title', 'authors'] });
      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('books.batchUpdate calls hook for batch metadata edits', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ updated_count: 1, errors: [] }));

      await api.books.batchUpdate({
        book_ids: ['book-1'],
        updates: { language: 'en' }
      });

      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('resolution.refreshAll uses the new bulk refresh endpoint', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(mockResponse({ count: 1, task_ids: ['task-1'] }));

      await api.resolution.refreshAll();

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/books/refresh-metadata/all');
      expect(init?.method).toBe('POST');
      expect(hook).not.toHaveBeenCalled();
      unlisten();
    });

    it('books.delete calls hook on success', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(new Response(null, { status: 204 }));

      await api.books.delete('book-1');

      expect(hook).toHaveBeenCalledOnce();
      unlisten();
    });

    it('mutation does NOT call hook on failure', async () => {
      const hook = vi.fn();
      const unlisten = onCountsChanged(hook);
      fetchSpy.mockResolvedValueOnce(
        new Response(JSON.stringify({ error: { status: 500, message: 'Server error' } }), {
          status: 500,
          headers: { 'Content-Type': 'application/json' }
        })
      );

      await expect(api.duplicates.dismiss('link-1')).rejects.toThrow(ApiError);

      expect(hook).not.toHaveBeenCalled();
      unlisten();
    });
  });

  describe('api.ui.sidebarCounts()', () => {
    it('calls GET /api/ui/sidebar-counts', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({ duplicates: 3, needs_review: 5, unidentified: 2 })
      );

      const result = await api.ui.sidebarCounts();

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/ui/sidebar-counts');
      expect(init?.method).toBe('GET');
      expect(result).toEqual({ duplicates: 3, needs_review: 5, unidentified: 2 });
    });
  });

  describe('api.books.uploadCover()', () => {
    it('sends FormData with the file', async () => {
      setSessionToken('my-token');
      fetchSpy.mockResolvedValueOnce(
        mockResponse({
          id: 'book-1',
          title: 'Test',
          sort_title: 'test',
          has_cover: true
        })
      );

      const file = new File(['image data'], 'cover.jpg', { type: 'image/jpeg' });
      await api.books.uploadCover('book-1', file);

      const [url, init] = fetchSpy.mock.calls[0];
      expect(url).toBe('/api/books/book-1/cover');
      expect(init?.method).toBe('POST');
      expect(init?.body).toBeInstanceOf(FormData);
      const formData = init?.body as FormData;
      expect(formData.get('file')).toBeInstanceOf(File);
    });

    it('does NOT set Content-Type header (browser sets it)', async () => {
      setSessionToken('my-token');
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'book-1' }));

      const file = new File(['data'], 'cover.jpg', { type: 'image/jpeg' });
      await api.books.uploadCover('book-1', file);

      const [, init] = fetchSpy.mock.calls[0];
      expect((init?.headers as Record<string, string>)['Content-Type']).toBeUndefined();
    });

    it('attaches Authorization header', async () => {
      setSessionToken('my-token');
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'book-1' }));

      const file = new File(['data'], 'cover.jpg', { type: 'image/jpeg' });
      await api.books.uploadCover('book-1', file);

      const [, init] = fetchSpy.mock.calls[0];
      expect((init?.headers as Record<string, string>)['Authorization']).toBe('Bearer my-token');
    });
  });
});
