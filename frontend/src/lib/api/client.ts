import { goto } from '$app/navigation';
import { parseApiError } from './errors.js';
import type {
	AddIdentifierRequest,
	AddWatchedDirectoryRequest,
	AuthorResponse,
	AuthStatusResponse,
	BatchIsbnScanResponse,
	BatchSetTagsRequest,
	BatchUpdateBooksRequest,
	BatchUpdateResponse,
	BookDetail,
	BookListParams,
	BookmarkResponse,
	BrowseResponse,
	CandidateResponse,
	ContinueReadingItem,
	CreateAuthorRequest,
	CreateBookmarkRequest,
	CreatePublisherRequest,
	DetectFsRequest,
	DuplicateCountResponse,
	DuplicateLinkResponse,
	FlagDuplicateRequest,
	FsDetectionResponse,
	IdentifyAllResponse,
	IdentifyResponse,
	IsbnScanResponse,
	LoginRequest,
	LoginResponse,
	MergeRequest,
	PaginatedAuthors,
	PaginatedBooks,
	PaginatedDuplicates,
	PaginatedPublishers,
	PaginatedSeries,
	PaginatedTags,
	PublisherResponse,
	ReadingProgressResponse,
	ScanManifestResponse,
	ScanTriggeredResponse,
	SeriesResponse,
	SetBookAuthorsRequest,
	SetBookSeriesRequest,
	SetBookTagsRequest,
	SettingsResponse,
	SetupRequest,
	StatsResponse,
	TaskCreatedResponse,
	TaskResponse,
	UpdateBookRequest,
	UpdateIdentifierRequest,
	UpdateProgressRequest,
	UpdateSettingsResponse,
	UpdateWatchedDirectoryRequest,
	UploadResponse,
	User,
	WatchedDirectoryResponse
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
async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
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

		/** Batch update scalar fields across multiple books. */
		batchUpdate(data: BatchUpdateBooksRequest): Promise<BatchUpdateResponse> {
			return request<BatchUpdateResponse>('POST', '/books/batch-update', data);
		},

		/** Batch update tags across multiple books. */
		batchTags(data: BatchSetTagsRequest): Promise<BatchUpdateResponse> {
			return request<BatchUpdateResponse>('POST', '/books/batch-tags', data);
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

			const response = await fetch(`${BASE_URL}/books/${encodeURIComponent(id)}/cover`, {
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

			return (await response.json()) as BookDetail;
		}
	},

	identifiers: {
		/** Add a new identifier to a book. */
		add(bookId: string, data: AddIdentifierRequest): Promise<BookDetail> {
			return request<BookDetail>('POST', `/books/${encodeURIComponent(bookId)}/identifiers`, data);
		},

		/** Update an existing identifier. */
		update(
			bookId: string,
			identifierId: string,
			data: UpdateIdentifierRequest
		): Promise<BookDetail> {
			return request<BookDetail>(
				'PUT',
				`/books/${encodeURIComponent(bookId)}/identifiers/${encodeURIComponent(identifierId)}`,
				data
			);
		},

		/** Delete an identifier from a book. */
		delete(bookId: string, identifierId: string): Promise<void> {
			return request<void>(
				'DELETE',
				`/books/${encodeURIComponent(bookId)}/identifiers/${encodeURIComponent(identifierId)}`
			);
		}
	},

	duplicates: {
		/** List pending duplicate pairs with pagination. */
		list(params?: { page?: number; per_page?: number }): Promise<PaginatedDuplicates> {
			const searchParams = new URLSearchParams();
			if (params) {
				for (const [key, value] of Object.entries(params)) {
					if (value !== undefined && value !== null) {
						searchParams.set(key, String(value));
					}
				}
			}
			const qs = searchParams.toString();
			return request<PaginatedDuplicates>('GET', `/duplicates${qs ? `?${qs}` : ''}`);
		},

		/** Get a single duplicate link by ID. */
		get(id: string): Promise<DuplicateLinkResponse> {
			return request<DuplicateLinkResponse>('GET', `/duplicates/${encodeURIComponent(id)}`);
		},

		/** Merge a duplicate pair. */
		merge(id: string, data: MergeRequest): Promise<BookDetail> {
			return request<BookDetail>('POST', `/duplicates/${encodeURIComponent(id)}/merge`, data);
		},

		/** Dismiss a duplicate pair. */
		dismiss(id: string): Promise<void> {
			return request<void>('POST', `/duplicates/${encodeURIComponent(id)}/dismiss`);
		},

		/** Manually flag a book as a duplicate of another. */
		flag(bookId: string, otherBookId: string): Promise<DuplicateLinkResponse> {
			const body: FlagDuplicateRequest = { other_book_id: otherBookId };
			return request<DuplicateLinkResponse>(
				'POST',
				`/books/${encodeURIComponent(bookId)}/duplicates`,
				body
			);
		},

		/** Get the count of pending duplicates. */
		count(): Promise<DuplicateCountResponse> {
			return request<DuplicateCountResponse>('GET', '/duplicates/count');
		},

		/** Find duplicate links for a specific book. */
		forBook(bookId: string): Promise<DuplicateLinkResponse[]> {
			return request<DuplicateLinkResponse[]>(
				'GET',
				`/books/${encodeURIComponent(bookId)}/duplicates`
			);
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

		/** Create a new author. */
		create(data: CreateAuthorRequest): Promise<AuthorResponse> {
			return request<AuthorResponse>('POST', '/authors', data);
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

	filesystem: {
		/** Browse a server directory for files and subdirectories. */
		browse(path?: string, dirsOnly?: boolean): Promise<BrowseResponse> {
			const params = new URLSearchParams();
			if (path) params.set('path', path);
			if (dirsOnly) params.set('dirs_only', 'true');
			const qs = params.toString();
			return request<BrowseResponse>('GET', `/filesystem/browse${qs ? `?${qs}` : ''}`);
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
		/** List recent top-level tasks. */
		list(): Promise<TaskResponse[]> {
			return request<TaskResponse[]>('GET', '/tasks');
		},

		/** Get a single task by ID. */
		get(id: string): Promise<TaskResponse> {
			return request<TaskResponse>('GET', `/tasks/${encodeURIComponent(id)}`);
		},

		/** List child tasks of a parent. */
		children(id: string): Promise<TaskResponse[]> {
			return request<TaskResponse[]>('GET', `/tasks/${encodeURIComponent(id)}/children`);
		},

		/** Cancel a running or pending task. */
		cancel(id: string): Promise<TaskResponse> {
			return request<TaskResponse>('POST', `/tasks/${encodeURIComponent(id)}/cancel`);
		}
	},

	identify: {
		/** Trigger identification for a single book. */
		book(id: string): Promise<IdentifyResponse> {
			return request<IdentifyResponse>('POST', `/books/${encodeURIComponent(id)}/identify`);
		},

		/** List identification candidates for a book. */
		candidates(bookId: string): Promise<CandidateResponse[]> {
			return request<CandidateResponse[]>('GET', `/books/${encodeURIComponent(bookId)}/candidates`);
		},

		/** Apply a candidate to a book, updating its metadata. */
		applyCandidate(
			bookId: string,
			candidateId: string,
			excludeFields?: string[]
		): Promise<BookDetail> {
			const body =
				excludeFields && excludeFields.length > 0 ? { exclude_fields: excludeFields } : undefined;
			return request<BookDetail>(
				'POST',
				`/books/${encodeURIComponent(bookId)}/candidates/${encodeURIComponent(candidateId)}/apply`,
				body
			);
		},

		/** Reject a candidate. */
		rejectCandidate(bookId: string, candidateId: string): Promise<void> {
			return request<void>(
				'POST',
				`/books/${encodeURIComponent(bookId)}/candidates/${encodeURIComponent(candidateId)}/reject`
			);
		},

		/** Undo an applied candidate, restoring all candidates to pending. */
		undoCandidate(bookId: string, candidateId: string): Promise<BookDetail> {
			return request<BookDetail>(
				'POST',
				`/books/${encodeURIComponent(bookId)}/candidates/${encodeURIComponent(candidateId)}/undo`
			);
		},

		/** Trigger identification for multiple books. */
		batch(bookIds: string[]): Promise<IdentifyResponse[]> {
			return request<IdentifyResponse[]>('POST', '/identify/batch', { book_ids: bookIds });
		},

		/** Identify all unidentified/needs-review books. */
		all(maxBooks?: number): Promise<IdentifyAllResponse> {
			return request<IdentifyAllResponse>('POST', '/identify/all', {
				max_books: maxBooks
			});
		}
	},

	isbnScan: {
		/** Trigger ISBN content scan for a single book. */
		scanBook(id: string): Promise<IsbnScanResponse> {
			return request<IsbnScanResponse>('POST', `/isbn-scan/book/${encodeURIComponent(id)}`);
		},

		/** Trigger ISBN content scan for multiple books. */
		scanBatch(bookIds: string[]): Promise<BatchIsbnScanResponse> {
			return request<BatchIsbnScanResponse>('POST', '/isbn-scan/batch', {
				book_ids: bookIds
			});
		}
	},

	settings: {
		/** Fetch all instance settings. */
		get(): Promise<SettingsResponse> {
			return request<SettingsResponse>('GET', '/settings');
		},

		/** Update one or more settings. Null value = reset to default. */
		update(settings: Record<string, unknown>): Promise<UpdateSettingsResponse> {
			return request<UpdateSettingsResponse>('PUT', '/settings', { settings });
		}
	},

	watchedDirectories: {
		/** List all watched directories. */
		list(): Promise<WatchedDirectoryResponse[]> {
			return request<WatchedDirectoryResponse[]>('GET', '/watched-directories');
		},

		/** Add a new watched directory. */
		add(data: AddWatchedDirectoryRequest): Promise<WatchedDirectoryResponse> {
			return request<WatchedDirectoryResponse>('POST', '/watched-directories', data);
		},

		/** Update an existing watched directory. */
		update(
			id: string,
			data: UpdateWatchedDirectoryRequest
		): Promise<WatchedDirectoryResponse> {
			return request<WatchedDirectoryResponse>(
				'PUT',
				`/watched-directories/${encodeURIComponent(id)}`,
				data
			);
		},

		/** Delete a watched directory. */
		delete(id: string): Promise<void> {
			return request<void>('DELETE', `/watched-directories/${encodeURIComponent(id)}`);
		},

		/** Trigger a manual full scan of a watched directory. */
		triggerScan(id: string): Promise<ScanTriggeredResponse> {
			return request<ScanTriggeredResponse>(
				'POST',
				`/watched-directories/${encodeURIComponent(id)}/scan`
			);
		},

		/** Detect filesystem type for a given path. */
		detectFilesystem(path: string): Promise<FsDetectionResponse> {
			const body: DetectFsRequest = { path };
			return request<FsDetectionResponse>('POST', '/watched-directories/detect', body);
		}
	},

	stats: {
		/** Fetch library and usage statistics. */
		get(): Promise<StatsResponse> {
			return request<StatsResponse>('GET', '/stats');
		}
	},

	reader: {
		/** Get reading progress for a book. Returns null if no progress exists. */
		async getProgress(bookId: string): Promise<ReadingProgressResponse | null> {
			try {
				return await request<ReadingProgressResponse>(
					'GET',
					`/reader/progress/${encodeURIComponent(bookId)}`
				);
			} catch (err: unknown) {
				if (err && typeof err === 'object' && 'status' in err && err.status === 404) {
					return null;
				}
				throw err;
			}
		},

		/** Update reading progress for a book file. */
		updateProgress(
			bookId: string,
			fileId: string,
			data: UpdateProgressRequest
		): Promise<ReadingProgressResponse> {
			return request<ReadingProgressResponse>(
				'PUT',
				`/reader/progress/${encodeURIComponent(bookId)}/${encodeURIComponent(fileId)}`,
				data
			);
		},

		/** Clear reading progress for a book. */
		clearProgress(bookId: string): Promise<void> {
			return request<void>('DELETE', `/reader/progress/${encodeURIComponent(bookId)}`);
		},

		/** Get continue-reading list. */
		continueReading(limit?: number): Promise<ContinueReadingItem[]> {
			const params = new URLSearchParams();
			if (limit !== undefined) params.set('limit', String(limit));
			const qs = params.toString();
			return request<ContinueReadingItem[]>(
				'GET',
				`/reader/continue-reading${qs ? `?${qs}` : ''}`
			);
		},

		/** List bookmarks for a book file. */
		listBookmarks(bookId: string, fileId: string): Promise<BookmarkResponse[]> {
			return request<BookmarkResponse[]>(
				'GET',
				`/reader/bookmarks/${encodeURIComponent(bookId)}/${encodeURIComponent(fileId)}`
			);
		},

		/** Create a bookmark for a book file. */
		createBookmark(
			bookId: string,
			fileId: string,
			data: CreateBookmarkRequest
		): Promise<BookmarkResponse> {
			return request<BookmarkResponse>(
				'POST',
				`/reader/bookmarks/${encodeURIComponent(bookId)}/${encodeURIComponent(fileId)}`,
				data
			);
		},

		/** Delete a bookmark by ID. */
		deleteBookmark(bookmarkId: string): Promise<void> {
			return request<void>(
				'DELETE',
				`/reader/bookmarks/${encodeURIComponent(bookmarkId)}`
			);
		},

		/** Fetch a book file as a Blob for the reader. */
		async fetchFileBlob(bookId: string, fileId: string): Promise<Blob> {
			const headers: Record<string, string> = {
				Accept: 'application/octet-stream'
			};
			const token = getSessionToken();
			if (token) {
				headers['Authorization'] = `Bearer ${token}`;
			}

			const response = await fetch(
				`${BASE_URL}/books/${encodeURIComponent(bookId)}/files/${encodeURIComponent(fileId)}/content`,
				{ method: 'GET', headers }
			);

			if (!response.ok) {
				const error = await parseApiError(response);
				if (error.isUnauthorized) {
					handleUnauthorized();
				}
				throw error;
			}

			return response.blob();
		}
	}
} as const;

export { ApiError } from './errors.js';
export type {
	AddIdentifierRequest,
	AddWatchedDirectoryRequest,
	ApiErrorResponse,
	AuthorEntry,
	AuthorResponse,
	AuthStatusResponse,
	BatchIsbnScanResponse,
	BatchSetTagsRequest,
	BatchUpdateBooksRequest,
	BatchUpdateResponse,
	BookAuthorLink,
	BookDetail,
	BookFormat,
	BookListParams,
	BookmarkResponse,
	BookSeriesLink,
	BookSummary,
	BookTagLink,
	BrowseResponse,
	CandidateResponse,
	CandidateSeriesInfo,
	ChildrenSummary,
	ConfigSource,
	ContinueReadingItem,
	CreateAuthorRequest,
	CreateBookmarkRequest,
	CreatePublisherRequest,
	DetectFsRequest,
	DuplicateCountResponse,
	DuplicateLinkResponse,
	FileEntry,
	FlagDuplicateRequest,
	FormatSummary,
	FsDetectionResponse,
	FsEntry,
	IdentifierEntry,
	IdentifyAllResponse,
	IdentifyResponse,
	IsbnScanResponse,
	LoginRequest,
	LoginResponse,
	MergeRequest,
	MetadataSource,
	MetadataStatus,
	PaginatedAuthors,
	PaginatedBooks,
	PaginatedDuplicates,
	PaginatedPublishers,
	PaginatedSeries,
	PaginatedTags,
	PublisherResponse,
	ReadingProgressResponse,
	ScanManifestResponse,
	ScanTriggeredResponse,
	SeriesEntry,
	SeriesResponse,
	SetBookAuthorsRequest,
	SetBookSeriesRequest,
	SetBookTagsRequest,
	StatsResponse,
	SettingEntry,
	SettingOverride,
	SettingsResponse,
	SettingType,
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
	TocItem,
	UpdateBookRequest,
	UpdateIdentifierRequest,
	UpdateProgressRequest,
	UpdateSettingsResponse,
	UpdateWatchedDirectoryRequest,
	UploadResponse,
	User,
	UserRole,
	WatchedDirectoryResponse,
	WatchMode
} from './types.js';
