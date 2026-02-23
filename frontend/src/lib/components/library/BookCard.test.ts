import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import BookCard from './BookCard.svelte';
import { createBookSummary, createFileEntry, createAuthorEntry } from '$lib/test-utils/index.js';

describe('BookCard', () => {
	it('renders book title', () => {
		const book = createBookSummary({ title: 'The Great Gatsby' });
		render(BookCard, { props: { book } });
		// Title appears in both the placeholder area and the text below the cover
		const titles = screen.getAllByText('The Great Gatsby');
		expect(titles.length).toBeGreaterThanOrEqual(1);
	});

	it('renders author names', () => {
		const book = createBookSummary({
			authors: [
				createAuthorEntry({ name: 'F. Scott Fitzgerald' }),
				createAuthorEntry({ id: 'author-2', name: 'Jane Austen', position: 1 })
			]
		});
		render(BookCard, { props: { book } });
		expect(screen.getByText('F. Scott Fitzgerald, Jane Austen')).toBeInTheDocument();
	});

	it('links to /books/{book.id}', () => {
		const book = createBookSummary({ id: 'abc-123' });
		render(BookCard, { props: { book } });
		const link = screen.getByRole('link');
		expect(link).toHaveAttribute('href', '/books/abc-123');
	});

	it('shows format badge (uppercase) from first file', () => {
		const book = createBookSummary({
			files: [createFileEntry({ format: 'epub' })]
		});
		render(BookCard, { props: { book } });
		expect(screen.getByText('EPUB')).toBeInTheDocument();
	});

	it('does not show format badge when no files', () => {
		const book = createBookSummary({ files: [] });
		render(BookCard, { props: { book } });
		expect(screen.queryByText('EPUB')).toBeNull();
	});

	it('shows status indicator for needs_review (amber dot)', () => {
		const book = createBookSummary({ metadata_status: 'needs_review' });
		render(BookCard, { props: { book } });
		const indicator = screen.getByTitle('Needs review');
		expect(indicator).toBeInTheDocument();
		expect(indicator.className).toContain('bg-amber-500');
	});

	it('shows status indicator for unidentified (red dot)', () => {
		const book = createBookSummary({ metadata_status: 'unidentified' });
		render(BookCard, { props: { book } });
		const indicator = screen.getByTitle('Unidentified');
		expect(indicator).toBeInTheDocument();
		expect(indicator.className).toContain('bg-red-500');
	});

	it('no status indicator for identified', () => {
		const book = createBookSummary({ metadata_status: 'identified' });
		render(BookCard, { props: { book } });
		expect(screen.queryByTitle('Needs review')).toBeNull();
		expect(screen.queryByTitle('Unidentified')).toBeNull();
	});

	it('shows placeholder when no cover (has_cover: false)', () => {
		const book = createBookSummary({ has_cover: false, title: 'No Cover Book' });
		render(BookCard, { props: { book } });
		// The placeholder shows the title inside the cover area
		// There are two instances: one in the cover placeholder, one below
		const titleElements = screen.getAllByText('No Cover Book');
		expect(titleElements.length).toBeGreaterThanOrEqual(2);
	});

	it('shows cover image when has_cover is true', () => {
		const book = createBookSummary({ id: 'book-42', has_cover: true, title: 'With Cover' });
		render(BookCard, { props: { book } });
		const img = screen.getByAltText('Cover of With Cover');
		expect(img).toBeInTheDocument();
		expect(img).toHaveAttribute('src', '/api/books/book-42/cover?size=sm');
	});
});
