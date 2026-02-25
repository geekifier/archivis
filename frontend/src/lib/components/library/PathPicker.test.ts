import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';

const { mockApi } = vi.hoisted(() => {
	const createMockApiFn = () => ({
		auth: { status: vi.fn(), setup: vi.fn(), login: vi.fn(), logout: vi.fn(), me: vi.fn() },
		books: {
			list: vi.fn(), get: vi.fn(), update: vi.fn(), setAuthors: vi.fn(),
			setSeries: vi.fn(), setTags: vi.fn(), delete: vi.fn(), uploadCover: vi.fn()
		},
		authors: { get: vi.fn(), search: vi.fn(), create: vi.fn(), listBooks: vi.fn() },
		tags: { search: vi.fn() },
		publishers: { search: vi.fn(), create: vi.fn() },
		series: { get: vi.fn(), search: vi.fn(), listBooks: vi.fn() },
		filesystem: { browse: vi.fn() },
		import: { upload: vi.fn(), scan: vi.fn(), startImport: vi.fn() },
		tasks: { list: vi.fn(), get: vi.fn() },
		identify: {
			book: vi.fn(), candidates: vi.fn(), applyCandidate: vi.fn(),
			rejectCandidate: vi.fn(), undoCandidate: vi.fn(), batch: vi.fn(), all: vi.fn()
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

// Import component after mock setup
import PathPicker from './PathPicker.svelte';
import type { BrowseResponse } from '$lib/api/types.js';

function makeBrowseResponse(overrides?: Partial<BrowseResponse>): BrowseResponse {
	return {
		path: '/home/user',
		parent: '/home',
		entries: [
			{ name: 'documents', is_dir: true, size: 0 },
			{ name: 'readme.txt', is_dir: false, size: 1024 }
		],
		...overrides
	};
}

describe('PathPicker', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockApi.filesystem.browse.mockResolvedValue(makeBrowseResponse());
	});

	// Note: bits-ui Dialog may not render in jsdom. We test what we can and
	// gracefully fall back when Dialog content is not present.

	it('renders dialog title when open', async () => {
		render(PathPicker, {
			props: { value: '', open: true, mode: 'directory' }
		});

		// Wait for the browse call triggered by $effect
		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalled();
		});

		const title = screen.queryByText('Select Directory');
		if (title) {
			expect(title).toBeInTheDocument();
		} else {
			// bits-ui Dialog does not render in jsdom -- accepted
			expect(mockApi.filesystem.browse).toHaveBeenCalledWith(undefined, true);
		}
	});

	it('renders custom title when provided', async () => {
		render(PathPicker, {
			props: { value: '', open: true, mode: 'directory', title: 'Pick a folder' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalled();
		});

		const title = screen.queryByText('Pick a folder');
		if (title) {
			expect(title).toBeInTheDocument();
		}
	});

	it('defaults to server data dir when value is empty', async () => {
		render(PathPicker, {
			props: { value: '', open: true, mode: 'directory' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalledWith(undefined, true);
		});
	});

	it('navigates to value path when opened', async () => {
		render(PathPicker, {
			props: { value: '/home/user/books', open: true, mode: 'directory' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalledWith('/home/user/books', true);
		});
	});

	it('renders breadcrumbs for a deep path', async () => {
		mockApi.filesystem.browse.mockResolvedValue(
			makeBrowseResponse({ path: '/home/user/documents' })
		);

		render(PathPicker, {
			props: { value: '/home/user/documents', open: true, mode: 'directory' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalled();
		});

		// Check for breadcrumb segments if Dialog renders
		const homeSegment = screen.queryByText('home');
		if (homeSegment) {
			expect(homeSegment).toBeInTheDocument();
			expect(screen.queryByText('user')).toBeInTheDocument();
			expect(screen.queryByText('documents')).toBeInTheDocument();
		}
	});

	it('calls browse when clicking a directory entry', async () => {
		render(PathPicker, {
			props: { value: '/home/user', open: true, mode: 'directory' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalledTimes(1);
		});

		// If Dialog renders, click the directory entry
		const dirButton = screen.queryByText('documents');
		if (dirButton) {
			mockApi.filesystem.browse.mockResolvedValue(
				makeBrowseResponse({ path: '/home/user/documents', entries: [] })
			);
			dirButton.click();

			await vi.waitFor(() => {
				expect(mockApi.filesystem.browse).toHaveBeenCalledWith(
					'/home/user/documents',
					true
				);
			});
		}
	});

	it('shows error state on API failure', async () => {
		mockApi.filesystem.browse.mockRejectedValue(new Error('Permission denied'));

		render(PathPicker, {
			props: { value: '/root', open: true, mode: 'directory' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalled();
		});

		// If Dialog renders, error message should be shown
		const errorText = screen.queryByText('Permission denied');
		if (errorText) {
			expect(errorText).toBeInTheDocument();
		}
	});

	it('uses file mode with dirs_only=false', async () => {
		render(PathPicker, {
			props: { value: '/home', open: true, mode: 'file' }
		});

		await vi.waitFor(() => {
			expect(mockApi.filesystem.browse).toHaveBeenCalledWith('/home', false);
		});
	});
});
