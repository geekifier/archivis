import { goto } from '$app/navigation';
import { api, setSessionToken, getSessionToken } from '$lib/api/index.js';
import { ApiError } from '$lib/api/errors.js';
import type { User } from '$lib/api/types.js';
import { navCounts } from '$lib/stores/nav-counts.svelte.js';

function createAuthStore() {
	let user = $state<User | null>(null);
	let loading = $state(true);
	let setupRequired = $state<boolean | null>(null);

	const isAuthenticated = $derived(user !== null);

	/**
	 * Check auth status on app load.
	 * Determines if setup is required, then validates the current session.
	 */
	async function checkAuth(): Promise<void> {
		loading = true;
		try {
			const status = await api.auth.status();
			setupRequired = status.setup_required;

			if (status.setup_required) {
				user = null;
				return;
			}

			const token = getSessionToken();
			if (token) {
				try {
					user = await api.auth.me();
				} catch (err) {
					if (err instanceof ApiError && err.isUnauthorized) {
						setSessionToken(null);
						user = null;
					} else {
						throw err;
					}
				}
			} else {
				user = null;
			}
		} catch (err) {
			// If we can't reach the API at all, treat as unauthenticated
			if (!(err instanceof ApiError)) {
				user = null;
				setupRequired = null;
			}
		} finally {
			loading = false;
		}
	}

	async function login(username: string, password: string): Promise<void> {
		const result = await api.auth.login({ username, password });
		user = result.user;
		setupRequired = false;
	}

	async function logout(): Promise<void> {
		try {
			await api.auth.logout();
		} finally {
			user = null;
			navCounts.reset();
			goto('/login');
		}
	}

	async function setup(username: string, password: string, email?: string): Promise<void> {
		await api.auth.setup({ username, password, email: email || undefined });
		// Auto-login after setup
		await login(username, password);
	}

	return {
		get user() {
			return user;
		},
		get loading() {
			return loading;
		},
		get setupRequired() {
			return setupRequired;
		},
		get isAuthenticated() {
			return isAuthenticated;
		},
		checkAuth,
		login,
		logout,
		setup
	};
}

export const auth = createAuthStore();
