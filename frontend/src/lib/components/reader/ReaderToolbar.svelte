<script lang="ts">
  import { goto } from '$app/navigation';
  import { reader } from '$lib/stores/reader.svelte.js';

  interface Props {
    bookId: string;
    bookTitle: string;
    visible: boolean;
    onToggleToc: () => void;
    onToggleBookmarks: () => void;
    onToggleSettings: () => void;
    onToggleFullscreen: () => void;
  }

  let {
    bookId,
    bookTitle,
    visible,
    onToggleToc,
    onToggleBookmarks,
    onToggleSettings,
    onToggleFullscreen
  }: Props = $props();
</script>

{#if visible}
  <div
    class="absolute inset-x-0 top-0 z-30 flex items-center gap-3 border-b border-border bg-background/90 px-4 py-2 backdrop-blur-sm transition-transform duration-300"
  >
    <!-- Left section: back button + title -->
    <button
      onclick={() => {
        window.close();
        goto(`/books/${bookId}`);
      }}
      class="flex size-11 shrink-0 items-center justify-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground sm:size-auto sm:justify-start"
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
      <span class="hidden sm:inline">Back</span>
    </button>
    <span class="min-w-0 truncate text-sm font-medium">{bookTitle}</span>

    <!-- Center section: current chapter (hidden on mobile) -->
    {#if reader.currentChapter}
      <span
        class="hidden min-w-0 flex-1 truncate text-center text-xs text-muted-foreground md:block"
      >
        {reader.currentChapter}
      </span>
    {:else}
      <span class="hidden flex-1 md:block"></span>
    {/if}

    <!-- Right section: action buttons -->
    <div class="ml-auto flex shrink-0 items-center gap-0.5 sm:gap-1">
      <!-- TOC button -->
      <button
        onclick={onToggleToc}
        class="inline-flex size-11 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground sm:size-9"
        aria-label="Table of contents"
        title="Table of contents (t)"
      >
        <svg
          class="size-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="3" y1="6" x2="21" y2="6" />
          <line x1="3" y1="12" x2="15" y2="12" />
          <line x1="3" y1="18" x2="18" y2="18" />
        </svg>
      </button>

      <!-- Bookmarks button -->
      <button
        onclick={onToggleBookmarks}
        class="inline-flex size-11 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground sm:size-9"
        aria-label="Bookmarks"
        title="Bookmarks (b)"
      >
        <svg
          class="size-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z" />
        </svg>
      </button>

      <!-- Settings button -->
      <button
        onclick={onToggleSettings}
        class="inline-flex size-11 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground sm:size-9"
        aria-label="Reader settings"
        title="Settings (s)"
      >
        <svg
          class="size-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="3" />
          <path
            d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"
          />
        </svg>
      </button>

      <!-- Fullscreen button -->
      <button
        onclick={onToggleFullscreen}
        class="inline-flex size-11 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground sm:size-9"
        aria-label="Toggle fullscreen"
        title="Fullscreen (f)"
      >
        {#if reader.isFullscreen}
          <svg
            class="size-5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M8 3v3a2 2 0 0 1-2 2H3" />
            <path d="M21 8h-3a2 2 0 0 1-2-2V3" />
            <path d="M3 16h3a2 2 0 0 1 2 2v3" />
            <path d="M16 21v-3a2 2 0 0 1 2-2h3" />
          </svg>
        {:else}
          <svg
            class="size-5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M8 3H5a2 2 0 0 0-2 2v3" />
            <path d="M21 8V5a2 2 0 0 0-2-2h-3" />
            <path d="M3 16v3a2 2 0 0 0 2 2h3" />
            <path d="M16 21h3a2 2 0 0 0 2-2v-3" />
          </svg>
        {/if}
      </button>
    </div>
  </div>
{/if}
