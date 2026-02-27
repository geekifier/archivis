import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

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
const { api, getSessionToken } = await import('$lib/api/index.js');
const { reader } = await import('./reader.svelte.js');

const mockApi = api as unknown as import('$lib/test-utils/api-mock.js').MockApi;
const mockGetSessionToken = getSessionToken as unknown as ReturnType<typeof vi.fn>;

// localStorage mock
const localStorageMap = new Map<string, string>();
const localStorageMock = {
	getItem: vi.fn((key: string) => localStorageMap.get(key) ?? null),
	setItem: vi.fn((key: string, value: string) => localStorageMap.set(key, value)),
	removeItem: vi.fn((key: string) => localStorageMap.delete(key)),
	clear: vi.fn(() => localStorageMap.clear()),
	get length() {
		return localStorageMap.size;
	},
	key: vi.fn(() => null)
};

describe('reader store', () => {
	beforeEach(() => {
		localStorageMap.clear();
		Object.defineProperty(globalThis, 'localStorage', {
			value: localStorageMock,
			writable: true,
			configurable: true
		});
		localStorageMock.getItem.mockClear();
		localStorageMock.setItem.mockClear();
		mockGetSessionToken.mockReset();
		for (const group of Object.values(mockApi)) {
			for (const fn of Object.values(group as Record<string, ReturnType<typeof vi.fn>>)) {
				fn.mockReset();
			}
		}

		// Initialize with known state for each test
		reader.init('book-1', 'file-1', 'Test Book', 'epub');
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	describe('init', () => {
		it('sets identity fields', () => {
			reader.init('book-abc', 'file-xyz', 'My Book', 'pdf');

			expect(reader.bookId).toBe('book-abc');
			expect(reader.fileId).toBe('file-xyz');
			expect(reader.bookTitle).toBe('My Book');
			expect(reader.format).toBe('pdf');
		});

		it('resets progress and UI state', () => {
			// First set some state
			reader.updateLocation({
				cfi: 'epubcfi(/6/4)',
				fraction: 0.5,
				tocItem: { label: 'Ch 1', href: 'ch1.xhtml' },
				pageItem: null,
				location: { current: 10, total: 20 }
			});

			// Re-init should reset
			reader.init('book-new', 'file-new', 'New Book', 'epub');

			expect(reader.progress).toBe(0);
			expect(reader.location).toBeNull();
			expect(reader.currentChapter).toBeNull();
			expect(reader.initialized).toBe(true);
		});

		it('closes panels on init', () => {
			reader.toggleSettingsPanel();
			expect(reader.settingsPanelOpen).toBe(true);

			reader.init('book-2', 'file-2', 'Book 2', 'epub');

			expect(reader.settingsPanelOpen).toBe(false);
			expect(reader.tocPanelOpen).toBe(false);
			expect(reader.bookmarksPanelOpen).toBe(false);
		});
	});

	describe('preferences', () => {
		it('has expected default values', () => {
			const prefs = reader.preferences;
			expect(prefs.fontSize).toBe(100);
			expect(prefs.fontFamily).toBe('default');
			expect(prefs.lineHeight).toBe(1.4);
			expect(prefs.theme).toBe('light');
			expect(prefs.flow).toBe('paginated');
			expect(prefs.margins).toBe(40);
			expect(prefs.maxWidth).toBe(800);
			expect(prefs.maxColumns).toBe(1);
		});

		it('updatePreference changes one preference, leaves others unchanged', () => {
			const before = { ...reader.preferences };
			reader.updatePreference('fontSize', 120);

			expect(reader.preferences.fontSize).toBe(120);
			expect(reader.preferences.fontFamily).toBe(before.fontFamily);
			expect(reader.preferences.theme).toBe(before.theme);
		});

		it('updatePreference persists to localStorage', () => {
			reader.updatePreference('fontSize', 130);

			expect(localStorageMock.setItem).toHaveBeenCalled();
			const stored = localStorageMap.get('archivis-reader-prefs');
			expect(stored).toBeDefined();
			const parsed = JSON.parse(stored!);
			expect(parsed.fontSize).toBe(130);
		});

		it('loadPreferences restores from localStorage', () => {
			const savedPrefs = { fontSize: 150, theme: 'dark', fontFamily: 'serif' };
			localStorageMap.set('archivis-reader-prefs', JSON.stringify(savedPrefs));

			reader.loadPreferences();

			expect(reader.preferences.fontSize).toBe(150);
			expect(reader.preferences.theme).toBe('dark');
			expect(reader.preferences.fontFamily).toBe('serif');
			// Non-overridden defaults should remain
			expect(reader.preferences.lineHeight).toBe(1.4);
		});

		it('loadPreferences handles corrupt JSON gracefully', () => {
			localStorageMap.set('archivis-reader-prefs', '{invalid json!!!');

			reader.loadPreferences();

			// Should fall back to defaults
			expect(reader.preferences.fontSize).toBe(100);
			expect(reader.preferences.theme).toBe('light');
		});

		it('multiple updatePreference calls accumulate', () => {
			reader.updatePreference('fontSize', 120);
			reader.updatePreference('theme', 'dark');
			reader.updatePreference('fontFamily', 'serif');

			expect(reader.preferences.fontSize).toBe(120);
			expect(reader.preferences.theme).toBe('dark');
			expect(reader.preferences.fontFamily).toBe('serif');
		});
	});

	describe('panel toggles', () => {
		it('toggleSettingsPanel opens settings and closes others', () => {
			reader.toggleTocPanel();
			expect(reader.tocPanelOpen).toBe(true);

			reader.toggleSettingsPanel();
			expect(reader.settingsPanelOpen).toBe(true);
			expect(reader.tocPanelOpen).toBe(false);
			expect(reader.bookmarksPanelOpen).toBe(false);
		});

		it('toggleTocPanel opens toc and closes others', () => {
			reader.toggleSettingsPanel();
			expect(reader.settingsPanelOpen).toBe(true);

			reader.toggleTocPanel();
			expect(reader.tocPanelOpen).toBe(true);
			expect(reader.settingsPanelOpen).toBe(false);
			expect(reader.bookmarksPanelOpen).toBe(false);
		});

		it('toggleBookmarksPanel opens bookmarks and closes others', () => {
			reader.toggleSettingsPanel();
			expect(reader.settingsPanelOpen).toBe(true);

			reader.toggleBookmarksPanel();
			expect(reader.bookmarksPanelOpen).toBe(true);
			expect(reader.settingsPanelOpen).toBe(false);
			expect(reader.tocPanelOpen).toBe(false);
		});

		it('toggling the same panel twice closes it', () => {
			reader.toggleSettingsPanel();
			expect(reader.settingsPanelOpen).toBe(true);

			reader.toggleSettingsPanel();
			expect(reader.settingsPanelOpen).toBe(false);
		});
	});

	describe('updateLocation', () => {
		it('sets progress, chapter, and location', () => {
			reader.updateLocation({
				cfi: 'epubcfi(/6/8)',
				fraction: 0.42,
				tocItem: { label: 'Chapter 3', href: 'ch3.xhtml' },
				pageItem: null,
				location: { current: 5, total: 12 }
			});

			expect(reader.progress).toBe(0.42);
			expect(reader.progressPercent).toBe(42);
			expect(reader.currentChapter).toBe('Chapter 3');
			expect(reader.location).toBe('epubcfi(/6/8)');
		});

		it('saves to localStorage on location update', () => {
			reader.updateLocation({
				cfi: 'epubcfi(/6/2)',
				fraction: 0.1,
				tocItem: null,
				pageItem: null,
				location: null
			});

			const key = 'archivis-reader-book-1-file-1';
			expect(localStorageMock.setItem).toHaveBeenCalled();
			const stored = localStorageMap.get(key);
			expect(stored).toBeDefined();
			const parsed = JSON.parse(stored!);
			expect(parsed.location).toBe('epubcfi(/6/2)');
			expect(parsed.progress).toBe(0.1);
		});
	});

	describe('loadSavedLocation', () => {
		it('returns stored location', () => {
			const key = 'archivis-reader-book-1-file-1';
			localStorageMap.set(key, JSON.stringify({ location: 'epubcfi(/6/10)', progress: 0.7 }));

			const loc = reader.loadSavedLocation();

			expect(loc).toBe('epubcfi(/6/10)');
		});

		it('returns null when no stored data', () => {
			const loc = reader.loadSavedLocation();
			expect(loc).toBeNull();
		});

		it('returns null for corrupt stored data', () => {
			const key = 'archivis-reader-book-1-file-1';
			localStorageMap.set(key, 'not json');

			const loc = reader.loadSavedLocation();
			expect(loc).toBeNull();
		});
	});
});
