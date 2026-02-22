/**
 * Hand-written API types for auth endpoints.
 *
 * These mirror the Rust backend types and are used when the backend
 * is not running (so openapi-typescript can't generate from the spec).
 * When generated types are available via `npm run generate:api`, prefer those.
 */

export type UserRole = 'admin' | 'user';

export interface User {
	id: string;
	username: string;
	email: string | null;
	role: UserRole;
	is_active: boolean;
	created_at: string;
}

export interface AuthStatusResponse {
	setup_required: boolean;
}

export interface LoginRequest {
	username: string;
	password: string;
}

export interface LoginResponse {
	token: string;
	user: User;
}

export interface SetupRequest {
	username: string;
	password: string;
	email?: string;
}

/** Error envelope returned by the backend for all error responses. */
export interface ApiErrorResponse {
	error: {
		status: number;
		message: string;
	};
}

// --- Book types ---

export type MetadataStatus = 'identified' | 'needs_review' | 'unidentified';
export type BookFormat = 'epub' | 'pdf' | 'mobi' | 'cbz' | 'fb2' | 'txt' | 'djvu' | 'azw3';
export type SortField = 'added_at' | 'title' | 'sort_title' | 'updated_at' | 'rating' | 'metadata_status';
export type SortOrder = 'asc' | 'desc';

export interface AuthorEntry {
	id: string;
	name: string;
	sort_name: string;
	role: string;
	position: number;
}

export interface SeriesEntry {
	id: string;
	name: string;
	description: string | null;
	position: number | null;
}

export interface TagEntry {
	id: string;
	name: string;
	category: string | null;
}

export interface FileEntry {
	id: string;
	format: BookFormat;
	file_size: number;
	hash: string;
	added_at: string;
}

export interface BookSummary {
	id: string;
	title: string;
	sort_title: string;
	description: string | null;
	language: string | null;
	publication_date: string | null;
	added_at: string;
	updated_at: string;
	rating: number | null;
	page_count: number | null;
	metadata_status: MetadataStatus;
	metadata_confidence: number;
	has_cover: boolean;
	authors?: AuthorEntry[];
	series?: SeriesEntry[];
	tags?: TagEntry[];
	files?: FileEntry[];
}

export interface PaginatedBooks {
	items: BookSummary[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

export interface BookListParams {
	page?: number;
	per_page?: number;
	sort_by?: SortField;
	sort_order?: SortOrder;
	q?: string;
	format?: BookFormat;
	status?: MetadataStatus;
	include?: string;
}
