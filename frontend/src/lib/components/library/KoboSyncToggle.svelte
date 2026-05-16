<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type { BookFormat, FileEntry, KoboSyncStateResponse } from '$lib/api/types.js';
  import { Switch } from '$lib/components/ui/switch/index.js';

  let {
    bookId,
    files,
    initial,
    onchange
  }: {
    bookId: string;
    files: FileEntry[];
    initial?: KoboSyncStateResponse;
    onchange?: (state: KoboSyncStateResponse | null) => void;
  } = $props();

  const epubFiles = $derived(files.filter((f) => (f.format as BookFormat) === 'epub'));
  const hasEpub = $derived(epubFiles.length > 0);

  // Derived so parent updates after `onchange` flow back in.
  const syncState = $derived(initial ?? null);
  const enabled = $derived(syncState?.enabled === true);

  let busy = $state(false);
  let error: string | null = $state(null);

  async function toggle(next: boolean) {
    if (!hasEpub) return;
    busy = true;
    error = null;
    try {
      let updated: KoboSyncStateResponse;
      if (next) {
        updated = await api.kobo.setBookSync(bookId, { enabled: true });
      } else {
        await api.kobo.deleteBookSync(bookId);
        updated = {
          enabled: false,
          selected_book_file_id: null,
          eligible_file_ids: epubFiles.map((f) => f.id),
          stale: false,
          reason: null
        };
      }
      onchange?.(updated);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="rounded-md border border-border p-3">
  <div class="flex items-center justify-between gap-3">
    <div>
      <div class="text-sm font-medium">Sync to Kobo</div>
      <div class="text-xs text-muted-foreground">
        {#if !hasEpub}
          Requires an EPUB file.
        {:else if syncState?.stale}
          {syncState.reason ?? 'Selected file is no longer available.'}
        {:else if enabled}
          Delivers a KEPUB to all of your paired Kobo devices.
        {:else}
          Off — this book will not appear on your Kobo devices.
        {/if}
      </div>
    </div>
    <Switch
      checked={enabled}
      disabled={!hasEpub || busy}
      onCheckedChange={(v) => toggle(v)}
    />
  </div>
  {#if error}
    <p class="mt-2 text-xs text-destructive">{error}</p>
  {/if}
</div>
