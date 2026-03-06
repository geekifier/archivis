<script lang="ts">
  import { api, type StatsResponse } from '$lib/api/index.js';
  import { taskTypeLabel } from '$lib/components/tasks/task-utils.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatFileSize } from '$lib/utils.js';

  let loading = $state(true);
  let error = $state<string | null>(null);
  let stats = $state<StatsResponse | null>(null);

  const generatedAt = $derived(
    stats?.generated_at ? new Date(stats.generated_at).toLocaleString() : null
  );

  async function fetchStats() {
    loading = true;
    error = null;

    try {
      stats = await api.stats.get();
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to load statistics';
    } finally {
      loading = false;
    }
  }

  function titleCase(input: string): string {
    return input
      .split('_')
      .map((chunk) =>
        chunk.length > 0 ? chunk[0].toUpperCase() + chunk.slice(1).toLowerCase() : chunk
      )
      .join(' ');
  }

  $effect(() => {
    void fetchStats();
  });
</script>

<svelte:head>
  <title>Statistics - Archivis</title>
</svelte:head>

<div class="mx-auto max-w-6xl space-y-6">
  <div class="flex flex-wrap items-start justify-between gap-3">
    <div>
      <h1 class="text-3xl font-bold tracking-tight">Statistics</h1>
      <p class="text-muted-foreground">Library and usage insights for this Archivis instance.</p>
    </div>
    {#if generatedAt}
      <div class="text-sm text-muted-foreground">Updated {generatedAt}</div>
    {/if}
  </div>

  {#if loading}
    <div class="flex items-center justify-center rounded-lg border border-border py-14">
      <span class="text-muted-foreground">Loading statistics...</span>
    </div>
  {:else if error}
    <div
      class="flex flex-col items-center justify-center gap-3 rounded-lg border border-destructive/40 bg-destructive/5 py-14"
    >
      <p class="text-sm text-destructive">{error}</p>
      <Button variant="outline" size="sm" onclick={fetchStats}>Retry</Button>
    </div>
  {:else if stats}
    <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-xs uppercase tracking-wider text-muted-foreground">Books</p>
        <p class="mt-2 text-2xl font-semibold">{stats.library.books.toLocaleString()}</p>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-xs uppercase tracking-wider text-muted-foreground">Files</p>
        <p class="mt-2 text-2xl font-semibold">{stats.library.files.toLocaleString()}</p>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-xs uppercase tracking-wider text-muted-foreground">Library Size</p>
        <p class="mt-2 text-2xl font-semibold">{formatFileSize(stats.library.total_file_size)}</p>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-xs uppercase tracking-wider text-muted-foreground">Avg Files / Book</p>
        <p class="mt-2 text-2xl font-semibold">{stats.library.average_files_per_book.toFixed(2)}</p>
      </div>
    </div>

    <div class="grid gap-4 lg:grid-cols-2">
      <section class="rounded-lg border border-border bg-card">
        <div class="border-b border-border px-4 py-3">
          <h2 class="text-base font-semibold">Files by Format</h2>
        </div>
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-border text-left text-muted-foreground">
                <th class="px-4 py-2 font-medium">Format</th>
                <th class="px-4 py-2 font-medium">Files</th>
                <th class="px-4 py-2 font-medium">Size</th>
              </tr>
            </thead>
            <tbody>
              {#if stats.library.files_by_format.length === 0}
                <tr>
                  <td class="px-4 py-3 text-muted-foreground" colspan="3">No files imported yet.</td
                  >
                </tr>
              {:else}
                {#each stats.library.files_by_format as row (row.format)}
                  <tr class="border-b border-border/60 last:border-0">
                    <td class="px-4 py-2 font-medium uppercase">{row.format}</td>
                    <td class="px-4 py-2">{row.file_count.toLocaleString()}</td>
                    <td class="px-4 py-2">{formatFileSize(row.total_size)}</td>
                  </tr>
                {/each}
              {/if}
            </tbody>
          </table>
        </div>
      </section>

      <section class="rounded-lg border border-border bg-card">
        <div class="border-b border-border px-4 py-3">
          <h2 class="text-base font-semibold">Metadata Status</h2>
        </div>
        <div class="space-y-2 p-4">
          {#if stats.library.metadata_status.length === 0}
            <p class="text-sm text-muted-foreground">No metadata statuses available yet.</p>
          {:else}
            {#each stats.library.metadata_status as row (row.status)}
              <div
                class="flex items-center justify-between rounded-md border border-border/70 px-3 py-2"
              >
                <span class="text-sm">{titleCase(row.status)}</span>
                <span class="text-sm font-semibold">{row.count.toLocaleString()}</span>
              </div>
            {/each}
          {/if}
        </div>
      </section>
    </div>

    <section class="rounded-lg border border-border bg-card">
      <div class="border-b border-border px-4 py-3">
        <h2 class="text-base font-semibold">Task and Queue Usage</h2>
      </div>
      <div class="grid gap-4 p-4 lg:grid-cols-3">
        <div class="rounded-md border border-border/70 p-3">
          <p class="text-xs uppercase tracking-wider text-muted-foreground">Tasks Total</p>
          <p class="mt-1 text-xl font-semibold">{stats.usage.tasks_total.toLocaleString()}</p>
        </div>
        <div class="rounded-md border border-border/70 p-3">
          <p class="text-xs uppercase tracking-wider text-muted-foreground">Tasks (24h)</p>
          <p class="mt-1 text-xl font-semibold">{stats.usage.tasks_last_24h.toLocaleString()}</p>
        </div>
        <div class="rounded-md border border-border/70 p-3">
          <p class="text-xs uppercase tracking-wider text-muted-foreground">Pending Duplicates</p>
          <p class="mt-1 text-xl font-semibold">
            {stats.usage.pending_duplicates.toLocaleString()}
          </p>
        </div>
        <div class="rounded-md border border-border/70 p-3 lg:col-span-1">
          <p class="text-xs uppercase tracking-wider text-muted-foreground">Review Candidates</p>
          <p class="mt-1 text-xl font-semibold">
            {stats.usage.pending_candidates.toLocaleString()}
          </p>
        </div>
        <div class="rounded-md border border-border/70 p-3 lg:col-span-1">
          <p class="mb-2 text-xs uppercase tracking-wider text-muted-foreground">Tasks by Status</p>
          {#if stats.usage.tasks_by_status.length === 0}
            <p class="text-sm text-muted-foreground">No tasks yet.</p>
          {:else}
            <div class="space-y-1">
              {#each stats.usage.tasks_by_status as row (row.status)}
                <div class="flex items-center justify-between text-sm">
                  <span>{titleCase(row.status)}</span>
                  <span class="font-medium">{row.count.toLocaleString()}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
        <div class="rounded-md border border-border/70 p-3 lg:col-span-1">
          <p class="mb-2 text-xs uppercase tracking-wider text-muted-foreground">Tasks by Type</p>
          {#if stats.usage.tasks_by_type.length === 0}
            <p class="text-sm text-muted-foreground">No tasks yet.</p>
          {:else}
            <div class="space-y-1">
              {#each stats.usage.tasks_by_type as row (row.task_type)}
                <div class="flex items-center justify-between text-sm">
                  <span>{taskTypeLabel(row.task_type)}</span>
                  <span class="font-medium">{row.count.toLocaleString()}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    </section>

    {#if stats.db}
      <section class="rounded-lg border border-border bg-card">
        <div class="border-b border-border px-4 py-3">
          <h2 class="text-base font-semibold">Database Diagnostics (Admin)</h2>
        </div>
        <div class="space-y-4 p-4">
          <div class="grid gap-3 sm:grid-cols-3">
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">Main DB</p>
              <p class="mt-1 font-semibold">{formatFileSize(stats.db.files.main_db_size)}</p>
            </div>
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">WAL</p>
              <p class="mt-1 font-semibold">{formatFileSize(stats.db.files.wal_size)}</p>
            </div>
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">SHM</p>
              <p class="mt-1 font-semibold">{formatFileSize(stats.db.files.shm_size)}</p>
            </div>
          </div>

          <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">Page Size</p>
              <p class="mt-1 font-semibold">{stats.db.pages.page_size.toLocaleString()} B</p>
            </div>
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">Pages</p>
              <p class="mt-1 font-semibold">{stats.db.pages.page_count.toLocaleString()}</p>
            </div>
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">Used Bytes</p>
              <p class="mt-1 font-semibold">{formatFileSize(stats.db.pages.used_bytes)}</p>
            </div>
            <div class="rounded-md border border-border/70 p-3">
              <p class="text-xs uppercase tracking-wider text-muted-foreground">Free Bytes</p>
              <p class="mt-1 font-semibold">{formatFileSize(stats.db.pages.free_bytes)}</p>
            </div>
          </div>

          {#if !stats.db.table_size_estimates_available}
            <p class="text-sm text-muted-foreground">
              Table/index size estimates are unavailable on this SQLite build.
            </p>
          {/if}

          <div class="overflow-x-auto rounded-md border border-border/70">
            <table class="w-full text-sm">
              <thead>
                <tr class="border-b border-border bg-muted/40 text-left text-muted-foreground">
                  <th class="px-3 py-2 font-medium">Object</th>
                  <th class="px-3 py-2 font-medium">Type</th>
                  <th class="px-3 py-2 font-medium">Rows</th>
                  <th class="px-3 py-2 font-medium">Estimated Size</th>
                </tr>
              </thead>
              <tbody>
                {#if stats.db.objects.length === 0}
                  <tr>
                    <td class="px-3 py-2 text-muted-foreground" colspan="4">No objects found.</td>
                  </tr>
                {:else}
                  {#each stats.db.objects as row (row.object_type + ':' + row.name)}
                    <tr class="border-b border-border/60 last:border-0">
                      <td class="px-3 py-2 font-medium">{row.name}</td>
                      <td class="px-3 py-2 uppercase">{row.object_type}</td>
                      <td class="px-3 py-2"
                        >{row.row_count == null ? '—' : row.row_count.toLocaleString()}</td
                      >
                      <td class="px-3 py-2">
                        {row.estimated_bytes == null ? '—' : formatFileSize(row.estimated_bytes)}
                      </td>
                    </tr>
                  {/each}
                {/if}
              </tbody>
            </table>
          </div>
        </div>
      </section>
    {/if}
  {/if}
</div>
