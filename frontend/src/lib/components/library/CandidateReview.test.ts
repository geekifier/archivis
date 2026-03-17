import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { createBookDetail, createCandidateResponse } from '$lib/test-utils/factories.js';
import type { BookDetail } from '$lib/api/types.js';

type ApplyFn = (updated: BookDetail) => void;
type RejectFn = (candidateId: string) => void;
type UndoFn = (updated: BookDetail) => void;

const { mockApi } = vi.hoisted(() => {
  const createMockApiFn = () => ({
    auth: { status: vi.fn(), setup: vi.fn(), login: vi.fn(), logout: vi.fn(), me: vi.fn() },
    books: {
      list: vi.fn(),
      get: vi.fn(),
      update: vi.fn(),
      setAuthors: vi.fn(),
      setSeries: vi.fn(),
      setTags: vi.fn(),
      delete: vi.fn(),
      uploadCover: vi.fn()
    },
    authors: { get: vi.fn(), search: vi.fn(), create: vi.fn(), listBooks: vi.fn() },
    tags: { search: vi.fn() },
    publishers: { search: vi.fn(), create: vi.fn() },
    series: { get: vi.fn(), search: vi.fn(), listBooks: vi.fn() },
    import: { upload: vi.fn(), scan: vi.fn(), startImport: vi.fn() },
    tasks: { list: vi.fn(), get: vi.fn() },
    resolution: {
      book: vi.fn(),
      candidates: vi.fn(),
      applyCandidate: vi.fn(),
      rejectCandidate: vi.fn(),
      undoCandidate: vi.fn(),
      rejectCandidates: vi.fn(),
      trustMetadata: vi.fn(),
      untrustMetadata: vi.fn(),
      batch: vi.fn(),
      all: vi.fn()
    }
  });
  return { mockApi: createMockApiFn() };
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
  }
}));

import CandidateReview from './CandidateReview.svelte';

describe('CandidateReview', () => {
  let user: ReturnType<typeof userEvent.setup>;
  let book: BookDetail;

  beforeEach(() => {
    vi.clearAllMocks();
    user = userEvent.setup();
    book = createBookDetail({
      id: 'book-1',
      title: 'Test Book',
      authors: [
        {
          id: 'a1',
          name: 'Original Author',
          sort_name: 'Author, Original',
          role: 'author',
          position: 0
        }
      ]
    });
  });

  it('shows "No candidates found" when candidates is empty', () => {
    render(CandidateReview, {
      props: {
        book,
        candidates: [],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText('No candidates found for this book.')).toBeInTheDocument();
  });

  it('renders pending candidates with provider name, score, Apply/Reject buttons', () => {
    const candidate = createCandidateResponse({
      id: 'c1',
      provider_name: 'open_library',
      score: 0.85,
      status: 'pending',
      title: 'Candidate Title'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText('Open Library')).toBeInTheDocument();
    expect(screen.getByText('85%')).toBeInTheDocument();
    expect(screen.getByText('Apply')).toBeInTheDocument();
    expect(screen.getByText('Reject')).toBeInTheDocument();
  });

  it('renders dispute pills for disputed candidates', () => {
    const candidate = createCandidateResponse({
      id: 'c1',
      disputes: [
        "Title differs from provider's suggestion",
        'Authors change skipped — no strong identifier match'
      ]
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText("Title differs from provider's suggestion")).toBeInTheDocument();
    expect(
      screen.getByText('Authors change skipped — no strong identifier match')
    ).toBeInTheDocument();
  });

  it('renders rejected candidates section', () => {
    const rejected = createCandidateResponse({
      id: 'c2',
      provider_name: 'open_library',
      status: 'rejected',
      title: 'Rejected Title',
      score: 0.5
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [rejected],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText('Rejected (1)')).toBeInTheDocument();
    expect(screen.getByText('50%')).toBeInTheDocument();
  });

  it('renders applied candidates with Undo button', () => {
    const applied = createCandidateResponse({
      id: 'c3',
      provider_name: 'open_library',
      status: 'applied',
      score: 0.9
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [applied],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    // "Applied" section heading
    expect(screen.getByText('Applied', { selector: 'h4' })).toBeInTheDocument();
    expect(screen.getByText('Undo')).toBeInTheDocument();
  });

  it('Apply calls api.resolution.applyCandidate and onapply callback', async () => {
    const updatedBook = createBookDetail({ title: 'Updated Book' });
    mockApi.resolution.applyCandidate.mockResolvedValue(updatedBook);
    const onapply = vi.fn<ApplyFn>();

    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      title: 'Candidate Title'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply,
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });

    await user.click(screen.getByText('Apply'));

    await vi.waitFor(() => {
      expect(mockApi.resolution.applyCandidate).toHaveBeenCalledWith('book-1', 'c1', undefined);
    });
    await vi.waitFor(() => {
      expect(onapply).toHaveBeenCalledWith(updatedBook);
    });
  });

  it('Reject calls api.resolution.rejectCandidate and onreject callback', async () => {
    mockApi.resolution.rejectCandidate.mockResolvedValue(undefined);
    const onreject = vi.fn<RejectFn>();

    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject,
        onundo: vi.fn<UndoFn>(),
      }
    });

    await user.click(screen.getByText('Reject'));

    await vi.waitFor(() => {
      expect(mockApi.resolution.rejectCandidate).toHaveBeenCalledWith('book-1', 'c1');
    });
    await vi.waitFor(() => {
      expect(onreject).toHaveBeenCalledWith('c1');
    });
  });

  it('Undo calls api.resolution.undoCandidate and onundo callback', async () => {
    const restoredBook = createBookDetail({ title: 'Restored Book' });
    mockApi.resolution.undoCandidate.mockResolvedValue(restoredBook);
    const onundo = vi.fn<UndoFn>();

    const applied = createCandidateResponse({
      id: 'c3',
      status: 'applied'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [applied],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo,
      }
    });

    await user.click(screen.getByText('Undo'));

    await vi.waitFor(() => {
      expect(mockApi.resolution.undoCandidate).toHaveBeenCalledWith('book-1', 'c3');
    });
    await vi.waitFor(() => {
      expect(onundo).toHaveBeenCalledWith(restoredBook);
    });
  });

  it('shows error message when API call fails', async () => {
    mockApi.resolution.applyCandidate.mockRejectedValue(new Error('Network error'));

    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      title: 'Candidate Title'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });

    await user.click(screen.getByText('Apply'));

    await vi.waitFor(() => {
      expect(screen.getByText('Network error')).toBeInTheDocument();
    });
  });

  it('shows metadata lock status when the book is locked', () => {
    render(CandidateReview, {
      props: {
        book: createBookDetail({ metadata_locked: true }),
        candidates: [createCandidateResponse()],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });

    expect(
      screen.getByText(
        'Metadata is locked. Refreshes stay in review-only mode until you unlock the book.'
      )
    ).toBeInTheDocument();
  });

  it('renders tier badge when tier is present', () => {
    const candidate = createCandidateResponse({
        id: 'c1',
        status: 'pending',
        tier: 'strong_id_match',
        title: 'Candidate Title'
    });
    render(CandidateReview, {
        props: {
            book,
            candidates: [candidate],
            onapply: vi.fn<ApplyFn>(),
            onreject: vi.fn<RejectFn>(),
            onundo: vi.fn<UndoFn>(),
            }
    });
    expect(screen.getByText('Strong ID match')).toBeInTheDocument();
  });

  it('treats same authors in different order as a match (dimmed, no checkbox)', () => {
    const bookWithAuthors = createBookDetail({
      id: 'book-1',
      title: 'Test Book',
      authors: [
        { id: 'a1', name: 'Alice Smith', sort_name: 'Smith, Alice', role: 'author', position: 0 },
        { id: 'a2', name: 'Bob Jones', sort_name: 'Jones, Bob', role: 'author', position: 1 }
      ]
    });
    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      title: 'Test Book',
      authors: [
        { name: 'Bob Jones', role: 'author' },
        { name: 'Alice Smith', role: 'author' }
      ]
    });
    render(CandidateReview, {
      props: {
        book: bookWithAuthors,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });

    // Find the Authors row
    const authorsLabel = screen.getByText('Authors');
    const authorsRow = authorsLabel.closest('tr')!;
    // The row should be dimmed (opacity-40)
    expect(authorsRow.className).toContain('opacity-40');
    // No checkbox should be rendered in the row
    expect(authorsRow.querySelector('input[type="checkbox"]')).toBeNull();
  });

  it('renders non-author contributors as separate rows', () => {
    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      authors: [
        { name: 'Andrzej Sapkowski', role: 'author' },
        { name: 'David French', role: 'translator' }
      ]
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText('Translator')).toBeInTheDocument();
    expect(screen.getByText('David French')).toBeInTheDocument();
  });

  it('renders Language and Page Count rows when candidate has values', () => {
    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      title: 'Candidate Title',
      language: 'fr',
      language_label: 'French',
      page_count: 412
    });
    render(CandidateReview, {
      props: {
        book: createBookDetail({ language: 'en', language_label: 'English', page_count: 300 }),
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText('Language')).toBeInTheDocument();
    expect(screen.getByText('English')).toBeInTheDocument();
    expect(screen.getByText('French')).toBeInTheDocument();
    expect(screen.getByText('Pages')).toBeInTheDocument();
    expect(screen.getByText('300')).toBeInTheDocument();
    expect(screen.getByText('412')).toBeInTheDocument();
  });

  it('renders "Merged" badge when `is_composite` is true', () => {
    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      is_composite: true,
      title: 'Composite Candidate'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.getByText('Merged')).toBeInTheDocument();
  });

  it('does not render "Merged" badge when `is_composite` is false', () => {
    const candidate = createCandidateResponse({
      id: 'c1',
      status: 'pending',
      is_composite: false,
      title: 'Regular Candidate'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [candidate],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    expect(screen.queryByText('Merged')).not.toBeInTheDocument();
  });

  it('when a candidate is applied, pending candidates still show Apply/Reject buttons', () => {
    const applied = createCandidateResponse({
      id: 'c1',
      status: 'applied',
      provider_name: 'open_library',
      score: 0.9
    });
    const pending = createCandidateResponse({
      id: 'c2',
      status: 'pending',
      provider_name: 'hardcover',
      score: 0.7,
      title: 'Alternative Title'
    });
    render(CandidateReview, {
      props: {
        book,
        candidates: [applied, pending],
        onapply: vi.fn<ApplyFn>(),
        onreject: vi.fn<RejectFn>(),
        onundo: vi.fn<UndoFn>(),
      }
    });
    // Pending candidates still have Apply/Reject buttons (no blocking text)
    expect(screen.getByText('Apply')).toBeInTheDocument();
    expect(screen.getByText('Reject')).toBeInTheDocument();
    // The Undo button for the applied candidate should be there
    expect(screen.getByText('Undo')).toBeInTheDocument();
  });
});
