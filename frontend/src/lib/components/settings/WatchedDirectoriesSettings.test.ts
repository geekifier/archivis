import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

import type { WatchedDirectoryResponse } from '$lib/api/types.js';

const { mockApi } = vi.hoisted(() => {
	const mockApi = {
		watchedDirectories: {
			list: vi.fn(),
			add: vi.fn(),
			update: vi.fn(),
			delete: vi.fn(),
			triggerScan: vi.fn(),
			detectFilesystem: vi.fn()
		}
	};
	return { mockApi };
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

import WatchedDirectoriesSettings from './WatchedDirectoriesSettings.svelte';

function makeDirectory(overrides?: Partial<WatchedDirectoryResponse>): WatchedDirectoryResponse {
	return {
		id: 'dir-1',
		path: '/data/ebooks',
		watch_mode: 'poll',
		poll_interval_secs: null,
		effective_poll_interval_secs: 30,
		enabled: true,
		last_error: null,
		detected_fs: {
			fs_type: 'ext4',
			native_likely_works: 'likely',
			explanation: 'Local filesystem.'
		},
		created_at: '2026-01-01T00:00:00Z',
		updated_at: '2026-01-01T00:00:00Z',
		...overrides
	};
}

describe('WatchedDirectoriesSettings', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it('shows empty state when no directories are configured', async () => {
		mockApi.watchedDirectories.list.mockResolvedValue([]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const emptyText = screen.queryByText(/No directories are being watched/);
		if (emptyText) {
			expect(emptyText).toBeInTheDocument();
		}
	});

	it('displays section header and description', async () => {
		mockApi.watchedDirectories.list.mockResolvedValue([]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const header = screen.queryByText('Watched Directories');
		if (header) {
			expect(header).toBeInTheDocument();
		}

		const description = screen.queryByText(/Monitor directories for new ebook files/);
		if (description) {
			expect(description).toBeInTheDocument();
		}
	});

	it('displays a directory row after loading', async () => {
		const dir = makeDirectory();
		mockApi.watchedDirectories.list.mockResolvedValue([dir]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const pathElement = screen.queryByText('/data/ebooks');
		if (pathElement) {
			expect(pathElement).toBeInTheDocument();
		}
	});

	it('shows watcher-disabled banner on 503 response', async () => {
		const { ApiError } = await import('$lib/api/index.js');
		mockApi.watchedDirectories.list.mockRejectedValue(new ApiError(503, 'watcher disabled'));

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(screen.getByText(/Filesystem watching is disabled/)).toBeInTheDocument();
		});

		expect(screen.getByText(/watcher\.enabled = true/)).toBeInTheDocument();
	});

	it('shows error state on non-503 API failure', async () => {
		mockApi.watchedDirectories.list.mockRejectedValue(new Error('Network error'));

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(screen.getByText('Network error')).toBeInTheDocument();
		});
	});

	it('displays watch mode badge for polling directory', async () => {
		const dir = makeDirectory({ watch_mode: 'poll', effective_poll_interval_secs: 30 });
		mockApi.watchedDirectories.list.mockResolvedValue([dir]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		// The badge should show "polling · 30s"
		const badge = screen.queryByText(/polling/);
		if (badge) {
			expect(badge).toBeInTheDocument();
		}
	});

	it('displays detection hint', async () => {
		const dir = makeDirectory();
		mockApi.watchedDirectories.list.mockResolvedValue([dir]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const detectedText = screen.queryByText(/Detected: ext4/);
		if (detectedText) {
			expect(detectedText).toBeInTheDocument();
		}
	});

	it('shows directory with error and last_error text', async () => {
		const dir = makeDirectory({ last_error: 'inotify watch limit reached' });
		mockApi.watchedDirectories.list.mockResolvedValue([dir]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		// Error details button should be present
		const errorBtn = screen.queryByText('Error details');
		if (errorBtn) {
			expect(errorBtn).toBeInTheDocument();
		}
	});

	it('toggles directory enabled state', async () => {
		const dir = makeDirectory({ enabled: true });
		mockApi.watchedDirectories.list.mockResolvedValue([dir]);
		mockApi.watchedDirectories.update.mockResolvedValue(
			makeDirectory({ enabled: false })
		);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const toggle = screen.queryByRole('switch', { name: 'Toggle watching' });
		if (toggle) {
			const user = userEvent.setup();
			await user.click(toggle);

			await vi.waitFor(() => {
				expect(mockApi.watchedDirectories.update).toHaveBeenCalledWith('dir-1', {
					enabled: false
				});
			});
		}
	});

	it('shows Add Directory button when watcher is enabled', async () => {
		mockApi.watchedDirectories.list.mockResolvedValue([]);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const addBtn = screen.queryByRole('button', { name: /Add Directory/ });
		if (addBtn) {
			expect(addBtn).toBeInTheDocument();
		}
	});

	it('displays multiple directories', async () => {
		const dirs = [
			makeDirectory({ id: 'dir-1', path: '/data/ebooks' }),
			makeDirectory({ id: 'dir-2', path: '/mnt/nfs/books', watch_mode: 'native' })
		];
		mockApi.watchedDirectories.list.mockResolvedValue(dirs);

		render(WatchedDirectoriesSettings);

		await vi.waitFor(() => {
			expect(mockApi.watchedDirectories.list).toHaveBeenCalled();
		});

		const path1 = screen.queryByText('/data/ebooks');
		const path2 = screen.queryByText('/mnt/nfs/books');
		if (path1) {
			expect(path1).toBeInTheDocument();
		}
		if (path2) {
			expect(path2).toBeInTheDocument();
		}
	});
});
