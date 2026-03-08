import type {
  AuthorEntry,
  BookDetail,
  BookSummary,
  CandidateResponse,
  FileEntry,
  IdentifierEntry,
  SeriesEntry,
  TagEntry
} from '$lib/api/types.js';

export function createAuthorEntry(overrides?: Partial<AuthorEntry>): AuthorEntry {
  return {
    id: 'author-1',
    name: 'Test Author',
    sort_name: 'Author, Test',
    role: 'author',
    position: 0,
    ...overrides
  };
}

export function createSeriesEntry(overrides?: Partial<SeriesEntry>): SeriesEntry {
  return {
    id: 'series-1',
    name: 'Test Series',
    description: null,
    position: 1,
    ...overrides
  };
}

export function createTagEntry(overrides?: Partial<TagEntry>): TagEntry {
  return {
    id: 'tag-1',
    name: 'fiction',
    category: null,
    ...overrides
  };
}

export function createFileEntry(overrides?: Partial<FileEntry>): FileEntry {
  return {
    id: 'file-1',
    format: 'epub',
    file_size: 1048576,
    hash: 'abc123',
    added_at: '2024-01-01T00:00:00Z',
    ...overrides
  };
}

export function createIdentifierEntry(overrides?: Partial<IdentifierEntry>): IdentifierEntry {
  return {
    id: 'ident-1',
    identifier_type: 'isbn13',
    value: '9780000000001',
    source: { type: 'embedded' },
    confidence: 1.0,
    ...overrides
  };
}

export function createBookSummary(overrides?: Partial<BookSummary>): BookSummary {
  return {
    id: 'book-1',
    title: 'Test Book',
    subtitle: null,
    sort_title: 'test book',
    description: null,
    language: null,
    publication_date: null,
    added_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
    rating: null,
    page_count: null,
    metadata_status: 'needs_review',
    resolution_state: 'pending',
    resolution_outcome: null,
    metadata_locked: false,
    ingest_quality_score: 0,
    has_cover: false,
    authors: [],
    series: [],
    tags: [],
    files: [],
    ...overrides
  };
}

export function createBookDetail(overrides?: Partial<BookDetail>): BookDetail {
  return {
    id: 'book-1',
    title: 'Test Book',
    subtitle: null,
    sort_title: 'test book',
    description: null,
    language: null,
    publication_date: null,
    publisher_id: null,
    publisher_name: null,
    added_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
    rating: null,
    page_count: null,
    metadata_status: 'needs_review',
    resolution_state: 'pending',
    resolution_outcome: null,
    metadata_locked: false,
    metadata_provenance: {},
    ingest_quality_score: 0,
    has_cover: false,
    authors: [],
    series: [],
    tags: [],
    files: [],
    identifiers: [],
    ...overrides
  };
}

export function createCandidateResponse(overrides?: Partial<CandidateResponse>): CandidateResponse {
  return {
    id: 'candidate-1',
    provider_name: 'Open Library',
    score: 0.85,
    title: 'Candidate Title',
    authors: [{ name: 'Candidate Author', role: 'author' }],
    description: undefined,
    publisher: undefined,
    publication_date: undefined,
    isbn: undefined,
    series: undefined,
    cover_url: undefined,
    match_reasons: ['isbn_match'],
    disputes: [],
    status: 'pending',
    tier: undefined,
    ...overrides
  };
}
