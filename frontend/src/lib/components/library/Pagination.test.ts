import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import Pagination from './Pagination.svelte';

describe('Pagination', () => {
	let onPageChange: ReturnType<typeof vi.fn<(page: number) => void>>;
	let user: ReturnType<typeof userEvent.setup>;

	beforeEach(() => {
		onPageChange = vi.fn<(page: number) => void>();
		user = userEvent.setup();
	});

	it('does not render when totalPages <= 1', () => {
		const { container } = render(Pagination, {
			props: { page: 1, totalPages: 1, onPageChange }
		});
		expect(container.querySelector('nav')).toBeNull();
	});

	it('does not render when totalPages is 0', () => {
		const { container } = render(Pagination, {
			props: { page: 1, totalPages: 0, onPageChange }
		});
		expect(container.querySelector('nav')).toBeNull();
	});

	it('renders Previous and Next buttons', () => {
		render(Pagination, {
			props: { page: 2, totalPages: 5, onPageChange }
		});
		expect(screen.getByText('Previous')).toBeInTheDocument();
		expect(screen.getByText('Next')).toBeInTheDocument();
	});

	it('disables Previous button when page=1', () => {
		render(Pagination, {
			props: { page: 1, totalPages: 5, onPageChange }
		});
		const prev = screen.getByText('Previous');
		expect(prev.closest('button')).toBeDisabled();
	});

	it('disables Next button when page=totalPages', () => {
		render(Pagination, {
			props: { page: 5, totalPages: 5, onPageChange }
		});
		const next = screen.getByText('Next');
		expect(next.closest('button')).toBeDisabled();
	});

	it('clicking Previous calls onPageChange with page-1', async () => {
		render(Pagination, {
			props: { page: 3, totalPages: 5, onPageChange }
		});
		await user.click(screen.getByText('Previous'));
		expect(onPageChange).toHaveBeenCalledWith(2);
	});

	it('clicking Next calls onPageChange with page+1', async () => {
		render(Pagination, {
			props: { page: 3, totalPages: 5, onPageChange }
		});
		await user.click(screen.getByText('Next'));
		expect(onPageChange).toHaveBeenCalledWith(4);
	});

	it('renders page number buttons', () => {
		render(Pagination, {
			props: { page: 1, totalPages: 3, onPageChange }
		});
		expect(screen.getByText('1')).toBeInTheDocument();
		expect(screen.getByText('2')).toBeInTheDocument();
		expect(screen.getByText('3')).toBeInTheDocument();
	});

	it('active page has aria-current="page"', () => {
		render(Pagination, {
			props: { page: 2, totalPages: 3, onPageChange }
		});
		const activeButton = screen.getByText('2').closest('button');
		expect(activeButton).toHaveAttribute('aria-current', 'page');
	});

	it('non-active pages do not have aria-current', () => {
		render(Pagination, {
			props: { page: 2, totalPages: 3, onPageChange }
		});
		const otherButton = screen.getByText('1').closest('button');
		expect(otherButton).not.toHaveAttribute('aria-current');
	});

	it('clicking a page number calls onPageChange with that number', async () => {
		render(Pagination, {
			props: { page: 1, totalPages: 5, onPageChange }
		});
		await user.click(screen.getByText('3'));
		expect(onPageChange).toHaveBeenCalledWith(3);
	});

	it('shows ellipsis for many pages', () => {
		render(Pagination, {
			props: { page: 5, totalPages: 10, onPageChange }
		});
		const ellipses = screen.getAllByText('...');
		expect(ellipses.length).toBeGreaterThanOrEqual(1);
	});
});
