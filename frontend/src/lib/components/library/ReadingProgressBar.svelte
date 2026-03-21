<script lang="ts">
  import { api } from '$lib/api/index.js';

  interface Props {
    bookId: string;
    fileId: string;
    progress: number;
    onProgressChange: (newProgress: number) => void;
  }

  let { bookId, fileId, progress, onProgressChange }: Props = $props();

  let toggling = $state(false);
  let hovered = $state(false);

  const isRead = $derived(progress >= 0.95);
  const percent = $derived(Math.round(progress * 100));

  async function handleToggle() {
    if (toggling || !fileId) return;

    const clearing = isRead;
    const newProgress = clearing ? 0.0 : 1.0;
    toggling = true;
    try {
      await api.reader.updateProgress(bookId, fileId, {
        progress: newProgress,
        location: clearing ? null : undefined
      });
      // Clear localStorage so the reader doesn't restore from a stale position
      if (clearing && typeof localStorage !== 'undefined') {
        localStorage.removeItem(`archivis-reader-${bookId}-${fileId}`);
      }
      onProgressChange(newProgress);
    } catch (err) {
      console.error('Failed to update reading progress:', err);
    } finally {
      toggling = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      handleToggle();
    }
  }
</script>

<button
  type="button"
  class="group flex h-6 w-full cursor-pointer items-center gap-2 rounded-md px-1 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
  disabled={toggling}
  onclick={handleToggle}
  onkeydown={handleKeydown}
  onmouseenter={() => (hovered = true)}
  onmouseleave={() => (hovered = false)}
  role="switch"
  aria-checked={isRead}
  aria-label={isRead
    ? 'Reading complete. Click to clear progress'
    : progress > 0
      ? `Reading progress: ${percent}%. Click to mark as read`
      : 'Not started. Click to mark as read'}
  title={hovered ? undefined : isRead ? 'Complete' : progress > 0 ? `${percent}%` : ''}
>
  <!-- Progress bar track -->
  <div
    class="h-1.5 flex-1 overflow-hidden rounded-full {isRead
      ? 'bg-emerald-500/20'
      : 'bg-muted'}"
  >
    {#if progress > 0}
      <div
        class="h-full rounded-full transition-all duration-300 {isRead
          ? 'bg-emerald-500'
          : 'bg-primary'}"
        style="width: {percent}%"
      ></div>
    {/if}
  </div>

  <!-- Status label (right edge): shows status at rest, action on hover -->
  <span class="pointer-events-none w-[5.5rem] shrink-0 text-right text-xs">
    {#if hovered}
      <span class="font-medium text-muted-foreground">
        {#if toggling}
          ...
        {:else if isRead}
          Clear
        {:else}
          Mark as read
        {/if}
      </span>
    {:else}
      <span
        class={isRead
          ? 'font-medium text-emerald-600 dark:text-emerald-400'
          : 'text-muted-foreground'}
      >
        {#if isRead}
          Complete
        {:else if progress > 0}
          {percent}%
        {:else}
          Unread
        {/if}
      </span>
    {/if}
  </span>
</button>
