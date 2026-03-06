import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import {
  createBookDetail,
  createAuthorEntry,
  createSeriesEntry,
  createTagEntry
} from '$lib/test-utils/factories.js';
import type { BookDetail } from '$lib/api/types.js';

type CancelFn = () => void;
type SaveFn = (updated: BookDetail) => void;

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
    identify: {
      book: vi.fn(),
      candidates: vi.fn(),
      applyCandidate: vi.fn(),
      rejectCandidate: vi.fn(),
      undoCandidate: vi.fn(),
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

import BookEditForm from './BookEditForm.svelte';

describe('BookEditForm', () => {
  let user: ReturnType<typeof userEvent.setup>;
  let book: BookDetail;

  beforeEach(() => {
    vi.clearAllMocks();
    user = userEvent.setup();
    book = createBookDetail({
      id: 'book-1',
      title: 'Original Title',
      description: 'Original description',
      authors: [
        createAuthorEntry({
          id: 'a1',
          name: 'Author One',
          sort_name: 'One, Author',
          role: 'author',
          position: 0
        }),
        createAuthorEntry({
          id: 'a2',
          name: 'Author Two',
          sort_name: 'Two, Author',
          role: 'author',
          position: 1
        })
      ],
      tags: [createTagEntry({ id: 'tag-1', name: 'fiction', category: null })],
      series: [createSeriesEntry({ id: 'series-1', name: 'Test Series', position: 1 })]
    });
  });

  it('renders form with initial book values', () => {
    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave: vi.fn<SaveFn>() }
    });
    // Title input should have the book's title
    const titleInput = screen.getByLabelText('Title') as HTMLInputElement;
    expect(titleInput.value).toBe('Original Title');

    // Description textarea should have the description
    const descArea = screen.getByLabelText('Description') as HTMLTextAreaElement;
    expect(descArea.value).toBe('Original description');
  });

  it('has Save and Cancel buttons', () => {
    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave: vi.fn<SaveFn>() }
    });
    expect(screen.getByText('Save')).toBeInTheDocument();
    expect(screen.getByText('Cancel')).toBeInTheDocument();
  });

  it('Cancel calls oncancel', async () => {
    const oncancel = vi.fn<CancelFn>();
    render(BookEditForm, {
      props: { book, oncancel, onsave: vi.fn<SaveFn>() }
    });
    await user.click(screen.getByText('Cancel'));
    expect(oncancel).toHaveBeenCalled();
  });

  it('when no changes are made, save calls oncancel (no-op)', async () => {
    const oncancel = vi.fn<CancelFn>();
    const onsave = vi.fn<SaveFn>();
    render(BookEditForm, {
      props: { book, oncancel, onsave }
    });
    await user.click(screen.getByText('Save'));
    // When nothing changed, the component calls oncancel() instead of onsave()
    expect(oncancel).toHaveBeenCalled();
    expect(onsave).not.toHaveBeenCalled();
  });

  it('changing title and saving calls api.books.update with changed fields', async () => {
    const updatedBook = createBookDetail({ title: 'New Title' });
    mockApi.books.update.mockResolvedValue(updatedBook);
    const onsave = vi.fn<SaveFn>();

    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave }
    });

    const titleInput = screen.getByLabelText('Title') as HTMLInputElement;
    await user.clear(titleInput);
    await user.type(titleInput, 'New Title');
    await user.click(screen.getByText('Save'));

    await vi.waitFor(() => {
      expect(mockApi.books.update).toHaveBeenCalledWith('book-1', { title: 'New Title' });
    });
    await vi.waitFor(() => {
      expect(onsave).toHaveBeenCalledWith(updatedBook);
    });
  });

  it('author list renders existing authors', () => {
    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave: vi.fn<SaveFn>() }
    });
    expect(screen.getByText('Author One')).toBeInTheDocument();
    expect(screen.getByText('Author Two')).toBeInTheDocument();
  });

  it('author remove button removes author from the list', async () => {
    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave: vi.fn<SaveFn>() }
    });
    // There should be "Remove author" buttons (one per author)
    const removeButtons = screen.getAllByLabelText('Remove author');
    expect(removeButtons.length).toBe(2);

    // Remove the first author
    await user.click(removeButtons[0]);
    expect(screen.queryByText('Author One')).not.toBeInTheDocument();
    expect(screen.getByText('Author Two')).toBeInTheDocument();
  });

  it('series section renders existing series', () => {
    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave: vi.fn<SaveFn>() }
    });
    expect(screen.getByText('Test Series')).toBeInTheDocument();
  });

  it('tag section renders existing tags', () => {
    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave: vi.fn<SaveFn>() }
    });
    expect(screen.getByText('fiction')).toBeInTheDocument();
  });

  it('error display: when api.books.update throws, shows error message', async () => {
    mockApi.books.update.mockRejectedValue(new Error('Update failed'));
    const onsave = vi.fn<SaveFn>();

    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave }
    });

    const titleInput = screen.getByLabelText('Title') as HTMLInputElement;
    await user.clear(titleInput);
    await user.type(titleInput, 'Changed Title');
    await user.click(screen.getByText('Save'));

    await vi.waitFor(() => {
      expect(screen.getByText('Update failed')).toBeInTheDocument();
    });
  });

  it('save with scalar changes only calls api.books.update, not setAuthors/setTags/setSeries', async () => {
    const updatedBook = createBookDetail({
      title: 'Changed Title',
      description: 'Original description',
      authors: book.authors,
      tags: book.tags,
      series: book.series
    });
    mockApi.books.update.mockResolvedValue(updatedBook);
    const onsave = vi.fn<SaveFn>();

    render(BookEditForm, {
      props: { book, oncancel: vi.fn<CancelFn>(), onsave }
    });

    const titleInput = screen.getByLabelText('Title') as HTMLInputElement;
    await user.clear(titleInput);
    await user.type(titleInput, 'Changed Title');
    await user.click(screen.getByText('Save'));

    await vi.waitFor(() => {
      expect(mockApi.books.update).toHaveBeenCalled();
    });

    // These should NOT have been called since only the title changed
    expect(mockApi.books.setAuthors).not.toHaveBeenCalled();
    expect(mockApi.books.setTags).not.toHaveBeenCalled();
    expect(mockApi.books.setSeries).not.toHaveBeenCalled();
  });
});
