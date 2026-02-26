<script lang="ts">
	import '../app.css';
	import favicon from '$lib/assets/favicon.svg';
	import { goto } from '$app/navigation';
	import { theme } from '$lib/theme.svelte.js';
	import { auth } from '$lib/stores/auth.svelte.js';
	import { filters } from '$lib/stores/filters.svelte.js';
	import { api } from '$lib/api/index.js';
	import type { BookFormat, MetadataStatus } from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { page } from '$app/state';

	let { children } = $props();

	let sidebarOpen = $state(false);

	/** Pages that don't require authentication. */
	const publicPaths = ['/login', '/setup'];
	const isPublicPage = $derived(publicPaths.includes(page.url.pathname));

	/** Pages that render chromeless (no sidebar/header) — handled by their own layout. */
	const chromelessPaths = ['/read'];
	const isChromeless = $derived(
		chromelessPaths.some((p) => page.url.pathname.startsWith(p))
	);

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
		{ href: '/import', label: 'Import', icon: 'import' },
		{ href: '/stats', label: 'Statistics', icon: 'stats' },
		{ href: '/duplicates', label: 'Duplicates', icon: 'duplicates' },
		...(auth.user?.role === 'admin'
			? [{ href: '/settings', label: 'Settings', icon: 'settings' }]
			: [])
	]);

	let duplicateCount = $state<number | null>(null);

	function isActive(href: string): boolean {
		if (href === '/') return page.url.pathname === '/' || page.url.pathname.startsWith('/books');
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

	// Fetch duplicate count when authenticated
	$effect(() => {
		if (auth.isAuthenticated) {
			api.duplicates
				.count()
				.then((result) => {
					duplicateCount = result.count;
				})
				.catch(() => {
					// Silently ignore count fetch errors
				});
		}
	});

	// Fetch needs_review and unidentified counts when authenticated and on library page
	$effect(() => {
		if (auth.isAuthenticated && isLibraryPage) {
			api.books
				.list({ status: 'needs_review', per_page: 1 })
				.then((result) => {
					filters.setNeedsReviewCount(result.total);
				})
				.catch(() => {
					// Silently ignore count fetch errors
				});
			api.books
				.list({ status: 'unidentified', per_page: 1 })
				.then((result) => {
					filters.setUnidentifiedCount(result.total);
				})
				.catch(() => {
					// Silently ignore count fetch errors
				});
		}
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
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
				<a href="/" class="text-lg font-semibold text-sidebar-primary">Archivis</a>
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
						{#if item.icon === 'duplicates' && duplicateCount != null && duplicateCount > 0}
							<span
								class="min-w-5 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-center text-xs font-medium text-amber-600 dark:text-amber-400"
							>
								{duplicateCount}
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
							{#if st.value === 'needs_review' && filters.needsReviewCount !== null && filters.needsReviewCount > 0}
								<span
									class="min-w-5 rounded-full bg-amber-500/15 px-1.5 py-0.5 text-center text-xs font-medium text-amber-600 dark:text-amber-400"
								>
									{filters.needsReviewCount}
								</span>
							{/if}
							{#if st.value === 'unidentified' && filters.unidentifiedCount !== null && filters.unidentifiedCount > 0}
								<span
									class="min-w-5 rounded-full bg-gray-400/15 px-1.5 py-0.5 text-center text-xs font-medium text-gray-600 dark:text-gray-400"
								>
									{filters.unidentifiedCount}
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
								onclick={() => auth.logout()}
								aria-label="Log out"
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
{/if}
