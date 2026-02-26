import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

import type { SettingEntry } from '$lib/api/types.js';

const { mockApi, mockAuth } = vi.hoisted(() => {
	const mockApi = {
		settings: {
			get: vi.fn(),
			update: vi.fn()
		}
	};

	const mockAuth = {
		loading: false,
		user: {
			id: 'admin-id',
			username: 'admin',
			email: null,
			role: 'admin' as const,
			is_active: true,
			created_at: '2026-01-01T00:00:00Z'
		},
		isAuthenticated: true
	};

	return { mockApi, mockAuth };
});

vi.mock('$lib/api/index.js', () => ({
	api: mockApi
}));

vi.mock('$lib/stores/auth.svelte.js', () => ({
	auth: mockAuth
}));

import SettingsPage from './+page.svelte';

function makeHardcoverTokenSetting(overrides: Partial<SettingEntry> = {}): SettingEntry {
	return {
		key: 'metadata.hardcover.api_token',
		value: '***',
		effective_value: '***',
		source: 'database',
		scope: 'runtime',
		override: null,
		requires_restart: false,
		label: 'API Token',
		description: 'Bearer token for the Hardcover GraphQL API',
		section: 'metadata.hardcover',
		value_type: 'optional_string',
		sensitive: true,
		is_set: true,
		...overrides
	};
}

describe('Settings page sensitive controls', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeHardcoverTokenSetting()]
		});
		mockApi.settings.update.mockResolvedValue({
			updated: ['metadata.hardcover.api_token'],
			requires_restart: false
		});
	});

	it('uses a textarea for token replacement and saves the entered value', async () => {
		const user = userEvent.setup();

		render(SettingsPage);

		await vi.waitFor(() => {
			expect(mockApi.settings.get).toHaveBeenCalledTimes(1);
		});

		const tokenField = (await screen.findByLabelText('API Token')) as HTMLTextAreaElement;
		expect(tokenField.tagName).toBe('TEXTAREA');
		expect(tokenField.value).toBe('');
		expect(tokenField.placeholder).toContain('Token is configured');
		expect(
			screen.getByText('The current token cannot be revealed. Paste a new token to replace it.')
		).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Show value' })).not.toBeInTheDocument();

		const replacementToken =
			'hc_pat_9f7d7d1a3ec04a3d8fa1e8f2878b31a5f26059df8ed6405f95fc9de3d35fd58';
		await user.type(tokenField, replacementToken);

		const saveButton = screen.getByRole('button', { name: 'Save Changes' });
		await vi.waitFor(() => {
			expect(saveButton).toBeEnabled();
		});

		await user.click(saveButton);
		await vi.waitFor(() => {
			expect(mockApi.settings.update).toHaveBeenCalledWith({
				'metadata.hardcover.api_token': replacementToken
			});
		});
	});

	it('resets token to default on save', async () => {
		const user = userEvent.setup();

		render(SettingsPage);

		await vi.waitFor(() => {
			expect(mockApi.settings.get).toHaveBeenCalledTimes(1);
		});

		await screen.findByLabelText('API Token');
		await user.click(screen.getByRole('button', { name: 'Reset to default' }));

		const tokenField = (await screen.findByLabelText('API Token')) as HTMLTextAreaElement;
		expect(tokenField.value).toBe('');
		expect(tokenField.placeholder).toBe('Will be cleared on save');

		const saveButton = screen.getByRole('button', { name: 'Save Changes' });
		await vi.waitFor(() => {
			expect(saveButton).toBeEnabled();
		});

		await user.click(saveButton);
		await vi.waitFor(() => {
			expect(mockApi.settings.update).toHaveBeenCalledWith({
				'metadata.hardcover.api_token': null
			});
		});
	});
});
