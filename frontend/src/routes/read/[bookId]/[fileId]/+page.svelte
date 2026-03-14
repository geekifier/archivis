<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { api, ApiError } from '$lib/api/index.js';
  import type { BookDetail, ReadingProgressResponse, TocItem } from '$lib/api/types.js';
  import ReaderView from '$lib/components/reader/ReaderView.svelte';
  import ReaderToolbar from '$lib/components/reader/ReaderToolbar.svelte';
  import ReaderTocPanel from '$lib/components/reader/ReaderTocPanel.svelte';
  import ReaderSettingsPanel from '$lib/components/reader/ReaderSettingsPanel.svelte';
  import ReaderProgressBar from '$lib/components/reader/ReaderProgressBar.svelte';
  import ReaderBookmarkPanel from '$lib/components/reader/ReaderBookmarkPanel.svelte';
  import { reader } from '$lib/stores/reader.svelte.js';
  import { blobToBookFile } from '$lib/utils/reader.js';

  const bookId = $derived(page.params.bookId ?? '');
  const fileId = $derived(page.params.fileId ?? '');

  let book = $state<BookDetail | null>(null);
  let bookBlob = $state<Blob | null>(null);
  let savedLocation = $state<string | null>(null);
  let loading = $state(true);
  let readerView = $state<ReturnType<typeof ReaderView> | null>(null);

  // Error state with type differentiation
  type ErrorKind =
    | 'not_found'
    | 'network'
    | 'format_unsupported'
    | 'reader_import'
    | 'book_open'
    | 'generic';
  let errorKind = $state<ErrorKind | null>(null);
  let errorMessage = $state<string | null>(null);

  // Auto-hide timer for toolbar
  const TOOLBAR_HIDE_DELAY = 3000;
  let autoHideTimer: ReturnType<typeof setTimeout> | null = null;

  // Theme-reactive background color
  const themes: Record<string, string> = {
    light: '#ffffff',
    dark: '#1a1a1a',
    sepia: '#f4ecd8'
  };
  const containerBg = $derived(themes[reader.preferences.theme] ?? '#ffffff');

  const READABLE_FORMATS = new Set(['epub', 'pdf', 'mobi', 'azw3', 'fb2', 'cbz']);

  onMount(() => {
    void loadReader();
  });

  // Auto-hide toolbar after 3 seconds when visible
  $effect(() => {
    if (reader.toolbarVisible) {
      resetAutoHideTimer();
    } else {
      clearAutoHideTimer();
    }

    return () => {
      clearAutoHideTimer();
    };
  });

  function resetAutoHideTimer(): void {
    clearAutoHideTimer();
    // Don't auto-hide if a panel is open
    if (reader.tocPanelOpen || reader.settingsPanelOpen || reader.bookmarksPanelOpen) return;
    autoHideTimer = setTimeout(() => {
      reader.hideToolbar();
    }, TOOLBAR_HIDE_DELAY);
  }

  function clearAutoHideTimer(): void {
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
  }

  function handleInteraction(): void {
    if (reader.toolbarVisible) {
      resetAutoHideTimer();
    }
  }

  function setError(kind: ErrorKind, message: string): void {
    errorKind = kind;
    errorMessage = message;
  }

  function clearError(): void {
    errorKind = null;
    errorMessage = null;
  }

  async function loadReader(): Promise<void> {
    loading = true;
    clearError();
    bookBlob = null;
    try {
      // 1. Fetch book metadata
      book = await api.books.get(bookId);

      // 2. Find the file entry to get the format
      const fileEntry = book.files.find((f) => f.id === fileId);
      if (!fileEntry) {
        setError('not_found', 'The requested file was not found for this book.');
        loading = false;
        return;
      }

      const fmt = fileEntry.format;

      // 3. Check if the format is readable
      if (!READABLE_FORMATS.has(fmt)) {
        setError(
          'format_unsupported',
          `The ${fmt.toUpperCase()} format is not supported by the reader.`
        );
        loading = false;
        return;
      }

      // 4. Initialize the reader store
      reader.init(bookId, fileId, book.title, fmt);
      reader.loadPreferences();

      // 5. Try to load saved location from localStorage first (instant)
      savedLocation = reader.loadSavedLocation();

      // 6. Then try server progress (may have newer data)
      let serverProgress: ReadingProgressResponse | null = null;
      try {
        serverProgress = await api.reader.getProgress(bookId);
      } catch {
        // No saved progress, that's fine
      }

      if (serverProgress?.location) {
        savedLocation = serverProgress.location;
      }

      // Merge server preferences if available
      if (serverProgress?.preferences) {
        const serverPrefs = serverProgress.preferences;
        for (const [key, value] of Object.entries(serverPrefs)) {
          if (value !== undefined && value !== null) {
            reader.updatePreference(key as keyof typeof reader.preferences, value as never);
          }
        }
      }

      // 7. Fetch file and wrap as File so foliate-js can inspect .name for format detection
      const blob = await api.reader.fetchFileBlob(bookId, fileId);
      bookBlob = blobToBookFile(blob, fmt);
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        if (err.isNotFound) {
          setError('not_found', 'The book or file could not be found.');
        } else {
          setError('network', err.userMessage);
        }
      } else if (err instanceof TypeError && err.message.includes('fetch')) {
        setError(
          'network',
          'A network error occurred while loading the book. Please check your connection and try again.'
        );
      } else {
        setError('generic', err instanceof Error ? err.message : 'Failed to load reader');
      }
    } finally {
      loading = false;
    }
  }

  function handleReaderError(err: Error): void {
    if (err.message.includes('Failed to load script')) {
      setError(
        'reader_import',
        'The reader engine failed to load. Please refresh the page or try a different browser.'
      );
    } else {
      setError('book_open', `The book could not be opened: ${err.message}`);
    }
  }

  function handleRelocate(detail: Parameters<NonNullable<typeof reader.updateLocation>>[0]): void {
    reader.updateLocation(detail);
  }

  function handleTocLoaded(toc: TocItem[]): void {
    reader.setToc(toc);
  }

  function handleLoad(): void {
    // Book is ready
  }

  function handleTocNavigate(href: string): void {
    readerView?.goTo(href);
  }

  function handleScrub(fraction: number): void {
    readerView?.goToFraction(fraction);
  }

  function handleBookmarkNavigate(location: string): void {
    readerView?.goTo(location);
  }

  function handleKeydown(e: KeyboardEvent): void {
    // Don't capture when typing in an input
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;

    switch (e.key) {
      case 'ArrowRight':
      case 'PageDown':
      case ' ':
        e.preventDefault();
        readerView?.next();
        break;
      case 'ArrowLeft':
      case 'PageUp':
        e.preventDefault();
        readerView?.prev();
        break;
      case 'Escape':
        if (reader.tocPanelOpen) {
          reader.toggleTocPanel();
        } else if (reader.settingsPanelOpen) {
          reader.toggleSettingsPanel();
        } else if (reader.bookmarksPanelOpen) {
          reader.toggleBookmarksPanel();
        } else {
          reader.toggleToolbar();
        }
        break;
      case 'f':
        reader.toggleFullscreen();
        break;
      case 't':
        reader.toggleTocPanel();
        break;
      case 's':
        reader.toggleSettingsPanel();
        break;
      case 'b':
        reader.toggleBookmarksPanel();
        break;
      case '+':
      case '=':
        e.preventDefault();
        reader.updatePreference('fontSize', Math.min(200, reader.preferences.fontSize + 10));
        break;
      case '-':
        e.preventDefault();
        reader.updatePreference('fontSize', Math.max(80, reader.preferences.fontSize - 10));
        break;
    }
  }

  function handleBeforeUnload(): void {
    reader.saveProgressNow();
  }

  function handleFullscreenChange(): void {
    reader.setFullscreen(!!document.fullscreenElement);
    // When entering fullscreen, auto-hide the toolbar after a brief delay
    if (document.fullscreenElement) {
      setTimeout(() => {
        reader.hideToolbar();
      }, 1500);
    }
  }

  // Touch tap zone handling
  function handleViewportClick(e: MouseEvent): void {
    // Only handle direct clicks on the tap zone overlay, not bubbled events
    const target = e.currentTarget as HTMLElement;
    if (!target) return;

    const rect = target.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const width = rect.width;
    const fraction = x / width;

    if (fraction < 0.25) {
      // Left 25%: previous page
      readerView?.prev();
    } else if (fraction > 0.75) {
      // Right 25%: next page
      readerView?.next();
    } else {
      // Center 50%: toggle toolbar
      reader.toggleToolbar();
    }
  }

  onMount(() => {
    document.addEventListener('fullscreenchange', handleFullscreenChange);

    return () => {
      document.removeEventListener('fullscreenchange', handleFullscreenChange);
      clearAutoHideTimer();
      reader.destroy();
    };
  });
</script>

<svelte:window
  onkeydown={handleKeydown}
  onbeforeunload={handleBeforeUnload}
  onpointermove={handleInteraction}
/>

<svelte:head>
  <title>{book?.title ? `${book.title} — Archivis Reader` : 'Archivis Reader'}</title>
</svelte:head>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="relative flex h-screen flex-col" style:background-color={containerBg}>
  <!-- Toolbar -->
  <ReaderToolbar
    {bookId}
    bookTitle={book?.title ?? ''}
    visible={reader.toolbarVisible}
    onToggleToc={() => reader.toggleTocPanel()}
    onToggleBookmarks={() => reader.toggleBookmarksPanel()}
    onToggleSettings={() => reader.toggleSettingsPanel()}
    onToggleFullscreen={() => reader.toggleFullscreen()}
  />

  <!-- TOC Panel -->
  <ReaderTocPanel
    toc={reader.toc}
    currentHref={reader.currentHref}
    open={reader.tocPanelOpen}
    onClose={() => reader.toggleTocPanel()}
    onNavigate={handleTocNavigate}
  />

  <!-- Settings Panel -->
  <ReaderSettingsPanel
    open={reader.settingsPanelOpen}
    onClose={() => reader.toggleSettingsPanel()}
  />

  <!-- Bookmarks Panel -->
  <ReaderBookmarkPanel
    {bookId}
    {fileId}
    currentLocation={reader.location}
    currentProgress={reader.progress}
    open={reader.bookmarksPanelOpen}
    onClose={() => reader.toggleBookmarksPanel()}
    onNavigate={handleBookmarkNavigate}
  />

  <!-- Reader viewport -->
  {#if loading}
    <div class="flex flex-1 flex-col items-center justify-center gap-4">
      <!-- Spinner -->
      <svg
        class="size-10 animate-spin text-primary"
        xmlns="http://www.w3.org/2000/svg"
        fill="none"
        viewBox="0 0 24 24"
      >
        <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"
        ></circle>
        <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
        ></path>
      </svg>
      {#if book?.title}
        <p class="max-w-xs truncate text-center text-sm font-medium text-foreground">
          {book.title}
        </p>
      {/if}
      <p class="text-sm text-muted-foreground">Loading book...</p>
    </div>
  {:else if errorKind}
    <div class="flex flex-1 items-center justify-center p-6">
      <div class="max-w-md space-y-4 text-center">
        <!-- Error icon -->
        {#if errorKind === 'not_found'}
          <svg
            class="mx-auto size-12 text-muted-foreground"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
            <path d="M14 2v4a2 2 0 0 0 2 2h4" />
            <path d="m9.5 12.5 5 5" />
            <path d="m14.5 12.5-5 5" />
          </svg>
        {:else if errorKind === 'format_unsupported'}
          <svg
            class="mx-auto size-12 text-muted-foreground"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="12" cy="12" r="10" />
            <path d="M12 16v.01" />
            <path d="M12 8v4" />
          </svg>
        {:else}
          <svg
            class="mx-auto size-12 text-destructive"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path
              d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"
            />
            <line x1="12" x2="12" y1="9" y2="13" />
            <line x1="12" x2="12.01" y1="17" y2="17" />
          </svg>
        {/if}

        <h2 class="text-lg font-semibold text-foreground">
          {#if errorKind === 'not_found'}
            File Not Found
          {:else if errorKind === 'format_unsupported'}
            Format Not Supported
          {:else if errorKind === 'network'}
            Connection Error
          {:else if errorKind === 'reader_import'}
            Reader Failed to Load
          {:else if errorKind === 'book_open'}
            Unable to Open Book
          {:else}
            Something Went Wrong
          {/if}
        </h2>

        <p class="text-sm text-muted-foreground">
          {errorMessage}
        </p>

        <div class="flex flex-wrap items-center justify-center gap-3 pt-2">
          {#if errorKind === 'network' || errorKind === 'reader_import' || errorKind === 'generic'}
            <button
              onclick={() => loadReader()}
              class="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
            >
              <svg
                class="size-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
                <path d="M3 3v5h5" />
                <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
                <path d="M16 16h5v5" />
              </svg>
              Retry
            </button>
          {/if}

          {#if errorKind === 'format_unsupported' || errorKind === 'book_open'}
            <a
              href="/api/books/{bookId}/files/{fileId}/download"
              class="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
            >
              <svg
                class="size-4"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" x2="12" y1="15" y2="3" />
              </svg>
              Download Instead
            </a>
          {/if}

          <button
            onclick={() => {
              window.close();
              goto(`/books/${bookId}`);
            }}
            class="inline-flex items-center gap-1.5 rounded-md border border-border px-4 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted"
          >
            <svg
              class="size-4"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="m15 18-6-6 6-6" />
            </svg>
            Back to Book
          </button>
        </div>
      </div>
    </div>
  {:else if bookBlob}
    <div class="relative flex-1 overflow-hidden">
      <ReaderView
        bind:this={readerView}
        {bookBlob}
        {savedLocation}
        onRelocate={handleRelocate}
        onTocLoaded={handleTocLoaded}
        onLoad={handleLoad}
        onError={handleReaderError}
      />
      <!-- Touch/click tap zones overlay -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="absolute inset-0 z-10" onclick={handleViewportClick}></div>
    </div>

    <!-- Progress bar (always visible at bottom) -->
    <ReaderProgressBar
      progress={reader.progress}
      currentChapter={reader.currentChapter}
      toolbarVisible={reader.toolbarVisible}
      onScrub={handleScrub}
    />
  {/if}
</div>
