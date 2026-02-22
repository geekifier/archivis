import { goto } from '$app/navigation';
import { parseApiError } from './errors.js';
import type {
	AuthStatusResponse,
	BookListParams,
	LoginRequest,
	LoginResponse,
	PaginatedBooks,
	SetupRequest,
	User
} from './types.js';

const BASE_URL = '/api';

/** Session token stored in memory (cookie is the primary auth mechanism). */
let sessionToken: string | null = null;

export function setSessionToken(token: string | null) {
	sessionToken = token;
	if (token) {
		localStorage.setItem('archivis-session', token);
	} else {
		localStorage.removeItem('archivis-session');
	}
}

export function getSessionToken(): string | null {
	if (sessionToken) return sessionToken;
	if (typeof localStorage !== 'undefined') {
		sessionToken = localStorage.getItem('archivis-session');
	}
	return sessionToken;
}

/** Clear session state and redirect to login. */
function handleUnauthorized() {
	setSessionToken(null);
	// Avoid redirect loops if already on auth pages
	if (typeof window !== 'undefined') {
		const path = window.location.pathname;
		if (path !== '/login' && path !== '/setup') {
			goto('/login');
		}
	}
}

/**
 * Make an authenticated API request.
 * Automatically attaches the session token as a Bearer header
 * and handles 401 responses by clearing the session and redirecting.
 */
async function request<T>(
	method: string,
	path: string,
	body?: unknown
): Promise<T> {
	const headers: Record<string, string> = {
		Accept: 'application/json'
	};

	const token = getSessionToken();
	if (token) {
		headers['Authorization'] = `Bearer ${token}`;
	}

	if (body !== undefined) {
		headers['Content-Type'] = 'application/json';
	}

	const response = await fetch(`${BASE_URL}${path}`, {
		method,
		headers,
		body: body !== undefined ? JSON.stringify(body) : undefined
	});

	if (!response.ok) {
		const error = await parseApiError(response);
		if (error.isUnauthorized) {
			handleUnauthorized();
		}
		throw error;
	}

	// 204 No Content
	if (response.status === 204) {
		return undefined as T;
	}

	return (await response.json()) as T;
}

/** Type-safe API methods grouped by domain. */
export const api = {
	auth: {
		/** Check whether initial admin setup is required. */
		status(): Promise<AuthStatusResponse> {
			return request<AuthStatusResponse>('GET', '/auth/status');
		},

		/** Create the initial admin user. */
		setup(data: SetupRequest): Promise<User> {
			return request<User>('POST', '/auth/setup', data);
		},

		/** Authenticate and receive a session token. */
		async login(data: LoginRequest): Promise<LoginResponse> {
			const result = await request<LoginResponse>('POST', '/auth/login', data);
			setSessionToken(result.token);
			return result;
		},

		/** Invalidate the current session. */
		async logout(): Promise<void> {
			try {
				await request<void>('POST', '/auth/logout');
			} finally {
				setSessionToken(null);
			}
		},

		/** Get the currently authenticated user. */
		me(): Promise<User> {
			return request<User>('GET', '/auth/me');
		}
	},

	books: {
		list(params?: BookListParams): Promise<PaginatedBooks> {
			const searchParams = new URLSearchParams();
			if (params) {
				for (const [key, value] of Object.entries(params)) {
					if (value !== undefined && value !== null && value !== '') {
						searchParams.set(key, String(value));
					}
				}
			}
			const qs = searchParams.toString();
			return request<PaginatedBooks>('GET', `/books${qs ? `?${qs}` : ''}`);
		}
	}
} as const;

export { ApiError } from './errors.js';
export type {
	ApiErrorResponse,
	AuthorEntry,
	AuthStatusResponse,
	BookFormat,
	BookListParams,
	BookSummary,
	FileEntry,
	LoginRequest,
	LoginResponse,
	MetadataStatus,
	PaginatedBooks,
	SeriesEntry,
	SetupRequest,
	SortField,
	SortOrder,
	TagEntry,
	User,
	UserRole
} from './types.js';
