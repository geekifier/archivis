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

/** Metadata source — tagged union matching Rust's MetadataSource enum. */
export type MetadataSource =
	| { type: 'embedded' }
	| { type: 'filename' }
	| { type: 'provider'; name: string }
	| { type: 'user' };

export interface IdentifierEntry {
	id: string;
	identifier_type: string;
	value: string;
	source: MetadataSource;
	confidence: number;
}

/** Full book detail with all relations (from GET /api/books/{id}). */
export interface BookDetail {
	id: string;
	title: string;
	sort_title: string;
	description: string | null;
	language: string | null;
	publication_date: string | null;
	publisher_name: string | null;
	added_at: string;
	updated_at: string;
	rating: number | null;
	page_count: number | null;
	metadata_status: MetadataStatus;
	metadata_confidence: number;
	has_cover: boolean;
	authors: AuthorEntry[];
	series: SeriesEntry[];
	tags: TagEntry[];
	files: FileEntry[];
	identifiers: IdentifierEntry[];
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

// --- Update/edit request types ---

export interface UpdateBookRequest {
	title?: string;
	description?: string;
	language?: string;
	publication_date?: string; // "YYYY-MM-DD"
	rating?: number; // 0.0-5.0
	page_count?: number;
	metadata_status?: MetadataStatus;
}

export interface BookAuthorLink {
	author_id: string;
	role?: string; // defaults to "author"
	position?: number;
}

export interface SetBookAuthorsRequest {
	authors: BookAuthorLink[];
}

export interface BookTagLink {
	tag_id?: string;
	name?: string;
	category?: string;
}

export interface SetBookTagsRequest {
	tags: BookTagLink[];
}

// --- Autocomplete response types ---

export interface AuthorResponse {
	id: string;
	name: string;
	sort_name: string;
}

export interface TagResponse {
	id: string;
	name: string;
	category: string | null;
}

export interface SeriesResponse {
	id: string;
	name: string;
	description: string | null;
}

export interface PaginatedAuthors {
	items: AuthorResponse[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

export interface PaginatedTags {
	items: TagResponse[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

export interface PaginatedSeries {
	items: SeriesResponse[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

// --- Import types ---

export type TaskType = 'import_file' | 'import_directory';
export type TaskStatus = 'pending' | 'running' | 'completed' | 'failed';

export interface TaskCreatedResponse {
	task_id: string;
}

export interface UploadResponse {
	tasks: TaskCreatedResponse[];
}

export interface FormatSummary {
	format: string;
	count: number;
	total_size: number;
}

export interface ScanManifestResponse {
	total_files: number;
	total_size: number;
	formats: FormatSummary[];
}

export interface TaskResponse {
	id: string;
	task_type: TaskType;
	status: TaskStatus;
	progress: number;
	message: string | null;
	result: Record<string, unknown> | null;
	created_at: string;
	started_at: string | null;
	completed_at: string | null;
	error_message: string | null;
}

/** SSE progress event data for a single task. */
export interface TaskProgressEvent {
	task_id: string;
	status: TaskStatus;
	progress: number;
	message: string | null;
	result: Record<string, unknown> | null;
	error: string | null;
}
