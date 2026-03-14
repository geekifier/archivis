import { vi } from 'vitest';

export function createMockApi() {
  return {
    auth: {
      status: vi.fn(),
      setup: vi.fn(),
      login: vi.fn(),
      logout: vi.fn(),
      me: vi.fn()
    },
    books: {
      list: vi.fn(),
      get: vi.fn(),
      update: vi.fn(),
      refreshMetadata: vi.fn(),
      lockMetadata: vi.fn(),
      unlockMetadata: vi.fn(),
      protectFields: vi.fn(),
      unprotectFields: vi.fn(),
      setAuthors: vi.fn(),
      setSeries: vi.fn(),
      setTags: vi.fn(),
      delete: vi.fn(),
      uploadCover: vi.fn()
    },
    authors: {
      get: vi.fn(),
      search: vi.fn(),
      create: vi.fn(),
      listBooks: vi.fn()
    },
    tags: {
      search: vi.fn()
    },
    publishers: {
      search: vi.fn(),
      create: vi.fn()
    },
    series: {
      get: vi.fn(),
      search: vi.fn(),
      listBooks: vi.fn()
    },
    import: {
      upload: vi.fn(),
      scan: vi.fn(),
      startImport: vi.fn()
    },
    tasks: {
      list: vi.fn(),
      get: vi.fn()
    },
    resolution: {
      candidates: vi.fn(),
      applyCandidate: vi.fn(),
      rejectCandidate: vi.fn(),
      undoCandidate: vi.fn(),
      rejectCandidates: vi.fn(),
      trustMetadata: vi.fn(),
      untrustMetadata: vi.fn(),
      refreshBatch: vi.fn(),
      refreshAll: vi.fn()
    },
    duplicates: {
      list: vi.fn(),
      get: vi.fn(),
      merge: vi.fn(),
      dismiss: vi.fn(),
      flag: vi.fn(),
      count: vi.fn(),
      forBook: vi.fn()
    },
    isbnScan: {
      scanBook: vi.fn(),
      scanBatch: vi.fn()
    },
    ui: {
      sidebarCounts: vi.fn()
    },
    reader: {
      getProgress: vi.fn(),
      updateProgress: vi.fn(),
      clearProgress: vi.fn(),
      continueReading: vi.fn(),
      listBookmarks: vi.fn(),
      createBookmark: vi.fn(),
      deleteBookmark: vi.fn(),
      fetchFileBlob: vi.fn()
    }
  };
}

export type MockApi = ReturnType<typeof createMockApi>;
