import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

const { mockApi } = vi.hoisted(() => {
  const mockApi = {
    stats: {
      get: vi.fn()
    }
  };
  return { mockApi };
});

vi.mock('$lib/api/index.js', () => ({
  api: mockApi
}));

import StatsPage from './+page.svelte';

function makeStatsResponse(overrides?: Record<string, unknown>) {
  return {
    generated_at: '2026-02-26T00:00:00Z',
    library: {
      books: 42,
      files: 84,
      total_file_size: 123456789,
      average_files_per_book: 2,
      files_by_format: [{ format: 'epub', file_count: 60, total_size: 100000000 }],
      metadata_status: [{ status: 'identified', count: 30 }]
    },
    usage: {
      tasks_total: 12,
      tasks_last_24h: 3,
      tasks_by_status: [{ status: 'completed', count: 10 }],
      tasks_by_type: [{ task_type: 'import_file', count: 8 }],
      pending_duplicates: 1,
      pending_candidates: 2
    },
    db: null,
    ...overrides
  };
}

describe('Statistics page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockApi.stats.get.mockResolvedValue(makeStatsResponse());
  });

  it('renders library and usage statistics', async () => {
    render(StatsPage);

    await vi.waitFor(() => {
      expect(mockApi.stats.get).toHaveBeenCalledTimes(1);
    });

    expect(await screen.findByText('Statistics')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();
    expect(screen.getByText('84')).toBeInTheDocument();
    expect(screen.getByText('Task and Queue Usage')).toBeInTheDocument();
  });

  it('shows admin DB diagnostics when db stats are present', async () => {
    mockApi.stats.get.mockResolvedValue(
      makeStatsResponse({
        db: {
          files: { main_db_size: 1024, wal_size: 512, shm_size: 0 },
          pages: {
            page_size: 4096,
            page_count: 100,
            freelist_count: 10,
            used_pages: 90,
            used_bytes: 368640,
            free_bytes: 40960
          },
          table_size_estimates_available: true,
          objects: [
            {
              name: 'books',
              object_type: 'table',
              estimated_bytes: 40960,
              row_count: 42
            }
          ]
        }
      })
    );

    render(StatsPage);

    expect(await screen.findByText('Database Diagnostics (Admin)')).toBeInTheDocument();
    expect(screen.getByText('books')).toBeInTheDocument();
  });

  it('shows an error and can retry loading', async () => {
    const user = userEvent.setup();
    mockApi.stats.get
      .mockRejectedValueOnce(new Error('Failed to load statistics'))
      .mockResolvedValueOnce(makeStatsResponse());

    render(StatsPage);

    expect(await screen.findByText('Failed to load statistics')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Retry' }));

    await vi.waitFor(() => {
      expect(mockApi.stats.get).toHaveBeenCalledTimes(2);
    });
    expect(await screen.findByText('Task and Queue Usage')).toBeInTheDocument();
  });
});
