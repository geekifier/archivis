<script lang="ts">
	import '../app.css';
	import favicon from '$lib/assets/favicon.svg';
	import { goto } from '$app/navigation';
	import { theme } from '$lib/theme.svelte.js';
	import { auth } from '$lib/stores/auth.svelte.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { page } from '$app/state';

	let { children } = $props();

	let sidebarOpen = $state(false);

	/** Pages that don't require authentication. */
	const publicPaths = ['/login', '/setup'];
	const isPublicPage = $derived(publicPaths.includes(page.url.pathname));

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
			const redirect = page.url.pathname === '/' ? '' : `?redirect=${encodeURIComponent(page.url.pathname)}`;
			goto(`/login${redirect}`);
		}
	}

	const navItems = [
		{ href: '/', label: 'Library', icon: 'library' },
		{ href: '/import', label: 'Import', icon: 'import' }
	];

	function isActive(href: string): boolean {
		if (href === '/') return page.url.pathname === '/';
		return page.url.pathname.startsWith(href);
	}
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
					<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
					<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
					</svg>
				{/if}
			</Button>
		</div>
		{@render children()}
	</div>
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
			<nav class="flex-1 space-y-1 p-3">
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
							<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20" />
							</svg>
						{:else if item.icon === 'import'}
							<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<path d="M12 3v12" />
								<path d="m8 11 4 4 4-4" />
								<path d="M8 5H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-4" />
							</svg>
						{/if}
						{item.label}
					</a>
				{/each}
			</nav>
		</aside>

		<!-- Main area -->
		<div class="flex flex-1 flex-col overflow-hidden">
			<!-- Header -->
			<header class="flex h-14 items-center justify-between border-b border-border bg-background px-4">
				<div class="flex items-center gap-3">
					<!-- Mobile hamburger -->
					<Button
						variant="ghost"
						size="icon-sm"
						class="lg:hidden"
						onclick={() => (sidebarOpen = !sidebarOpen)}
						aria-label="Toggle sidebar"
					>
						<svg class="size-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
							<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
							<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
								<svg class="size-4" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
