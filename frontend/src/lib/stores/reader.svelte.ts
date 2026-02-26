import { api, getSessionToken } from '$lib/api/index.js';
import type { TocItem } from '$lib/api/types.js';

export interface ReaderPreferences {
	fontSize: number;
	fontFamily: string;
	lineHeight: number;
	theme: 'light' | 'dark' | 'sepia';
	flow: 'paginated' | 'scrolled';
	margins: number;
	maxWidth: number;
	maxColumns: number;
}

export interface ReaderLocation {
	cfi: string | null;
	fraction: number;
	tocItem: { label: string; href: string } | null;
	pageItem: { label: string } | null;
	location: { current: number; total: number } | null;
}

const DEFAULT_PREFERENCES: ReaderPreferences = {
	fontSize: 100,
	fontFamily: 'default',
	lineHeight: 1.4,
	theme: 'light',
	flow: 'paginated',
	margins: 40,
	maxWidth: 800,
	maxColumns: 1
};

const PREFS_STORAGE_KEY = 'archivis-reader-prefs';

function progressStorageKey(bookId: string, fileId: string): string {
	return `archivis-reader-${bookId}-${fileId}`;
}

function createReaderStore() {
	// Book identity
	let bookId = $state<string>('');
	let fileId = $state<string>('');
	let bookTitle = $state<string>('');
	let format = $state<string>('');

	// Location / progress
	let location = $state<string | null>(null);
	let progress = $state<number>(0);
	let currentChapter = $state<string | null>(null);
	let currentHref = $state<string | null>(null);

	// TOC
	let toc = $state<TocItem[]>([]);

	// Preferences
	let preferences = $state<ReaderPreferences>({ ...DEFAULT_PREFERENCES });

	// UI state
	let toolbarVisible = $state(true);
	let tocPanelOpen = $state(false);
	let settingsPanelOpen = $state(false);
	let bookmarksPanelOpen = $state(false);
	let isFullscreen = $state(false);

	// Loading state
	let initialized = $state(false);

	// Internal: debounce timer for server save
	let saveTimer: ReturnType<typeof setTimeout> | null = null;
	let dirty = false;

	const progressPercent = $derived(Math.round(progress * 100));

	/**
	 * Initialize the store for a new reading session.
	 */
	function init(newBookId: string, newFileId: string, title: string, fmt: string): void {
		// Clear any previous state
		if (saveTimer) {
			clearTimeout(saveTimer);
			saveTimer = null;
		}

		bookId = newBookId;
		fileId = newFileId;
		bookTitle = title;
		format = fmt;

		location = null;
		progress = 0;
		currentChapter = null;
		currentHref = null;
		toc = [];

		toolbarVisible = true;
		tocPanelOpen = false;
		settingsPanelOpen = false;
		bookmarksPanelOpen = false;
		isFullscreen = false;
		dirty = false;
		initialized = true;
	}

	/**
	 * Load preferences from localStorage (instant), then optionally merge from server.
	 */
	function loadPreferences(): void {
		if (typeof localStorage === 'undefined') return;

		const stored = localStorage.getItem(PREFS_STORAGE_KEY);
		if (stored) {
			try {
				const parsed = JSON.parse(stored) as Partial<ReaderPreferences>;
				preferences = { ...DEFAULT_PREFERENCES, ...parsed };
			} catch {
				preferences = { ...DEFAULT_PREFERENCES };
			}
		}
	}

	/**
	 * Load saved location from localStorage for instant restore.
	 */
	function loadSavedLocation(): string | null {
		if (typeof localStorage === 'undefined') return null;
		const key = progressStorageKey(bookId, fileId);
		const stored = localStorage.getItem(key);
		if (stored) {
			try {
				const parsed = JSON.parse(stored) as { location: string; progress: number };
				return parsed.location ?? null;
			} catch {
				return null;
			}
		}
		return null;
	}

	/**
	 * Called from ReaderView's relocate callback.
	 * Updates location, progress, currentChapter.
	 * Saves to localStorage immediately; schedules debounced server save.
	 */
	function updateLocation(detail: ReaderLocation): void {
		location = detail.cfi;
		progress = detail.fraction ?? 0;

		if (detail.tocItem) {
			currentChapter = detail.tocItem.label ?? null;
			currentHref = detail.tocItem.href ?? null;
		}

		// Immediate localStorage save
		saveToLocalStorage();

		// Schedule debounced server save
		scheduleSave();
	}

	/**
	 * Update a single preference and persist.
	 */
	function updatePreference<K extends keyof ReaderPreferences>(
		key: K,
		value: ReaderPreferences[K]
	): void {
		preferences = { ...preferences, [key]: value };
		savePreferencesToLocalStorage();
	}

	/**
	 * Immediately save progress to the server.
	 * Used for beforeunload. Uses fetch with keepalive: true.
	 */
	function saveProgressNow(): void {
		if (!bookId || !fileId || !dirty) return;

		if (saveTimer) {
			clearTimeout(saveTimer);
			saveTimer = null;
		}

		const token = getSessionToken();
		const headers: Record<string, string> = {
			'Content-Type': 'application/json',
			Accept: 'application/json'
		};
		if (token) {
			headers['Authorization'] = `Bearer ${token}`;
		}

		const body = JSON.stringify({
			location: location ?? undefined,
			progress,
			preferences: preferences as unknown as Record<string, unknown>
		});

		try {
			fetch(
				`/api/reader/progress/${encodeURIComponent(bookId)}/${encodeURIComponent(fileId)}`,
				{
					method: 'PUT',
					headers,
					body,
					keepalive: true
				}
			);
		} catch {
			// Best-effort on page unload
		}

		dirty = false;
	}

	/**
	 * Clean up timers and save final progress.
	 */
	function destroy(): void {
		if (dirty) {
			saveProgressNow();
		}
		if (saveTimer) {
			clearTimeout(saveTimer);
			saveTimer = null;
		}
		initialized = false;
	}

	// --- UI toggles ---

	function showToolbar(): void {
		toolbarVisible = true;
	}

	function hideToolbar(): void {
		toolbarVisible = false;
	}

	function toggleToolbar(): void {
		toolbarVisible = !toolbarVisible;
	}

	function toggleFullscreen(): void {
		if (typeof document === 'undefined') return;

		if (!document.fullscreenElement) {
			document.documentElement.requestFullscreen().catch(() => {
				// Fullscreen not supported or denied
			});
		} else {
			document.exitFullscreen().catch(() => {
				// Already exited
			});
		}
	}

	function setFullscreen(value: boolean): void {
		isFullscreen = value;
	}

	function toggleTocPanel(): void {
		tocPanelOpen = !tocPanelOpen;
		if (tocPanelOpen) {
			settingsPanelOpen = false;
			bookmarksPanelOpen = false;
		}
	}

	function toggleSettingsPanel(): void {
		settingsPanelOpen = !settingsPanelOpen;
		if (settingsPanelOpen) {
			tocPanelOpen = false;
			bookmarksPanelOpen = false;
		}
	}

	function toggleBookmarksPanel(): void {
		bookmarksPanelOpen = !bookmarksPanelOpen;
		if (bookmarksPanelOpen) {
			tocPanelOpen = false;
			settingsPanelOpen = false;
		}
	}

	function setToc(newToc: TocItem[]): void {
		toc = newToc;
	}

	// --- Internal helpers ---

	function saveToLocalStorage(): void {
		if (typeof localStorage === 'undefined') return;
		const key = progressStorageKey(bookId, fileId);
		const data = { location, progress };
		localStorage.setItem(key, JSON.stringify(data));
	}

	function savePreferencesToLocalStorage(): void {
		if (typeof localStorage === 'undefined') return;
		localStorage.setItem(PREFS_STORAGE_KEY, JSON.stringify(preferences));
	}

	function scheduleSave(): void {
		dirty = true;
		if (saveTimer) {
			clearTimeout(saveTimer);
		}
		saveTimer = setTimeout(() => {
			saveToServer();
		}, 10_000);
	}

	async function saveToServer(): Promise<void> {
		if (!bookId || !fileId) return;
		try {
			await api.reader.updateProgress(bookId, fileId, {
				location: location ?? undefined,
				progress,
				preferences: preferences as unknown as Record<string, unknown>
			});
			dirty = false;
		} catch (err) {
			console.error('Failed to save reading progress:', err);
		}
	}

	return {
		get bookId() {
			return bookId;
		},
		get fileId() {
			return fileId;
		},
		get bookTitle() {
			return bookTitle;
		},
		get format() {
			return format;
		},
		get location() {
			return location;
		},
		get progress() {
			return progress;
		},
		get progressPercent() {
			return progressPercent;
		},
		get currentChapter() {
			return currentChapter;
		},
		get currentHref() {
			return currentHref;
		},
		get toc() {
			return toc;
		},
		get preferences() {
			return preferences;
		},
		get toolbarVisible() {
			return toolbarVisible;
		},
		get tocPanelOpen() {
			return tocPanelOpen;
		},
		get settingsPanelOpen() {
			return settingsPanelOpen;
		},
		get bookmarksPanelOpen() {
			return bookmarksPanelOpen;
		},
		get isFullscreen() {
			return isFullscreen;
		},
		get initialized() {
			return initialized;
		},
		init,
		loadPreferences,
		loadSavedLocation,
		updateLocation,
		updatePreference,
		saveProgressNow,
		destroy,
		showToolbar,
		hideToolbar,
		toggleToolbar,
		toggleFullscreen,
		setFullscreen,
		toggleTocPanel,
		toggleSettingsPanel,
		toggleBookmarksPanel,
		setToc
	};
}

export const reader = createReaderStore();
