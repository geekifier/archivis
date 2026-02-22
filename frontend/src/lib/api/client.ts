import { goto } from '$app/navigation';
import { parseApiError } from './errors.js';
import type {
	AuthorResponse,
	AuthStatusResponse,
	BookDetail,
	BookListParams,
	CreatePublisherRequest,
	LoginRequest,
	LoginResponse,
	PaginatedAuthors,
	PaginatedBooks,
	PaginatedPublishers,
	PaginatedSeries,
	PaginatedTags,
	PublisherResponse,
	ScanManifestResponse,
	SeriesResponse,
	SetBookAuthorsRequest,
	SetBookSeriesRequest,
	SetBookTagsRequest,
	SetupRequest,
	TaskCreatedResponse,
	TaskResponse,
	UpdateBookRequest,
	UploadResponse,
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
		},

		/** Fetch full book detail by ID. */
		get(id: string): Promise<BookDetail> {
			return request<BookDetail>('GET', `/books/${encodeURIComponent(id)}`);
		},

		/** Partial-update scalar book fields. */
		update(id: string, data: UpdateBookRequest): Promise<BookDetail> {
			return request<BookDetail>('PUT', `/books/${encodeURIComponent(id)}`, data);
		},

		/** Replace all author links for a book. */
		setAuthors(id: string, data: SetBookAuthorsRequest): Promise<BookDetail> {
			return request<BookDetail>('POST', `/books/${encodeURIComponent(id)}/authors`, data);
		},

		/** Replace all series links for a book. */
		setSeries(id: string, data: SetBookSeriesRequest): Promise<BookDetail> {
			return request<BookDetail>('POST', `/books/${encodeURIComponent(id)}/series`, data);
		},

		/** Replace all tag links for a book. */
		setTags(id: string, data: SetBookTagsRequest): Promise<BookDetail> {
			return request<BookDetail>('POST', `/books/${encodeURIComponent(id)}/tags`, data);
		},

		/** Delete a book and all associated files. */
		delete(id: string): Promise<void> {
			return request<void>('DELETE', `/books/${encodeURIComponent(id)}`);
		},

		/** Upload or replace the cover image for a book. */
		async uploadCover(id: string, file: File): Promise<BookDetail> {
			const formData = new FormData();
			formData.append('file', file);

			const headers: Record<string, string> = {
				Accept: 'application/json'
			};
			const token = getSessionToken();
			if (token) {
				headers['Authorization'] = `Bearer ${token}`;
			}
			// Do NOT set Content-Type — let the browser set the multipart boundary.

			const response = await fetch(
				`${BASE_URL}/books/${encodeURIComponent(id)}/cover`,
				{
					method: 'POST',
					headers,
					body: formData
				}
			);

			if (!response.ok) {
				const error = await parseApiError(response);
				if (error.isUnauthorized) {
					handleUnauthorized();
				}
				throw error;
			}

			return (await response.json()) as BookDetail;
		}
	},

	authors: {
		/** Fetch author detail by ID. */
		get(id: string): Promise<AuthorResponse> {
			return request<AuthorResponse>('GET', `/authors/${encodeURIComponent(id)}`);
		},

		/** Search authors by query string. */
		search(q: string): Promise<PaginatedAuthors> {
			const params = new URLSearchParams({ q, per_page: '10' });
			return request<PaginatedAuthors>('GET', `/authors?${params.toString()}`);
		},

		/** List books by a specific author. */
		listBooks(id: string, params?: BookListParams): Promise<PaginatedBooks> {
			const searchParams = new URLSearchParams();
			if (params) {
				for (const [key, value] of Object.entries(params)) {
					if (value !== undefined && value !== null && value !== '') {
						searchParams.set(key, String(value));
					}
				}
			}
			const qs = searchParams.toString();
			return request<PaginatedBooks>(
				'GET',
				`/authors/${encodeURIComponent(id)}/books${qs ? `?${qs}` : ''}`
			);
		}
	},

	tags: {
		/** Search tags by query string. */
		search(q: string): Promise<PaginatedTags> {
			const params = new URLSearchParams({ q, per_page: '10' });
			return request<PaginatedTags>('GET', `/tags?${params.toString()}`);
		}
	},

	publishers: {
		/** Search publishers by query string. */
		search(q: string): Promise<PaginatedPublishers> {
			const params = new URLSearchParams({ q, per_page: '10' });
			return request<PaginatedPublishers>('GET', `/publishers?${params.toString()}`);
		},

		/** Create a new publisher. */
		create(data: CreatePublisherRequest): Promise<PublisherResponse> {
			return request<PublisherResponse>('POST', '/publishers', data);
		}
	},

	series: {
		/** Fetch series detail by ID. */
		get(id: string): Promise<SeriesResponse> {
			return request<SeriesResponse>('GET', `/series/${encodeURIComponent(id)}`);
		},

		/** Search series by query string. */
		search(q: string): Promise<PaginatedSeries> {
			const params = new URLSearchParams({ q, per_page: '10' });
			return request<PaginatedSeries>('GET', `/series?${params.toString()}`);
		},

		/** List books in a specific series. */
		listBooks(id: string, params?: BookListParams): Promise<PaginatedBooks> {
			const searchParams = new URLSearchParams();
			if (params) {
				for (const [key, value] of Object.entries(params)) {
					if (value !== undefined && value !== null && value !== '') {
						searchParams.set(key, String(value));
					}
				}
			}
			const qs = searchParams.toString();
			return request<PaginatedBooks>(
				'GET',
				`/series/${encodeURIComponent(id)}/books${qs ? `?${qs}` : ''}`
			);
		}
	},

	import: {
		/** Upload one or more ebook files via multipart form data. */
		async upload(files: File[]): Promise<UploadResponse> {
			const formData = new FormData();
			for (const file of files) {
				formData.append('file', file);
			}

			const headers: Record<string, string> = {
				Accept: 'application/json'
			};
			const token = getSessionToken();
			if (token) {
				headers['Authorization'] = `Bearer ${token}`;
			}
			// Do NOT set Content-Type — let the browser set the multipart boundary.

			const response = await fetch(`${BASE_URL}/import/upload`, {
				method: 'POST',
				headers,
				body: formData
			});

			if (!response.ok) {
				const error = await parseApiError(response);
				if (error.isUnauthorized) {
					handleUnauthorized();
				}
				throw error;
			}

			return (await response.json()) as UploadResponse;
		},

		/** Scan a directory for importable ebook files. */
		scan(path: string): Promise<ScanManifestResponse> {
			return request<ScanManifestResponse>('POST', '/import/scan', { path });
		},

		/** Start bulk import from a previously scanned directory. */
		startImport(path: string): Promise<TaskCreatedResponse> {
			return request<TaskCreatedResponse>('POST', '/import/scan/start', { path });
		}
	},

	tasks: {
		/** List recent tasks. */
		list(): Promise<TaskResponse[]> {
			return request<TaskResponse[]>('GET', '/tasks');
		},

		/** Get a single task by ID. */
		get(id: string): Promise<TaskResponse> {
			return request<TaskResponse>('GET', `/tasks/${encodeURIComponent(id)}`);
		}
	}
} as const;

export { ApiError } from './errors.js';
export type {
	ApiErrorResponse,
	AuthorEntry,
	AuthorResponse,
	AuthStatusResponse,
	BookAuthorLink,
	BookDetail,
	BookFormat,
	BookListParams,
	BookSeriesLink,
	BookSummary,
	BookTagLink,
	CreatePublisherRequest,
	FileEntry,
	FormatSummary,
	IdentifierEntry,
	LoginRequest,
	LoginResponse,
	MetadataSource,
	MetadataStatus,
	PaginatedAuthors,
	PaginatedBooks,
	PaginatedPublishers,
	PaginatedSeries,
	PaginatedTags,
	PublisherResponse,
	ScanManifestResponse,
	SeriesEntry,
	SeriesResponse,
	SetBookAuthorsRequest,
	SetBookSeriesRequest,
	SetBookTagsRequest,
	SetupRequest,
	SortField,
	SortOrder,
	TagEntry,
	TagResponse,
	TaskCreatedResponse,
	TaskProgressEvent,
	TaskResponse,
	TaskStatus,
	TaskType,
	UpdateBookRequest,
	UploadResponse,
	User,
	UserRole
} from './types.js';
