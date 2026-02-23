import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import BookListView from './BookListView.svelte';
import {
	createBookSummary,
	createFileEntry,
	createAuthorEntry,
	createSeriesEntry
} from '$lib/test-utils/index.js';
import type { SortField, SortOrder } from '$lib/api/index.js';

describe('BookListView', () => {
	let onSort: ReturnType<typeof vi.fn<(field: SortField, order: SortOrder) => void>>;
	let user: ReturnType<typeof userEvent.setup>;

	const defaultBooks = [
		createBookSummary({
			id: 'book-1',
			title: 'First Book',
			metadata_status: 'identified',
			authors: [createAuthorEntry({ name: 'Author One' })],
			files: [createFileEntry({ format: 'epub' })],
			series: [createSeriesEntry({ name: 'Series A', position: 1 })]
		}),
		createBookSummary({
			id: 'book-2',
			title: 'Second Book',
			metadata_status: 'needs_review',
			authors: [
				createAuthorEntry({ name: 'Author Two' }),
				createAuthorEntry({ id: 'author-3', name: 'Author Three', position: 1 })
			],
			files: [createFileEntry({ format: 'pdf' })],
			series: []
		})
	];

	beforeEach(() => {
		onSort = vi.fn<(field: SortField, order: SortOrder) => void>();
		user = userEvent.setup();
	});

	it('renders table with correct headers', () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		expect(screen.getByText('Title')).toBeInTheDocument();
		expect(screen.getByText('Author')).toBeInTheDocument();
		expect(screen.getByText('Series')).toBeInTheDocument();
		expect(screen.getByText('Format')).toBeInTheDocument();
		expect(screen.getByText('Date Added')).toBeInTheDocument();
		expect(screen.getByText('Status')).toBeInTheDocument();
	});

	it('renders rows for each book', () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		// Each book should have a title link
		expect(screen.getByText('First Book')).toBeInTheDocument();
		expect(screen.getByText('Second Book')).toBeInTheDocument();
	});

	it('shows book titles as links', () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		const firstLink = screen.getByText('First Book').closest('a');
		expect(firstLink).toHaveAttribute('href', '/books/book-1');
		const secondLink = screen.getByText('Second Book').closest('a');
		expect(secondLink).toHaveAttribute('href', '/books/book-2');
	});

	it('shows format badges (uppercase)', () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		expect(screen.getByText('EPUB')).toBeInTheDocument();
		expect(screen.getByText('PDF')).toBeInTheDocument();
	});

	it('shows status badges with correct labels', () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		expect(screen.getByText('Identified')).toBeInTheDocument();
		expect(screen.getByText('Needs Review')).toBeInTheDocument();
	});

	it('clicking sortable header (Title) calls onSort with field and order', async () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		// Title is a sortable header — clicking it when not currently sorted defaults to 'asc'
		const titleHeader = screen.getByText('Title').closest('th')!;
		await user.click(titleHeader);
		expect(onSort).toHaveBeenCalledWith('title', 'asc');
	});

	it('clicking the same sorted header toggles order (asc -> desc)', async () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'title', sortOrder: 'asc', onSort }
		});
		const titleHeader = screen.getByText('Title').closest('th')!;
		await user.click(titleHeader);
		expect(onSort).toHaveBeenCalledWith('title', 'desc');
	});

	it('clicking a different sortable header defaults to asc', async () => {
		render(BookListView, {
			props: { books: defaultBooks, sortBy: 'title', sortOrder: 'desc', onSort }
		});
		const dateHeader = screen.getByText('Date Added').closest('th')!;
		await user.click(dateHeader);
		expect(onSort).toHaveBeenCalledWith('added_at', 'asc');
	});

	it('renders with empty books array', () => {
		render(BookListView, {
			props: { books: [], sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		// Table headers should still render
		expect(screen.getByText('Title')).toBeInTheDocument();
	});

	it('shows unidentified status badge', () => {
		const books = [
			createBookSummary({ metadata_status: 'unidentified', title: 'Unknown Book' })
		];
		render(BookListView, {
			props: { books, sortBy: 'added_at', sortOrder: 'desc', onSort }
		});
		expect(screen.getByText('Unidentified')).toBeInTheDocument();
	});
});
