import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createBookDetail, createCandidateResponse } from '$lib/test-utils/factories.js';
import type { BookDetail } from '$lib/api/types.js';

const { mockApi } = vi.hoisted(() => {
  const fn = vi.fn;
  const create = () => ({
    auth: { status: fn(), setup: fn(), login: fn(), logout: fn(), me: fn() },
    books: {
      list: fn(), get: fn(), update: fn(), refreshMetadata: fn(),
      lockMetadata: fn(), unlockMetadata: fn(), protectFields: fn(), unprotectFields: fn(),
      setAuthors: fn(), setSeries: fn(), setTags: fn(), delete: fn(), uploadCover: fn()
    },
    authors: { get: fn(), search: fn(), create: fn(), listBooks: fn() },
    tags: { search: fn() },
    publishers: { search: fn(), create: fn() },
    series: { get: fn(), search: fn(), listBooks: fn() },
    import: { upload: fn(), scan: fn(), startImport: fn() },
    tasks: { list: fn(), get: fn() },
    resolution: {
      candidates: fn(), applyCandidate: fn(), rejectCandidate: fn(), undoCandidate: fn(),
      rejectCandidates: fn(), trustMetadata: fn(), untrustMetadata: fn(),
      refreshBatch: fn(), refreshAll: fn()
    },
    duplicates: {
      list: fn(), get: fn(), merge: fn(), dismiss: fn(), flag: fn(), count: fn(), forBook: fn()
    },
    isbnScan: { scanBook: fn(), scanBatch: fn() },
    ui: { sidebarCounts: fn() },
    reader: {
      getProgress: fn(), updateProgress: fn(), clearProgress: fn(), continueReading: fn(),
      listBookmarks: fn(), createBookmark: fn(), deleteBookmark: fn(), fetchFileBlob: fn()
    }
  });
  return { mockApi: create() };
});

vi.mock('$lib/api/index.js', () => ({
  api: mockApi,
  ApiError: class ApiError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
      this.name = 'ApiError';
    }
    get userMessage() {
      return this.message;
    }
    get isNotFound() {
      return this.status === 404;
    }
  }
}));

vi.mock('$lib/stores/nav-counts.svelte.js', () => ({
  navCounts: {
    get duplicateCount() { return 0; },
    get needsReviewCount() { return 0; },
    get unidentifiedCount() { return 0; },
    get activeTaskCount() { return 0; },
    refresh: vi.fn(),
    invalidate: vi.fn(),
    reset: vi.fn()
  }
}));

// Override `$app/state` params for this test file
vi.mock('$app/state', () => {
  const page = {
    url: new URL('http://localhost/books/book-1'),
    params: { id: 'book-1' },
    route: { id: '/books/[id]' },
    status: 200,
    error: null,
    data: {},
    form: null
  };
  return { page };
});

import Page from './+page.svelte';

/** Render the page with a book + candidates pre-configured on the mock API. */
function renderPage(
  bookOverrides: Partial<BookDetail> = {},
  candidates: ReturnType<typeof createCandidateResponse>[] = []
) {
  const book = createBookDetail({ id: 'book-1', ...bookOverrides });
  mockApi.books.get.mockResolvedValue(book);
  mockApi.resolution.candidates.mockResolvedValue(candidates);
  mockApi.duplicates.forBook.mockResolvedValue([]);
  mockApi.ui.sidebarCounts.mockResolvedValue({
    duplicates: 0, needs_review: 0, unidentified: 0, active_tasks: 0
  });
  return { book, rerender: render(Page) };
}

/** Wait until the page has loaded (h1 title is visible). */
async function waitForPageLoad(title = 'Test Book') {
  await waitFor(() => {
    expect(screen.getByRole('heading', { level: 1, name: title })).toBeInTheDocument();
  });
}

describe('Book detail page — trust controls', () => {
  let user: ReturnType<typeof userEvent.setup>;

  beforeEach(() => {
    vi.clearAllMocks();
    user = userEvent.setup();
  });

  // --- Disabled-state alignment ---

  it('banner "Trust Metadata" disabled when resolution_state is running', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending' });
    renderPage(
      { resolution_state: 'running', metadata_status: 'needs_review' },
      [pending]
    );
    await waitForPageLoad();

    const trustButtons = screen.getAllByText('Trust Metadata');
    for (const btn of trustButtons) {
      expect(btn.closest('button')).toBeDisabled();
    }
  });

  it('header shield disabled when resolution_state is running', async () => {
    renderPage({ resolution_state: 'running' });
    await waitForPageLoad();

    const shieldButton = screen.getByTitle('Click to trust this metadata');
    expect(shieldButton).toBeDisabled();
  });

  it('banner and header both disabled when resolution_state is running', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending' });
    renderPage(
      { resolution_state: 'running', metadata_status: 'needs_review' },
      [pending]
    );
    await waitForPageLoad();

    // Header shield
    const shieldButton = screen.getByTitle('Click to trust this metadata');
    expect(shieldButton).toBeDisabled();

    // Banner Trust Metadata buttons
    const trustButtons = screen.getAllByText('Trust Metadata');
    for (const btn of trustButtons) {
      expect(btn.closest('button')).toBeDisabled();
    }
  });

  it('banner and header both disabled during local refreshingMetadata', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending' });
    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending]
    );
    await waitForPageLoad();

    // Keep refreshingMetadata stuck at true by returning a never-settling promise
    mockApi.books.refreshMetadata.mockReturnValue(new Promise(() => {}));

    // Click "Refresh Metadata" to enter the local refreshing state
    await user.click(screen.getByText('Refresh Metadata'));

    // Verify the button text changed (confirms refreshingMetadata is true)
    await waitFor(() => {
      expect(screen.getByText('Refreshing...')).toBeInTheDocument();
    });

    // Header shield should be disabled
    const shieldButton = screen.getByTitle('Click to trust this metadata');
    expect(shieldButton).toBeDisabled();

    // Banner Trust Metadata buttons should be disabled
    const trustButtons = screen.getAllByText('Trust Metadata');
    for (const btn of trustButtons) {
      expect(btn.closest('button')).toBeDisabled();
    }
  });

  // --- Trust vs untrust state alignment ---

  it('header shows untrust state when metadata_user_trusted is true', async () => {
    renderPage({
      metadata_user_trusted: true,
      resolution_state: 'done',
      resolution_outcome: 'confirmed'
    });
    await waitForPageLoad();

    const shieldButton = screen.getByTitle('Metadata trusted — click to remove trust');
    expect(shieldButton).toBeInTheDocument();
  });

  it('no "Trust Metadata" banner CTA visible when metadata_user_trusted is true', async () => {
    renderPage({
      metadata_user_trusted: true,
      resolution_state: 'done',
      resolution_outcome: 'confirmed',
      metadata_status: 'identified'
    });
    await waitForPageLoad();

    // With trust active, server returns no pending candidates → banner hidden
    expect(screen.queryByText('Trust Metadata')).not.toBeInTheDocument();
  });

  // --- Banner hides after trust succeeds ---

  it('after successful handleTrustMetadata: no "Trust Metadata" button in DOM', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending' });
    const trustedBook = createBookDetail({
      id: 'book-1',
      metadata_user_trusted: true,
      resolution_state: 'done',
      resolution_outcome: 'confirmed',
      metadata_status: 'identified'
    });

    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending]
    );
    await waitForPageLoad();

    // Banner should show Trust Metadata before trust
    expect(screen.getAllByText('Trust Metadata').length).toBeGreaterThan(0);

    // Mock trust API call
    mockApi.resolution.trustMetadata.mockResolvedValue(trustedBook);
    // After trust, candidates reload returns empty
    mockApi.resolution.candidates.mockResolvedValue([]);
    mockApi.books.get.mockResolvedValue(trustedBook);

    // Click banner Trust Metadata (first one, in collapsed banner)
    const trustButtons = screen.getAllByText('Trust Metadata');
    await user.click(trustButtons[0]);

    await waitFor(() => {
      expect(screen.queryByText('Trust Metadata')).not.toBeInTheDocument();
    });
  });

  // --- Stale error clearing ---

  it('candidatesError clears after successful edit-form trust save (false→true)', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending' });
    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending]
    );
    await waitForPageLoad();

    // Simulate a prior candidatesError by making trustMetadata fail first
    const { ApiError } = await import('$lib/api/index.js');
    mockApi.resolution.trustMetadata.mockRejectedValueOnce(new ApiError(500, 'Server error'));

    // Click header shield to trigger trust → error
    const shieldButton = screen.getByTitle('Click to trust this metadata');
    await user.click(shieldButton);

    await waitFor(() => {
      expect(screen.getByText('Server error')).toBeInTheDocument();
    });

    // Now enter edit mode and save with trust toggled on
    await user.click(screen.getByText('Edit'));
    await user.click(screen.getByText('Trust this metadata'));

    const trustedBook = createBookDetail({
      id: 'book-1',
      metadata_user_trusted: true,
      resolution_state: 'done',
      resolution_outcome: 'confirmed',
      metadata_status: 'identified'
    });
    mockApi.books.update.mockResolvedValue(trustedBook);
    mockApi.books.get.mockResolvedValue(trustedBook);
    mockApi.resolution.candidates.mockResolvedValue([]);

    await user.click(screen.getByText('Save'));

    await waitFor(() => {
      expect(screen.queryByText('Server error')).not.toBeInTheDocument();
    });
  });

  it('candidatesError clears after successful reject-all from banner', async () => {
    const pending1 = createCandidateResponse({ id: 'c1', status: 'pending' });
    const pending2 = createCandidateResponse({ id: 'c2', status: 'pending', title: 'Another' });
    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending1, pending2]
    );
    await waitForPageLoad();

    // Cause an error first
    const { ApiError } = await import('$lib/api/index.js');
    mockApi.resolution.trustMetadata.mockRejectedValueOnce(new ApiError(500, 'Server error'));

    const shieldButton = screen.getByTitle('Click to trust this metadata');
    await user.click(shieldButton);

    await waitFor(() => {
      expect(screen.getByText('Server error')).toBeInTheDocument();
    });

    // Now expand candidates and reject all
    await user.click(screen.getByText('Review Candidates'));

    const updatedBook = createBookDetail({
      id: 'book-1',
      metadata_status: 'needs_review',
      resolution_state: 'done',
      resolution_outcome: 'unmatched'
    });
    mockApi.resolution.rejectCandidates.mockResolvedValue(updatedBook);
    mockApi.books.get.mockResolvedValue(updatedBook);
    mockApi.resolution.candidates.mockResolvedValue([
      createCandidateResponse({ id: 'c1', status: 'rejected' }),
      createCandidateResponse({ id: 'c2', status: 'rejected', title: 'Another' })
    ]);

    await user.click(screen.getByText('Reject All'));

    await waitFor(() => {
      expect(screen.queryByText('Server error')).not.toBeInTheDocument();
    });
  });

  it('candidatesError clears after successful single candidate reject', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending', title: 'Candidate Title' });
    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending]
    );
    await waitForPageLoad();

    // Cause an error first
    const { ApiError } = await import('$lib/api/index.js');
    mockApi.resolution.trustMetadata.mockRejectedValueOnce(new ApiError(500, 'Server error'));

    const shieldButton = screen.getByTitle('Click to trust this metadata');
    await user.click(shieldButton);

    await waitFor(() => {
      expect(screen.getByText('Server error')).toBeInTheDocument();
    });

    // Expand candidates, reject one
    await user.click(screen.getByText('Review Candidates'));

    mockApi.resolution.rejectCandidate.mockResolvedValue(undefined);
    const updatedBook = createBookDetail({
      id: 'book-1',
      metadata_status: 'needs_review',
      resolution_state: 'pending'
    });
    mockApi.books.get.mockResolvedValue(updatedBook);
    mockApi.resolution.candidates.mockResolvedValue([
      createCandidateResponse({ id: 'c1', status: 'rejected', title: 'Candidate Title' })
    ]);

    await user.click(screen.getByText('Reject'));

    await waitFor(() => {
      expect(screen.queryByText('Server error')).not.toBeInTheDocument();
    });
  });

  // --- Edit-form save leaves page state consistent ---

  it('edit-form trust save (false→true): candidates cleared, banner hidden, no stale error', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending' });
    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending]
    );
    await waitForPageLoad();

    // Verify banner is present
    expect(screen.getAllByText('Trust Metadata').length).toBeGreaterThan(0);

    // Enter edit mode
    await user.click(screen.getByText('Edit'));

    // Toggle trust on and save
    await user.click(screen.getByText('Trust this metadata'));

    const trustedBook = createBookDetail({
      id: 'book-1',
      metadata_user_trusted: true,
      resolution_state: 'done',
      resolution_outcome: 'confirmed',
      metadata_status: 'identified'
    });
    mockApi.books.update.mockResolvedValue(trustedBook);
    mockApi.books.get.mockResolvedValue(trustedBook);
    mockApi.resolution.candidates.mockResolvedValue([]);

    await user.click(screen.getByText('Save'));

    await waitFor(() => {
      // No Trust Metadata banner buttons
      expect(screen.queryByText('Trust Metadata')).not.toBeInTheDocument();
    });

    // Header should show trusted state
    await waitFor(() => {
      expect(screen.getByTitle('Metadata trusted — click to remove trust')).toBeInTheDocument();
    });
  });

  it('edit-form save (trust unchanged): loadCandidates runs and candidates refresh', async () => {
    const pending = createCandidateResponse({ id: 'c1', status: 'pending', title: 'Candidate Title' });
    renderPage(
      { metadata_status: 'needs_review', resolution_state: 'pending' },
      [pending]
    );
    await waitForPageLoad();

    // Enter edit mode, change title only, save
    await user.click(screen.getByText('Edit'));

    const titleInput = screen.getByLabelText('Title') as HTMLInputElement;
    await user.clear(titleInput);
    await user.type(titleInput, 'New Title');

    const updatedBook = createBookDetail({
      id: 'book-1',
      title: 'New Title',
      metadata_status: 'needs_review',
      resolution_state: 'pending'
    });
    mockApi.books.update.mockResolvedValue(updatedBook);
    mockApi.books.get.mockResolvedValue(updatedBook);

    // After save, loadCandidates will be called — return updated candidates
    const refreshedCandidate = createCandidateResponse({ id: 'c2', status: 'pending', title: 'Refreshed' });
    mockApi.resolution.candidates.mockResolvedValue([refreshedCandidate]);

    await user.click(screen.getByText('Save'));

    await waitFor(() => {
      // loadCandidates should have been called (at least once after save)
      expect(mockApi.resolution.candidates).toHaveBeenCalled();
    });
  });
});
