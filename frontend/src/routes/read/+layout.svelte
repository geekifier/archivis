<script lang="ts">
	import { auth } from '$lib/stores/auth.svelte.js';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { theme } from '$lib/theme.svelte.js';

	let { children } = $props();

	$effect(() => {
		theme.init();
	});

	$effect(() => {
		void runAuthGuard();
	});

	async function runAuthGuard() {
		await auth.checkAuth();
		if (!auth.isAuthenticated) {
			goto(`/login?redirect=${encodeURIComponent(page.url.pathname)}`);
		}
	}
</script>

{#if auth.loading}
	<div class="flex h-screen items-center justify-center bg-background">
		<div class="text-muted-foreground">Loading...</div>
	</div>
{:else}
	{@render children()}
{/if}
