<script lang="ts">
  import {
    createTable,
    getCoreRowModel,
    FlexRender,
    type ColumnDef,
    type Header
  } from '@tanstack/svelte-table';
  import type { BookSummary, SortField, SortOrder } from '$lib/api/index.js';
  import {
    columnToSortField,
    statusConfig,
    formatDate,
    formatAuthors,
    formatSeries,
    formatFormats
  } from './book-list-utils.js';
  import CoverImage from './CoverImage.svelte';
  import { placeholderHue } from '$lib/utils.js';

  interface Props {
    books: BookSummary[];
    sortBy: SortField;
    sortOrder: SortOrder;
    onSort: (field: SortField, order: SortOrder) => void;
    selectionMode?: boolean;
    selectedIds?: Set<string>;
    onselect?: (bookId: string, event: MouseEvent) => void;
  }

  let {
    books,
    sortBy,
    sortOrder,
    onSort,
    selectionMode = false,
    selectedIds = new Set(),
    onselect
  }: Props = $props();

  function handleHeaderClick(headerId: string) {
    const field = columnToSortField[headerId];
    if (!field) return;
    if (sortBy === field) {
      onSort(field, sortOrder === 'asc' ? 'desc' : 'asc');
    } else {
      onSort(field, 'asc');
    }
  }

  function sortIndicator(headerId: string): string {
    const field = columnToSortField[headerId];
    if (!field || sortBy !== field) return '';
    return sortOrder === 'asc' ? ' \u2191' : ' \u2193';
  }

  const columns: ColumnDef<BookSummary>[] = [
    {
      id: 'cover',
      header: '',
      size: 50,
      minSize: 40,
      maxSize: 60,
      enableResizing: false,
      cell: (info) => info.row.original
    },
    {
      id: 'title',
      accessorKey: 'title',
      header: 'Title',
      size: 280,
      minSize: 120,
      cell: (info) => info.row.original
    },
    {
      id: 'authors',
      header: 'Author',
      size: 200,
      minSize: 100,
      cell: (info) => formatAuthors(info.row.original)
    },
    {
      id: 'series',
      header: 'Series',
      size: 180,
      minSize: 80,
      cell: (info) => formatSeries(info.row.original)
    },
    {
      id: 'formats',
      header: 'Format',
      size: 120,
      minSize: 60,
      cell: (info) => info.row.original
    },
    {
      id: 'added_at',
      accessorKey: 'added_at',
      header: 'Date Added',
      size: 140,
      minSize: 100,
      cell: (info) => formatDate(info.getValue() as string)
    },
    {
      id: 'metadata_status',
      accessorKey: 'metadata_status',
      header: 'Status',
      size: 120,
      minSize: 80,
      cell: (info) => info.row.original
    }
  ];

  const table = createTable({
    get data() {
      return books;
    },
    columns,
    getCoreRowModel: getCoreRowModel(),
    columnResizeMode: 'onChange',
    enableColumnResizing: true
  });

  function getHeaderStyle(header: Header<BookSummary, unknown>): string {
    return `width: ${header.getSize()}px;`;
  }

  function getCellStyle(size: number): string {
    return `width: ${size}px;`;
  }

  function handleRowClick(bookId: string, event: MouseEvent) {
    if (selectionMode && onselect) {
      onselect(bookId, event);
    }
  }
</script>

<div class="overflow-x-auto rounded-lg border border-border">
  <table
    class="w-full text-sm"
    style="table-layout: fixed; width: {table.getCenterTotalSize() +
      (selectionMode ? 40 : 0)}px; min-width: 100%;"
  >
    <thead>
      {#each table.getHeaderGroups() as headerGroup (headerGroup.id)}
        <tr class="border-b border-border bg-muted/50">
          {#if selectionMode}
            <th class="w-10 px-2 py-2 text-center" style="width: 40px;">
              <!-- Header for checkbox column -->
            </th>
          {/if}
          {#each headerGroup.headers as header (header.id)}
            {@const isSortable = header.id in columnToSortField}
            <th
              class="relative px-3 py-2 text-left font-medium text-muted-foreground {isSortable
                ? 'cursor-pointer select-none hover:text-foreground'
                : ''}"
              style={getHeaderStyle(header)}
              onclick={() => isSortable && handleHeaderClick(header.id)}
            >
              <div class="flex items-center truncate">
                <FlexRender
                  content={header.column.columnDef.header}
                  context={header.getContext()}
                />
                {#if isSortable}
                  <span class="ml-1 text-xs">{sortIndicator(header.id)}</span>
                {/if}
              </div>
              {#if header.column.getCanResize()}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="absolute right-0 top-0 h-full w-1 cursor-col-resize select-none touch-none hover:bg-primary/50 {header.column.getIsResizing()
                    ? 'bg-primary'
                    : ''}"
                  onmousedown={header.getResizeHandler()}
                  ontouchstart={header.getResizeHandler()}
                ></div>
              {/if}
            </th>
          {/each}
        </tr>
      {/each}
    </thead>
    <tbody>
      {#each table.getRowModel().rows as row (row.id)}
        {@const isSelected = selectedIds.has(row.original.id)}
        <tr
          class="border-b border-border transition-colors {selectionMode
            ? 'cursor-pointer'
            : ''} {isSelected ? 'bg-primary/10' : 'hover:bg-muted/30'}"
          onclick={(e) => handleRowClick(row.original.id, e)}
        >
          {#if selectionMode}
            <td class="px-2 py-1.5 text-center" style="width: 40px;">
              <div
                class="mx-auto flex size-4 items-center justify-center rounded border transition-colors {isSelected
                  ? 'border-primary bg-primary'
                  : 'border-muted-foreground/50'}"
              >
                {#if isSelected}
                  <svg
                    class="size-3 text-primary-foreground"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="3"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                {/if}
              </div>
            </td>
          {/if}
          {#each row.getVisibleCells() as cell (cell.id)}
            <td class="overflow-hidden px-3 py-1.5" style={getCellStyle(cell.column.getSize())}>
              {#if cell.column.id === 'cover'}
                {@const book = cell.row.original}
                {#if selectionMode}
                  <div class="block h-10 w-7 flex-shrink-0 overflow-hidden rounded">
                    {#if book.has_cover}
                      <CoverImage src="/api/books/{book.id}/cover?size=sm&t={Date.parse(book.updated_at)}" alt="" loading="lazy" />
                    {:else}
                      <div
                        class="flex h-full w-full items-center justify-center text-[6px] text-white/70"
                        style="background-color: hsl({placeholderHue(book.id)}, 30%, 25%);"
                      >
                        <svg
                          xmlns="http://www.w3.org/2000/svg"
                          viewBox="0 0 24 24"
                          fill="currentColor"
                          class="size-3"
                        >
                          <path
                            d="M12 6.042A8.967 8.967 0 0 0 6 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 0 1 6 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 0 1 6-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0 0 18 18a8.967 8.967 0 0 0-6 2.292m0-14.25v14.25"
                          />
                        </svg>
                      </div>
                    {/if}
                  </div>
                {:else}
                  <a
                    href="/books/{book.id}"
                    class="block h-10 w-7 flex-shrink-0 overflow-hidden rounded"
                  >
                    {#if book.has_cover}
                      <CoverImage src="/api/books/{book.id}/cover?size=sm&t={Date.parse(book.updated_at)}" alt="" loading="lazy" />
                    {:else}
                      <div
                        class="flex h-full w-full items-center justify-center text-[6px] text-white/70"
                        style="background-color: hsl({placeholderHue(book.id)}, 30%, 25%);"
                      >
                        <svg
                          xmlns="http://www.w3.org/2000/svg"
                          viewBox="0 0 24 24"
                          fill="currentColor"
                          class="size-3"
                        >
                          <path
                            d="M12 6.042A8.967 8.967 0 0 0 6 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 0 1 6 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 0 1 6-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0 0 18 18a8.967 8.967 0 0 0-6 2.292m0-14.25v14.25"
                          />
                        </svg>
                      </div>
                    {/if}
                  </a>
                {/if}
              {:else if cell.column.id === 'title'}
                {@const book = cell.row.original}
                {#if selectionMode}
                  <span class="block truncate font-medium text-foreground">
                    {book.title}
                  </span>
                {:else}
                  <a
                    href="/books/{book.id}"
                    class="block truncate font-medium text-foreground hover:text-primary hover:underline"
                  >
                    {book.title}
                  </a>
                {/if}
              {:else if cell.column.id === 'formats'}
                {@const formats = formatFormats(cell.row.original)}
                <div class="flex flex-wrap gap-1">
                  {#each formats as fmt (fmt)}
                    <span
                      class="inline-flex rounded bg-secondary px-1.5 py-0.5 text-[10px] font-semibold uppercase text-secondary-foreground"
                    >
                      {fmt}
                    </span>
                  {/each}
                </div>
              {:else if cell.column.id === 'metadata_status'}
                {@const status = cell.row.original.metadata_status}
                {@const cfg = statusConfig[status]}
                {#if cfg}
                  <span
                    class="inline-flex rounded-full px-2 py-0.5 text-[10px] font-semibold {cfg.class}"
                  >
                    {cfg.label}
                  </span>
                {/if}
              {:else if cell.column.id === 'authors'}
                {@const authors = cell.row.original.authors ?? []}
                <span class="block truncate text-muted-foreground">
                  {#each authors as author, i (author.id)}
                    {#if i > 0},
                    {/if}
                    {#if selectionMode}
                      {author.name}
                    {:else}
                      <a href="/authors/{author.id}" class="hover:text-primary hover:underline"
                        >{author.name}</a
                      >
                    {/if}
                  {/each}
                </span>
              {:else if cell.column.id === 'series'}
                {@const series = cell.row.original.series ?? []}
                {#if series.length > 0}
                  {@const s = series[0]}
                  <span class="block truncate text-muted-foreground">
                    {#if selectionMode}
                      {s.name}{#if s.position != null}
                        #{s.position}{/if}
                    {:else}
                      <a href="/series/{s.id}" class="hover:text-primary hover:underline"
                        >{s.name}</a
                      >{#if s.position != null}&nbsp;<span class="text-muted-foreground/70"
                          >#{s.position}</span
                        >{/if}
                    {/if}
                  </span>
                {/if}
              {:else}
                <span class="block truncate text-muted-foreground">
                  <FlexRender content={cell.column.columnDef.cell} context={cell.getContext()} />
                </span>
              {/if}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>
