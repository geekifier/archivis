<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import type { BookFormat, MetadataStatus } from '$lib/api/types.js';
  import favicon from '$lib/assets/favicon.svg';
  import { Button } from '$lib/components/ui/button/index.js';
  import { auth } from '$lib/stores/auth.svelte.js';
  import { filters } from '$lib/stores/filters.svelte.js';
  import { navCounts } from '$lib/stores/nav-counts.svelte.js';
  import { theme } from '$lib/theme.svelte.js';
  import ChangePasswordDialog from '$lib/components/settings/ChangePasswordDialog.svelte';
  import '../app.css';

  let { children } = $props();

  let sidebarOpen = $state(false);
  let changePasswordOpen = $state(false);

  /** Pages that don't require authentication. */
  const publicPaths = ['/login', '/setup'];
  const isPublicPage = $derived(publicPaths.includes(page.url.pathname));

  /** Pages that render chromeless (no sidebar/header) — handled by their own layout. */
  const chromelessPaths = ['/read'];
  const isChromeless = $derived(chromelessPaths.some((p) => page.url.pathname.startsWith(p)));

  $effect(() => {
    theme.init();
  });

  // Auth guard: check auth on load and react to route changes
  $effect(() => {
    void runAuthGuard();
  });

  async function runAuthGuard() {
    await auth.checkAuth();

    if (auth.setupRequired) {
      if (page.url.pathname !== '/setup') {
        goto('/setup');
      }
      return;
    }

    if (!auth.isAuthenticated && !isPublicPage) {
      const redirect =
        page.url.pathname === '/' ? '' : `?redirect=${encodeURIComponent(page.url.pathname)}`;
      goto(`/login${redirect}`);
    }
  }

  const navItems = $derived([
    { href: '/', label: 'Library', icon: 'library' },
    { href: '/authors', label: 'Authors', icon: 'authors' },
    { href: '/series', label: 'Series', icon: 'series' },
    { href: '/import', label: 'Import', icon: 'import' },
    { href: '/stats', label: 'Statistics', icon: 'stats' },
    { href: '/duplicates', label: 'Duplicates', icon: 'duplicates' },
    ...(auth.user?.role === 'admin'
      ? [{ href: '/settings', label: 'Settings', icon: 'settings' }]
      : [])
  ]);

  function isActive(href: string): boolean {
    if (href === '/') return page.url.pathname === '/' || page.url.pathname.startsWith('/books');
    if (href === '/authors') return page.url.pathname.startsWith('/authors');
    if (href === '/series') return page.url.pathname.startsWith('/series');
    if (href === '/stats') return page.url.pathname.startsWith('/stats');
    if (href === '/duplicates') return page.url.pathname.startsWith('/duplicates');
    return page.url.pathname.startsWith(href);
  }

  const isLibraryPage = $derived(
    page.url.pathname === '/' || page.url.pathname.startsWith('/books')
  );

  const formats: { value: BookFormat; label: string }[] = [
    { value: 'epub', label: 'EPUB' },
    { value: 'pdf', label: 'PDF' },
    { value: 'mobi', label: 'MOBI' },
    { value: 'cbz', label: 'CBZ' },
    { value: 'fb2', label: 'FB2' },
    { value: 'txt', label: 'TXT' },
    { value: 'djvu', label: 'DJVU' },
    { value: 'azw3', label: 'AZW3' }
  ];

  const statuses: { value: MetadataStatus; label: string; colorClass: string }[] = [
    { value: 'identified', label: 'Identified', colorClass: 'bg-green-500' },
    { value: 'needs_review', label: 'Needs Review', colorClass: 'bg-amber-500' },
    { value: 'unidentified', label: 'Unidentified', colorClass: 'bg-gray-400' }
  ];

  // Refresh sidebar counts on every navigation while authenticated
  $effect(() => {
    void page.url.pathname;
    if (auth.isAuthenticated) {
      navCounts.refresh();
    }
  });
</script>

<svelte:head>
  <link rel="icon" href={favicon} />
  <link rel="icon" type="image/png" href="/favicon.png" />
</svelte:head>

{#if auth.loading}
  <!-- Loading state while checking auth -->
  <div class="flex h-screen items-center justify-center">
    <div class="text-muted-foreground">Loading...</div>
  </div>
{:else if isPublicPage}
  <!-- Minimal layout for login/setup pages -->
  <div class="min-h-screen bg-background">
    <div class="flex items-center justify-end p-4">
      <Button
        variant="ghost"
        size="icon-sm"
        onclick={() => theme.toggle()}
        aria-label="Toggle theme"
      >
        {#if theme.current === 'dark'}
          <svg
            class="size-4"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="12" cy="12" r="4" />
            <path d="M12 2v2" />
            <path d="M12 20v2" />
            <path d="m4.93 4.93 1.41 1.41" />
            <path d="m17.66 17.66 1.41 1.41" />
            <path d="M2 12h2" />
            <path d="M20 12h2" />
            <path d="m6.34 17.66-1.41 1.41" />
            <path d="m19.07 4.93-1.41 1.41" />
          </svg>
        {:else}
          <svg
            class="size-4"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
          </svg>
        {/if}
      </Button>
    </div>
    {@render children()}
  </div>
{:else if isChromeless}
  <!-- Chromeless layout for reader — child layout handles auth and chrome -->
  {@render children()}
{:else}
  <!-- Full app shell for authenticated users -->

  <!-- Mobile sidebar overlay -->
  {#if sidebarOpen}
    <button
      class="fixed inset-0 z-30 bg-black/50 lg:hidden"
      onclick={() => (sidebarOpen = false)}
      aria-label="Close sidebar"
    ></button>
  {/if}

  <div class="flex h-screen overflow-hidden">
    <!-- Sidebar -->
    <aside
      class="fixed inset-y-0 left-0 z-40 flex w-64 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground transition-transform duration-200 lg:static lg:translate-x-0
			{sidebarOpen ? 'translate-x-0' : '-translate-x-full'}"
    >
      <div class="flex h-14 items-center border-b border-sidebar-border px-4">
        <a href="/" class="text-lg font-semibold text-sidebar-primary" title="Archivis Home">
          <svg
            version="1.1"
            xmlns="http://www.w3.org/2000/svg"
            class="h-9 w-auto"
            viewBox="220 260 740 200"
          >
            <path
              d="m 343.17932,398.16222 -24.15744,0.051 -7.65938,22.82973 h 39.26129 z"
              class="fill-current/90"
            />
            <path
              d="m 286.38,453.04 c -3.72,1.42 -56.38,1.26 -56.38,-0.16 0,-0.62 1.83,-6.14 4.07,-12.25 7.1,-19.38 18.03,-48.94 20.01,-54.13 1.06,-2.75 5.54,-14.9 9.97,-27 25.19,-68.84 30.13,-81.88 31.75,-83.75 1.37,-1.59 4.04,-1.75 28.46,-1.75 h 26.96 c -1.04806,5.96953 -1.08009,6.69597 -2.38,9.07 -0.48,0.31 -3.51,8.41 -6.75,18 -3.24,9.58 -10.48,30.7 -16.1,46.93 -5.61,16.23 -10.8,31.3 -11.52,33.5 -7.79,23.81 -22.71,66.22 -23.97,68.14 -0.88,1.35 -2.74,2.88 -4.12,3.4 z"
              id="cover"
              class="cover"
              fill="#1a6eb6"
            />
            <path
              d="m 687.75,452.45 c -0.69,0.21 -6.73,0.62 -13.42,0.91 -10.37,0.45 -12.37,0.3 -13.5,-1.07 -1.03,-1.23 -1.44,-9.58 -1.83,-36.74 -0.56,-39.47 -0.63,-39.83 -7.54,-41.12 -10.03,-1.89 -22.75,4.76 -29.68,15.5 l -2.95,4.57 0.34,29 0.33,29 H 599.51 V 386 c -0.01,-51.28 -0.3,-67 -1.26,-68.68 -0.75,-1.32 -3.46,-2.9 -6.84,-4 -3.08,-1 -5.59,-2.27 -5.56,-2.82 0.03,-0.84 17.36,-4.79 30.29,-6.89 l 3.64,-0.6 -0.54,40.19 -0.55,40.18 8.41,-8.57 c 11,-11.23 16.35,-14.01 28.09,-14.61 7.38,-0.37 9.34,-0.12 13,1.67 4.63,2.26 8.28,6.79 9.84,12.23 0.53,1.87 0.97,17.42 0.97,34.74 0,35.12 0.3,36.9 6.66,39.94 3.47,1.65 4.24,3.01 2.09,3.67 z m 244.24,-0.13 c -7.19,1.97 -15.79,2.54 -24.72,1.63 -7.91,-0.81 -20.79,-4.36 -21.81,-6.02 -0.97,-1.56 2.73,-19.93 4.01,-19.93 0.6,0 1.85,1.28 2.77,2.83 5.09,8.61 17.01,15.17 27.55,15.17 14.24,-0.01 20.58,-10.82 12.03,-20.55 -2.14,-2.44 -7.67,-5.74 -18.2,-10.86 -22.89,-11.13 -28.41,-17.21 -27.36,-30.21 0.88,-10.95 7.74,-18.45 20.51,-22.39 8.36,-2.58 27.57,-2.67 37.23,-0.17 l 6.5,1.68 -0.32,3.5 c -0.18,1.92 -1.02,6.58 -1.86,10.35 l -1.53,6.85 -6.14,-6.53 c -5.27,-5.59 -7,-6.77 -12.12,-8.23 -11.86,-3.4 -23.58,1.43 -23.82,9.8 -0.2,6.91 4.11,10.6 21.23,18.18 5.49,2.44 13.08,6.44 16.87,8.89 8.62,5.58 12.19,11.7 12.19,20.93 0,7.07 -2.25,11.95 -7.87,17.02 -4.91,4.44 -8.04,6.1 -15.14,8.06 z m -142.92,-1.31 c -1.66,1.88 -11.74,3.53 -13.58,2.21 -0.56,-0.39 -6.59,-14.67 -13.41,-31.72 -19,-47.48 -19.95,-49.31 -26.73,-51.55 -1.84,-0.61 -3.35,-1.64 -3.35,-2.29 0,-1.22 20.8,-7.66 24.74,-7.66 1.22,0 2.89,1.03 3.71,2.28 1.35,2.06 14.25,35.27 23.2,59.72 1.91,5.23 3.88,9.91 4.37,10.4 1.58,1.61 12.94,-25.58 17.07,-40.86 1.18,-4.38 2.21,-10.83 2.28,-14.33 0.12,-6.01 -0.06,-6.54 -3.14,-9.29 -1.79,-1.61 -3.93,-2.92 -4.75,-2.92 -2.33,0 -1.73,-1.91 0.77,-2.45 5.23,-1.13 24.78,-2.65 25.82,-2.01 2.93,1.82 -2.06,20.48 -10.57,39.52 -5.63,12.58 -24.24,48.46 -26.43,50.95 z m -230.87,2.88 c -29.47,6.17 -55.74,-18.62 -52.84,-49.86 2.04,-21.9 16.43,-38.33 37.73,-43.04 6.49,-1.44 29.41,-0.84 34.91,0.9 2.81,0.9 4.41,1.37 5.14,2.44 0.94,1.39 0.38,3.81 -0.94,9.51 l -0.12,0.54 c -1.22,5.27 -2.7,9.89 -3.3,10.26 -0.6,0.36 -2.01,-0.64 -3.15,-2.24 -6.26,-8.77 -12.97,-12.36 -23.13,-12.36 -14.76,0 -22.49,7.67 -25.56,25.34 -1.45,8.38 -0.63,17.71 2.26,25.41 2.82,7.54 11.32,16.18 18.23,18.53 8.97,3.05 21.62,1.77 31.28,-3.17 5.18,-2.64 6.29,-2.69 6.29,-0.24 0,4.68 -16.66,15.86 -26.8,17.98 z M 468,424.47 V 453 h -9.33 c -5.14,0 -9.64,-0.3 -10,-0.67 -0.37,-0.36 -0.68,-17.8 -0.68,-38.75 -0.02,-28.57 -0.33,-38.7 -1.28,-40.58 -0.89,-1.77 -2.93,-3.03 -6.98,-4.32 -3.15,-1 -5.73,-2.21 -5.73,-2.68 0,-0.47 2.36,-1.32 5.25,-1.9 14.1,-2.78 22.29,-4.1 25.48,-4.1 h 3.52 l -0.43,12 c -0.24,6.6 -0.18,12 0.13,12 0.3,0 1.79,-2.38 3.31,-5.29 6.42,-12.28 14.57,-18.72 23.69,-18.69 8.45,0.02 8.6,0.24 7.14,10.63 -0.68,4.85 -1.42,9.02 -1.66,9.25 -0.23,0.23 -1.57,-0.1 -2.97,-0.74 -1.41,-0.64 -5.03,-1.16 -8.05,-1.16 -6.61,0 -12.23,2.59 -15.82,7.29 -5.4,7.08 -5.59,8.42 -5.59,39.18 z M 350.48,276.75 351.22,274 h 4.45066 c 1.5393,0 3.24684,1.16009 3.81369,2.59122 5.62725,14.20713 32.99639,83.34004 47.22565,119.90878 17.4,44.73 20.29,52.47 20.29,54.34 0,2.66 -7.07,4.02 -10.16,1.97 -1.47,-0.98 -7.33,-15.03 -19.93,-47.85 -9.82,-25.55 -18.74,-48.71 -19.83,-51.46 -1.08,-2.75 -6.9,-17.83 -12.92,-33.5 -14.11,-36.71 -15.33,-36.1 -15.32,-36.93 0.02,-0.83 1.24,-4.81 1.64,-6.32 z M 875,451.54 c 0,1.21 -2.37,1.46 -14,1.46 -7.7,0 -14.04,-0.34 -14.1,-0.75 -0.05,-0.41 -0.16,-18.14 -0.25,-39.39 0,-1.98 -0.01,-3.87 -0.02,-5.67 -0.09,-27.56 -0.12,-34.04 -3.25,-36.41 -1.17,-0.88 -2.77,-1.19 -4.97,-1.8 -2.46,-0.7 -4.32,-1.76 -4.12,-2.37 0.39,-1.21 24.17,-6.51 29.46,-6.57 L 867,360 v 40.75 c 0.01,38.59 0.11,40.92 1.95,43.94 1.07,1.75 2.87,3.68 4,4.28 1.13,0.61 2.05,1.76 2.05,2.57 z m -143.01,0.21 C 732,452.69 728.44,453 717.5,453 H 703 v -38.87 -4.18 c 0.01,-29.42 0.01,-36.43 -3.37,-39.02 -1.21,-0.93 -2.86,-1.29 -5.09,-1.92 -2.5,-0.7 -4.54,-1.8 -4.54,-2.45 0,-1.13 25.57,-6.59 30.75,-6.57 l 2.25,0.01 0.01,39.25 c 0,24.3 0.4,40.69 1.05,43.03 0.71,2.55 2.16,4.49 4.48,6 1.89,1.22 3.44,2.78 3.45,3.47 z m 130.44,-108.7 c -3.77,2.49 -10.03,2.49 -14.12,0 -3.88,-2.37 -5.31,-4.86 -5.31,-9.26 0,-6.46 5.43,-10.79 13.55,-10.79 3.95,0 5.16,0.51 8.05,3.4 5.28,5.28 4.37,12.35 -2.17,16.65 z m -144.44,0.46 c -9.73,5.03 -20.99,-2.9 -18.05,-12.7 1.47,-4.9 5.89,-7.8 11.88,-7.81 7.67,0 12.12,3.85 12.16,10.53 0.02,4.66 -1.9,7.86 -5.99,9.98 z"
              id="text"
              class="fill-current/90"
            />
            <path
              d="m 346.34,407.72 c 0,0 -6.63,-20.05 -9.35,-28.27 -2.72,-8.22 -4.95,-15.95 -4.97,-17.17 -0.03,-2.69 14.95,-48.28 15.86,-48.28 0.95,0 52.12,136.06 52.12,138.57 0,1.15 -1.85,1.4 -9.25,1.29 -5.09,-0.07 -10.37,-0.32 -11.75,-0.55 -13.3,-2.23 -21.13,-9.77 -26.07,-25.08 L 350.61,421 c 0,0 -2.23,-6.8 -4.27,-13.28 z"
              id="page"
              fill="#d98536"
            />
          </svg>
        </a>
      </div>
      <nav class="flex-1 space-y-1 overflow-y-auto p-3">
        {#each navItems as item (item.href)}
          <a
            href={item.href}
            onclick={() => (sidebarOpen = false)}
            class="flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors
						{isActive(item.href)
              ? 'bg-sidebar-accent text-sidebar-accent-foreground'
              : 'text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground'}"
          >
            {#if item.icon === 'library'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"
                />
              </svg>
            {:else if item.icon === 'authors'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
                <circle cx="9" cy="7" r="4" />
                <path d="M22 21v-2a4 4 0 0 0-3-3.87" />
                <path d="M16 3.13a4 4 0 0 1 0 7.75" />
              </svg>
            {:else if item.icon === 'series'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"
                />
                <path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65" />
                <path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65" />
              </svg>
            {:else if item.icon === 'import'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M12 3v12" />
                <path d="m8 11 4 4 4-4" />
                <path
                  d="M8 5H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-4"
                />
              </svg>
            {:else if item.icon === 'stats'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M3 3v18h18" />
                <path d="m19 9-5 5-4-4-3 3" />
              </svg>
            {:else if item.icon === 'duplicates'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <rect x="8" y="2" width="13" height="13" rx="2" />
                <path d="M5 8H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-1" />
              </svg>
            {:else if item.icon === 'settings'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path
                  d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"
                />
                <circle cx="12" cy="12" r="3" />
              </svg>
            {/if}
            <span class="flex-1">{item.label}</span>
            {#if item.icon === 'duplicates' && navCounts.duplicateCount != null && navCounts.duplicateCount > 0}
              <span
                class="min-w-5 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-center text-xs font-medium text-amber-600 dark:text-amber-400"
              >
                {navCounts.duplicateCount}
              </span>
            {/if}
          </a>
        {/each}

        {#if isLibraryPage}
          <!-- Separator -->
          <div class="my-3 border-t border-sidebar-border"></div>

          <!-- Format filters -->
          <div class="px-3 pb-1">
            <span class="text-xs font-semibold uppercase tracking-wider text-sidebar-foreground/60"
              >Format</span
            >
          </div>
          <div class="flex flex-wrap gap-1 px-2">
            {#each formats as fmt (fmt.value)}
              <button
                onclick={() => filters.setFormat(fmt.value)}
                class="rounded px-2 py-0.5 text-xs font-medium transition-colors
								{filters.activeFormat === fmt.value
                  ? 'bg-primary text-primary-foreground'
                  : 'text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground'}"
              >
                {fmt.label}
              </button>
            {/each}
          </div>

          <!-- Status filters -->
          <div class="mt-3 px-3 pb-1">
            <span class="text-xs font-semibold uppercase tracking-wider text-sidebar-foreground/60"
              >Status</span
            >
          </div>
          {#each statuses as st (st.value)}
            <button
              onclick={() => filters.setStatus(st.value)}
              class="flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors
							{filters.activeStatus === st.value
                ? 'bg-sidebar-accent text-sidebar-accent-foreground font-medium'
                : 'text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground'}"
            >
              <span class="size-2 rounded-full {st.colorClass}"></span>
              <span class="flex-1 text-left">{st.label}</span>
              {#if st.value === 'needs_review' && navCounts.needsReviewCount !== null && navCounts.needsReviewCount > 0}
                <span
                  class="min-w-5 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-center text-xs font-medium text-amber-600 dark:text-amber-400"
                >
                  {navCounts.needsReviewCount}
                </span>
              {/if}
              {#if st.value === 'unidentified' && navCounts.unidentifiedCount !== null && navCounts.unidentifiedCount > 0}
                <span
                  class="min-w-5 rounded-full bg-gray-400/15 px-1.5 py-0.5 text-center text-xs font-medium text-gray-600 dark:text-gray-400"
                >
                  {navCounts.unidentifiedCount}
                </span>
              {/if}
            </button>
          {/each}

          <!-- Clear filters -->
          {#if filters.hasActiveFilters}
            <div class="mt-2 px-2">
              <button
                onclick={() => filters.clearFilters()}
                class="w-full rounded-md px-3 py-1.5 text-xs text-sidebar-foreground/60 transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
              >
                Clear filters
              </button>
            </div>
          {/if}
        {/if}
      </nav>
    </aside>

    <!-- Main area -->
    <div class="flex flex-1 flex-col overflow-hidden">
      <!-- Header -->
      <header
        class="flex h-14 items-center justify-between border-b border-border bg-background px-4"
      >
        <div class="flex items-center gap-3">
          <!-- Mobile hamburger -->
          <Button
            variant="ghost"
            size="icon-sm"
            class="lg:hidden"
            onclick={() => (sidebarOpen = !sidebarOpen)}
            aria-label="Toggle sidebar"
          >
            <svg
              class="size-5"
              xmlns="http://www.w3.org/2000/svg"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <line x1="4" x2="20" y1="12" y2="12" />
              <line x1="4" x2="20" y1="6" y2="6" />
              <line x1="4" x2="20" y1="18" y2="18" />
            </svg>
          </Button>
          <span class="text-lg font-semibold lg:hidden">Archivis</span>
        </div>

        <div class="flex items-center gap-2">
          <!-- Theme toggle -->
          <Button
            variant="ghost"
            size="icon-sm"
            onclick={() => theme.toggle()}
            aria-label="Toggle theme"
          >
            {#if theme.current === 'dark'}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <circle cx="12" cy="12" r="4" />
                <path d="M12 2v2" />
                <path d="M12 20v2" />
                <path d="m4.93 4.93 1.41 1.41" />
                <path d="m17.66 17.66 1.41 1.41" />
                <path d="M2 12h2" />
                <path d="M20 12h2" />
                <path d="m6.34 17.66-1.41 1.41" />
                <path d="m19.07 4.93-1.41 1.41" />
              </svg>
            {:else}
              <svg
                class="size-4"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
              </svg>
            {/if}
          </Button>

          <!-- User menu -->
          {#if auth.user}
            <div class="flex items-center gap-2">
              <span class="hidden text-sm text-muted-foreground sm:inline">
                {auth.user.username}
              </span>
              <Button
                variant="ghost"
                size="icon-sm"
                onclick={() => (changePasswordOpen = true)}
                aria-label="Change password"
                title="Change password"
              >
                <svg
                  class="size-4"
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <circle cx="7.5" cy="15.5" r="5.5" />
                  <path d="m21 2-9.6 9.6" />
                  <path d="m15.5 7.5 3 3L22 7l-3-3" />
                </svg>
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onclick={() => auth.logout()}
                aria-label="Log out"
                title="Log out"
              >
                <svg
                  class="size-4"
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
                  <polyline points="16 17 21 12 16 7" />
                  <line x1="21" x2="9" y1="12" y2="12" />
                </svg>
              </Button>
            </div>
          {/if}
        </div>
      </header>

      <!-- Main content -->
      <main class="flex-1 overflow-auto p-6">
        {@render children()}
      </main>
    </div>
  </div>

  <ChangePasswordDialog bind:open={changePasswordOpen} />
{/if}
