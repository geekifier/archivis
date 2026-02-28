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
export type SortField =
	| 'added_at'
	| 'title'
	| 'sort_title'
	| 'updated_at'
	| 'rating'
	| 'metadata_status'
	| 'author'
	| 'series';
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
	format_version?: string | null;
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
	| { type: 'user' }
	| { type: 'content_scan' };

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
	publisher_id: string | null;
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

export interface AuthorListParams {
	page?: number;
	per_page?: number;
	sort_by?: 'sort_name' | 'name' | 'book_count';
	sort_order?: SortOrder;
	q?: string;
}

export interface SeriesListParams {
	page?: number;
	per_page?: number;
	sort_by?: 'name' | 'book_count';
	sort_order?: SortOrder;
	q?: string;
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
	/** Set to a UUID to assign a publisher, or null to clear. Omit to leave unchanged. */
	publisher_id?: string | null;
}

export interface BookAuthorLink {
	author_id: string;
	role?: string; // defaults to "author"
	position?: number;
}

export interface SetBookAuthorsRequest {
	authors: BookAuthorLink[];
}

export interface BookSeriesLink {
	series_id: string;
	position?: number | null;
}

export interface SetBookSeriesRequest {
	series: BookSeriesLink[];
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
	book_count: number;
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
	book_count: number;
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

export interface PublisherResponse {
	id: string;
	name: string;
}

export interface PaginatedPublishers {
	items: PublisherResponse[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

export interface CreatePublisherRequest {
	name: string;
}

export interface CreateAuthorRequest {
	name: string;
	sort_name?: string;
}

// --- Filesystem types ---

export interface FsEntry {
	name: string;
	is_dir: boolean;
	size: number;
}

export interface BrowseResponse {
	path: string;
	parent: string | null;
	entries: FsEntry[];
	file_count: number;
}

// --- Import types ---

export type TaskType = 'import_file' | 'import_directory' | 'identify_book' | 'scan_isbn';
export type TaskStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';

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

export interface ChildrenSummary {
	total: number;
	pending: number;
	running: number;
	completed: number;
	failed: number;
	cancelled: number;
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
	parent_task_id: string | null;
	children_summary?: ChildrenSummary | null;
}

/** SSE progress event data for a single task. */
export interface TaskProgressEvent {
	task_id: string;
	status: TaskStatus;
	progress: number;
	message: string | null;
	result: Record<string, unknown> | null;
	error: string | null;
	parent_task_id?: string | null;
	data?: Record<string, unknown> | null;
}

// --- Identifier management types ---

export interface AddIdentifierRequest {
	identifier_type: string; // "isbn13", "isbn10", "asin", etc.
	value: string;
}

export interface UpdateIdentifierRequest {
	identifier_type?: string;
	value?: string;
}

// --- UI helper types ---

export interface SidebarCountsResponse {
	duplicates: number;
	needs_review: number;
	unidentified: number;
}

// --- Duplicate types ---

export interface DuplicateLinkResponse {
	id: string;
	book_a: BookSummary;
	book_b: BookSummary;
	detection_method: string;
	confidence: number;
	status: string;
	created_at: string;
}

export interface PaginatedDuplicates {
	items: DuplicateLinkResponse[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

export interface MergeRequest {
	primary_id: string;
	secondary_id: string;
	prefer_metadata_from?: string;
}

export interface FlagDuplicateRequest {
	other_book_id: string;
}

export interface DuplicateCountResponse {
	count: number;
}

// --- Batch update types ---

export interface BatchUpdateBooksRequest {
	book_ids: string[];
	updates: {
		language?: string;
		metadata_status?: string;
		rating?: number;
		publisher_id?: string | null;
	};
}

export interface BatchSetTagsRequest {
	book_ids: string[];
	tags: BookTagLink[];
	mode: 'replace' | 'add';
}

export interface BatchUpdateResponse {
	updated_count: number;
	errors: Array<{ book_id: string; error: string }>;
}

// --- Identification types ---

/** Series information included in a candidate response. */
export interface CandidateSeriesInfo {
	name: string;
	position?: number;
}

/** A metadata identification candidate returned by a provider. */
export interface CandidateResponse {
	id: string;
	provider_name: string;
	score: number;
	title?: string;
	authors: string[];
	description?: string;
	publisher?: string;
	publication_date?: string;
	isbn?: string;
	series?: CandidateSeriesInfo;
	cover_url?: string;
	match_reasons: string[];
	status: 'pending' | 'applied' | 'rejected';
}

/** Response from triggering identification for a book. */
export interface IdentifyResponse {
	task_id: string;
}

/** Response from the identify-all endpoint. */
export interface IdentifyAllResponse {
	count: number;
	task_ids: string[];
}

// --- ISBN content scan types ---

/** Response from triggering an ISBN content scan for a book. */
export interface IsbnScanResponse {
	task_id: string;
}

/** Response from batch ISBN content scanning. */
export interface BatchIsbnScanResponse {
	tasks: IsbnScanResponse[];
}

// --- Settings types ---

export type ConfigSource = 'default' | 'file' | 'database' | 'env' | 'cli';
export type SettingScope = 'bootstrap' | 'runtime';
export type SettingType = 'string' | 'optional_string' | 'bool' | 'integer' | 'float' | 'select';

export interface SettingOverride {
	source: 'env' | 'cli';
	env_var?: string;
}

export interface SettingEntry {
	key: string;
	value: unknown;
	effective_value: unknown;
	source: ConfigSource;
	scope: SettingScope;
	override: SettingOverride | null;
	requires_restart: boolean;
	label: string;
	description: string;
	section: string;
	value_type: SettingType;
	sensitive?: boolean;
	is_set?: boolean;
	options?: string[];
}

export interface SettingsResponse {
	settings: SettingEntry[];
}

export interface UpdateSettingsResponse {
	updated: string[];
	requires_restart: boolean;
}

// --- Reader types ---

export interface ReadingProgressResponse {
	id: string;
	book_id: string;
	book_file_id: string;
	location: string | null;
	progress: number;
	device_id: string | null;
	preferences: Record<string, unknown> | null;
	started_at: string;
	updated_at: string;
}

export interface UpdateProgressRequest {
	location?: string | null;
	progress: number;
	device_id?: string | null;
	preferences?: Record<string, unknown> | null;
}

export interface ContinueReadingItem {
	book_id: string;
	book_title: string;
	book_file_id: string;
	file_format: string;
	progress: number;
	location: string | null;
	has_cover: boolean;
	updated_at: string;
}

export interface CreateBookmarkRequest {
	location: string;
	label?: string;
	excerpt?: string;
	position: number;
}

export interface BookmarkResponse {
	id: string;
	location: string;
	label: string | null;
	excerpt: string | null;
	position: number;
	created_at: string;
}

export interface TocItem {
	label: string;
	href: string;
	subitems?: TocItem[];
}

// --- Statistics types ---

export interface FormatStat {
	format: string;
	file_count: number;
	total_size: number;
}

export interface StatusCount {
	status: string;
	count: number;
}

export interface TaskTypeCount {
	task_type: string;
	count: number;
}

export interface LibraryStats {
	books: number;
	files: number;
	total_file_size: number;
	average_files_per_book: number;
	files_by_format: FormatStat[];
	metadata_status: StatusCount[];
}

export interface UsageStats {
	tasks_total: number;
	tasks_last_24h: number;
	tasks_by_status: StatusCount[];
	tasks_by_type: TaskTypeCount[];
	pending_duplicates: number;
	pending_candidates: number;
}

export interface DbFileStats {
	main_db_size: number;
	wal_size: number;
	shm_size: number;
}

export interface DbPageStats {
	page_size: number;
	page_count: number;
	freelist_count: number;
	used_pages: number;
	used_bytes: number;
	free_bytes: number;
}

export interface DbObjectStat {
	name: string;
	object_type: string;
	estimated_bytes: number | null;
	row_count: number | null;
}

export interface DbStats {
	files: DbFileStats;
	pages: DbPageStats;
	table_size_estimates_available: boolean;
	objects: DbObjectStat[];
}

export interface StatsResponse {
	generated_at: string;
	library: LibraryStats;
	usage: UsageStats;
	db: DbStats | null;
}

// --- Watched directory types ---

export type WatchMode = 'native' | 'poll';

export interface FsDetectionResponse {
	/** Detected filesystem type (e.g., "ext4", "NFS", "CIFS", "FUSE", "unknown"). */
	fs_type: string;
	/** Whether native OS events are expected to work: "likely", "unlikely", or "unknown". */
	native_likely_works: 'likely' | 'unlikely' | 'unknown';
	/** User-facing explanation of the detection result. */
	explanation: string;
}

export interface WatchedDirectoryResponse {
	id: string;
	path: string;
	watch_mode: WatchMode;
	poll_interval_secs: number | null;
	effective_poll_interval_secs: number;
	enabled: boolean;
	last_error: string | null;
	detected_fs: FsDetectionResponse | null;
	created_at: string;
	updated_at: string;
}

export interface AddWatchedDirectoryRequest {
	path: string;
	watch_mode?: WatchMode;
	poll_interval_secs?: number | null;
}

export interface UpdateWatchedDirectoryRequest {
	watch_mode?: WatchMode;
	poll_interval_secs?: number | null;
	enabled?: boolean;
}

export interface DetectFsRequest {
	path: string;
}

export interface ScanTriggeredResponse {
	task_id: string;
}
