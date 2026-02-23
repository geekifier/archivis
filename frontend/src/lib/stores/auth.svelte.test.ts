import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { goto } from '$app/navigation';
import { ApiError } from '$lib/api/errors.js';

// Mock the API module before importing the store
vi.mock('$lib/api/index.js', async () => {
	const { createMockApi } = await import('$lib/test-utils/api-mock.js');
	const mockApi = createMockApi();
	return {
		api: mockApi,
		setSessionToken: vi.fn(),
		getSessionToken: vi.fn(),
		ApiError: (await import('$lib/api/errors.js')).ApiError
	};
});

// Import after mocking
const { api, setSessionToken, getSessionToken } = await import('$lib/api/index.js');
const { auth } = await import('./auth.svelte.js');

const mockApi = api as unknown as import('$lib/test-utils/api-mock.js').MockApi;
const mockSetSessionToken = setSessionToken as unknown as ReturnType<typeof vi.fn>;
const mockGetSessionToken = getSessionToken as unknown as ReturnType<typeof vi.fn>;

describe('auth store', () => {
	beforeEach(() => {
		vi.mocked(goto).mockReset();
		mockSetSessionToken.mockReset();
		mockGetSessionToken.mockReset();
		// Reset all mock API functions
		for (const group of Object.values(mockApi)) {
			for (const fn of Object.values(group as Record<string, ReturnType<typeof vi.fn>>)) {
				fn.mockReset();
			}
		}
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('has correct initial state', () => {
		// The store starts with loading=true, user=null, setupRequired=null
		expect(auth.user).toBeNull();
		expect(auth.setupRequired).toBeNull();
		expect(auth.isAuthenticated).toBe(false);
	});

	describe('checkAuth', () => {
		it('sets setupRequired to true when setup is required', async () => {
			mockApi.auth.status.mockResolvedValue({ setup_required: true });

			await auth.checkAuth();

			expect(auth.setupRequired).toBe(true);
			expect(auth.user).toBeNull();
			expect(auth.loading).toBe(false);
		});

		it('leaves user null when no token exists', async () => {
			mockApi.auth.status.mockResolvedValue({ setup_required: false });
			mockGetSessionToken.mockReturnValue(null);

			await auth.checkAuth();

			expect(auth.setupRequired).toBe(false);
			expect(auth.user).toBeNull();
			expect(auth.loading).toBe(false);
		});

		it('sets user from api.auth.me when token is valid', async () => {
			const mockUser = {
				id: '1',
				username: 'admin',
				email: null,
				role: 'admin' as const,
				is_active: true,
				created_at: '2024-01-01T00:00:00Z'
			};
			mockApi.auth.status.mockResolvedValue({ setup_required: false });
			mockGetSessionToken.mockReturnValue('valid-token');
			mockApi.auth.me.mockResolvedValue(mockUser);

			await auth.checkAuth();

			expect(auth.user).toEqual(mockUser);
			expect(auth.isAuthenticated).toBe(true);
			expect(auth.loading).toBe(false);
		});

		it('clears token and user when api.auth.me throws 401', async () => {
			mockApi.auth.status.mockResolvedValue({ setup_required: false });
			mockGetSessionToken.mockReturnValue('expired-token');
			mockApi.auth.me.mockRejectedValue(new ApiError(401, 'Unauthorized'));

			await auth.checkAuth();

			expect(mockSetSessionToken).toHaveBeenCalledWith(null);
			expect(auth.user).toBeNull();
			expect(auth.loading).toBe(false);
		});
	});

	describe('login', () => {
		it('calls api.auth.login and sets user', async () => {
			const mockUser = {
				id: '1',
				username: 'admin',
				email: null,
				role: 'admin' as const,
				is_active: true,
				created_at: '2024-01-01T00:00:00Z'
			};
			mockApi.auth.login.mockResolvedValue({ token: 'new-token', user: mockUser });

			await auth.login('admin', 'password');

			expect(mockApi.auth.login).toHaveBeenCalledWith({
				username: 'admin',
				password: 'password'
			});
			expect(auth.user).toEqual(mockUser);
			expect(auth.setupRequired).toBe(false);
		});
	});

	describe('logout', () => {
		it('calls api.auth.logout, clears user, and navigates to /login', async () => {
			mockApi.auth.logout.mockResolvedValue(undefined);

			await auth.logout();

			expect(mockApi.auth.logout).toHaveBeenCalled();
			expect(auth.user).toBeNull();
			expect(goto).toHaveBeenCalledWith('/login');
		});

		it('still clears user and navigates even if logout throws', async () => {
			mockApi.auth.logout.mockRejectedValue(new Error('Network error'));

			// The error propagates through the finally block, but user is still cleared
			await expect(auth.logout()).rejects.toThrow('Network error');

			expect(auth.user).toBeNull();
			expect(goto).toHaveBeenCalledWith('/login');
		});
	});

	describe('setup', () => {
		it('calls api.auth.setup then auto-logs in', async () => {
			const mockUser = {
				id: '1',
				username: 'newadmin',
				email: 'admin@test.com',
				role: 'admin' as const,
				is_active: true,
				created_at: '2024-01-01T00:00:00Z'
			};
			mockApi.auth.setup.mockResolvedValue(mockUser);
			mockApi.auth.login.mockResolvedValue({ token: 'new-token', user: mockUser });

			await auth.setup('newadmin', 'password', 'admin@test.com');

			expect(mockApi.auth.setup).toHaveBeenCalledWith({
				username: 'newadmin',
				password: 'password',
				email: 'admin@test.com'
			});
			// Auto-login should have been called
			expect(mockApi.auth.login).toHaveBeenCalledWith({
				username: 'newadmin',
				password: 'password'
			});
			expect(auth.user).toEqual(mockUser);
		});
	});
});
