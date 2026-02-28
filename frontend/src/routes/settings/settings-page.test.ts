import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/svelte';
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

/** A fake JWT that passes the three-segment base64url validation. */
const FAKE_JWT = 'eyJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJIYXJkY292ZXIifQ.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';

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

/** Return the nearest setting row (`div.px-6.py-4`) for a given label. */
function getSettingRow(labelText: string): HTMLElement {
	const label = screen.getByText(labelText, { selector: 'label' });
	return label.closest('div.px-6.py-4') as HTMLElement;
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
			settings: [makeBoolSetting(), makeHardcoverTokenSetting()]
		});

		render(SettingsPage);

		// Toggle the switch on (allowed because token is set)
		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		await user.click(toggle);

		const row = getSettingRow('Hardcover Enabled');

		// Should show "unsaved" badge
		expect(within(row).getByText('unsaved')).toBeInTheDocument();
		// Should show "Undo change" button, not "Reset to default" in this row
		expect(within(row).getByRole('button', { name: 'Undo change' })).toBeInTheDocument();
		expect(within(row).queryByRole('button', { name: 'Reset to default' })).not.toBeInTheDocument();
	});

	it('clicking undo reverts the toggle and removes badge', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting(), makeHardcoverTokenSetting()]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		await user.click(toggle);

		const row = getSettingRow('Hardcover Enabled');
		expect(within(row).getByText('unsaved')).toBeInTheDocument();

		// Click undo
		await user.click(within(row).getByRole('button', { name: 'Undo change' }));

		// Badge should be gone, no pending changes
		expect(within(row).queryByText('unsaved')).not.toBeInTheDocument();
		expect(within(row).queryByRole('button', { name: 'Undo change' })).not.toBeInTheDocument();
		// Save button should be disabled (no changes)
		expect(screen.getByRole('button', { name: 'Save Changes' })).toBeDisabled();
	});

	it('reset to default on database setting shows unsaved badge', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting({ source: 'database', value: true, effective_value: true }),
				makeHardcoverTokenSetting()
			]
		});

		render(SettingsPage);

		// Wait for settings to load, then find reset in the toggle row
		await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		const row = getSettingRow('Hardcover Enabled');
		const resetBtn = within(row).getByRole('button', { name: 'Reset to default' });
		expect(within(row).getByText('modified')).toBeInTheDocument();

		await user.click(resetBtn);

		// Should now show "unsaved" badge and "Undo change" button
		expect(within(row).queryByText('modified')).not.toBeInTheDocument();
		expect(within(row).getByText('unsaved')).toBeInTheDocument();
		expect(within(row).getByRole('button', { name: 'Undo change' })).toBeInTheDocument();
	});

	it('undo after reset to default restores modified badge', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting({ source: 'database', value: true, effective_value: true }),
				makeHardcoverTokenSetting()
			]
		});

		render(SettingsPage);

		await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		const row = getSettingRow('Hardcover Enabled');

		// Click reset
		await user.click(within(row).getByRole('button', { name: 'Reset to default' }));
		expect(within(row).getByText('unsaved')).toBeInTheDocument();

		// Click undo
		await user.click(within(row).getByRole('button', { name: 'Undo change' }));

		// Should restore "modified" badge
		expect(within(row).queryByText('unsaved')).not.toBeInTheDocument();
		expect(within(row).getByText('modified')).toBeInTheDocument();
		expect(within(row).getByRole('button', { name: 'Reset to default' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Save Changes' })).toBeDisabled();
	});

	it('pending edit on database setting shows undo button not reset button', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting({ source: 'database', value: true, effective_value: true }),
				makeHardcoverTokenSetting()
			]
		});

		render(SettingsPage);

		// Toggle the switch (creates a pending edit)
		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		await user.click(toggle);

		const row = getSettingRow('Hardcover Enabled');
		// Should show "Undo change", not "Reset to default"
		expect(within(row).getByRole('button', { name: 'Undo change' })).toBeInTheDocument();
		expect(within(row).queryByRole('button', { name: 'Reset to default' })).not.toBeInTheDocument();
	});
});

describe('Hardcover toggle requires valid API token', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockApi.watchedDirectories.list.mockResolvedValue([]);
		mockApi.settings.update.mockResolvedValue({
			updated: ['metadata.hardcover.enabled'],
			requires_restart: false
		});
	});

	it('toggle is disabled when no token is configured', async () => {
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting(),
				makeHardcoverTokenSetting({ is_set: false, value: null, effective_value: null, source: 'default' })
			]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		expect(toggle).toBeDisabled();
		expect(toggle).toHaveAttribute('title', 'Paste a valid Hardcover API token first');
	});

	it('toggle is enabled when a token is already saved', async () => {
		mockApi.settings.get.mockResolvedValue({
			settings: [makeBoolSetting(), makeHardcoverTokenSetting()]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		expect(toggle).toBeEnabled();
	});

	it('toggle becomes enabled after pasting a valid JWT token', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting(),
				makeHardcoverTokenSetting({ is_set: false, value: null, effective_value: null, source: 'default' })
			]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		expect(toggle).toBeDisabled();

		// Paste a valid JWT token
		const tokenField = screen.getByLabelText('API Token') as HTMLTextAreaElement;
		await user.click(tokenField);
		await user.paste(FAKE_JWT);

		await vi.waitFor(() => {
			expect(toggle).toBeEnabled();
		});
	});

	it('toggle becomes enabled after pasting a Bearer-prefixed JWT', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting(),
				makeHardcoverTokenSetting({ is_set: false, value: null, effective_value: null, source: 'default' })
			]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		expect(toggle).toBeDisabled();

		// Paste token with "Bearer " prefix (as copied from Hardcover)
		const tokenField = screen.getByLabelText('API Token') as HTMLTextAreaElement;
		await user.click(tokenField);
		await user.paste(`Bearer ${FAKE_JWT}`);

		await vi.waitFor(() => {
			expect(toggle).toBeEnabled();
		});
	});

	it('toggle stays disabled when token has invalid format', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting(),
				makeHardcoverTokenSetting({ is_set: false, value: null, effective_value: null, source: 'default' })
			]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		expect(toggle).toBeDisabled();

		// Type an invalid token (not a JWT)
		const tokenField = screen.getByLabelText('API Token') as HTMLTextAreaElement;
		await user.click(tokenField);
		await user.paste('not-a-valid-token');

		expect(toggle).toBeDisabled();
	});

	it('clearing the token auto-disables the enabled toggle', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [
				makeBoolSetting({ source: 'database', value: true, effective_value: true }),
				makeHardcoverTokenSetting()
			]
		});

		render(SettingsPage);

		const toggle = await screen.findByRole('switch', { name: 'Hardcover Enabled' });
		expect(toggle).toHaveAttribute('aria-checked', 'true');

		// Reset token to default (clears it)
		const tokenRow = getSettingRow('API Token');
		await user.click(within(tokenRow).getByRole('button', { name: 'Reset to default' }));

		// The enabled toggle should have been auto-reverted to off
		await vi.waitFor(() => {
			expect(toggle).toHaveAttribute('aria-checked', 'false');
		});
	});

	it('strips Bearer prefix when saving token', async () => {
		const user = userEvent.setup();
		mockApi.settings.get.mockResolvedValue({
			settings: [makeHardcoverTokenSetting()]
		});
		mockApi.settings.update.mockResolvedValue({
			updated: ['metadata.hardcover.api_token'],
			requires_restart: false
		});

		render(SettingsPage);

		const tokenField = (await screen.findByLabelText('API Token')) as HTMLTextAreaElement;
		await user.click(tokenField);
		await user.paste(`Bearer ${FAKE_JWT}`);

		const saveButton = screen.getByRole('button', { name: 'Save Changes' });
		await vi.waitFor(() => {
			expect(saveButton).toBeEnabled();
		});

		await user.click(saveButton);
		await vi.waitFor(() => {
			// Should save the JWT without the "Bearer " prefix
			expect(mockApi.settings.update).toHaveBeenCalledWith({
				'metadata.hardcover.api_token': FAKE_JWT
			});
		});
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

		// Paste a raw JWT (no Bearer prefix)
		await user.click(tokenField);
		await user.paste(FAKE_JWT);

		const saveButton = screen.getByRole('button', { name: 'Save Changes' });
		await vi.waitFor(() => {
			expect(saveButton).toBeEnabled();
		});

		await user.click(saveButton);
		await vi.waitFor(() => {
			expect(mockApi.settings.update).toHaveBeenCalledWith({
				'metadata.hardcover.api_token': FAKE_JWT
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
