import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

import type { SettingEntry } from '$lib/api/types.js';

const { mockApi, mockAuth } = vi.hoisted(() => {
	const mockApi = {
		settings: {
			get: vi.fn(),
			update: vi.fn()
		},
		watchedDirectories: {
			list: vi.fn(),
			add: vi.fn(),
			update: vi.fn(),
			delete: vi.fn(),
			triggerScan: vi.fn(),
			detectFilesystem: vi.fn()
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

function makeBoolSetting(overrides: Partial<SettingEntry> = {}): SettingEntry {
	return {
		key: 'metadata.hardcover.enabled',
		value: false,
		effective_value: false,
		source: 'default',
		scope: 'runtime',
		override: null,
		requires_restart: false,
		label: 'Hardcover Enabled',
		description: 'Whether Hardcover lookups are enabled',
		section: 'metadata.hardcover',
		value_type: 'bool',
		...overrides
	};
}

describe('Settings revert and reset behavior', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockApi.watchedDirectories.list.mockResolvedValue([]);
		mockApi.settings.update.mockResolvedValue({
			updated: ['metadata.hardcover.enabled'],
			requires_restart: false
		});
	});

	it('toggling a default setting shows unsaved badge and undo button', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting()]
		});

		render(SettingsPage);

		// Toggle the switch on
		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		await user.click(toggle);

		// Should show "unsaved" badge
		expect(screen.getByText('unsaved')).toBeInTheDocument();
		// Should show "Undo change" button (not "Reset to default")
		expect(screen.getByRole('button', { name: 'Undo change' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Reset to default' })).not.toBeInTheDocument();
	});

	it('clicking undo reverts the toggle and removes badge', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting()]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		await user.click(toggle);

		expect(screen.getByText('unsaved')).toBeInTheDocument();

		// Click undo
		await user.click(screen.getByRole('button', { name: 'Undo change' }));

		// Badge should be gone, no pending changes
		expect(screen.queryByText('unsaved')).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Undo change' })).not.toBeInTheDocument();
		// Save button should be disabled (no changes)
		expect(screen.getByRole('button', { name: 'Save Changes' })).toBeDisabled();
	});

	it('reset to default on database setting shows unsaved badge', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting({ source: 'database', value: true, effective_value: true })]
		});

		render(SettingsPage);

		// Wait for settings to load, then check "modified" badge and "Reset to default" button
		const resetBtn = await screen.findByRole('button', { name: 'Reset to default' });
		expect(screen.getByText('modified')).toBeInTheDocument();

		await user.click(resetBtn);

		// Should now show "unsaved" badge and "Undo change" button
		expect(screen.queryByText('modified')).not.toBeInTheDocument();
		expect(screen.getByText('unsaved')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Undo change' })).toBeInTheDocument();
	});

	it('undo after reset to default restores modified badge', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting({ source: 'database', value: true, effective_value: true })]
		});

		render(SettingsPage);

		// Click reset
		const resetBtn = await screen.findByRole('button', { name: 'Reset to default' });
		await user.click(resetBtn);
		expect(screen.getByText('unsaved')).toBeInTheDocument();

		// Click undo
		await user.click(screen.getByRole('button', { name: 'Undo change' }));

		// Should restore "modified" badge
		expect(screen.queryByText('unsaved')).not.toBeInTheDocument();
		expect(screen.getByText('modified')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Reset to default' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Save Changes' })).toBeDisabled();
	});

	it('pending edit on database setting shows undo button not reset button', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting({ source: 'database', value: true, effective_value: true })]
		});

		render(SettingsPage);

		// Toggle the switch (creates a pending edit)
		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		await user.click(toggle);

		// Should show "Undo change", not "Reset to default"
		expect(screen.getByRole('button', { name: 'Undo change' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Reset to default' })).not.toBeInTheDocument();
	});
});

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
		mockApi.watchedDirectories.list.mockResolvedValue([]);
	});

	it('uses a textarea for token replacement and saves the entered value', async () => {
		const user = userEvent.setup();

		render(SettingsPage);

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
