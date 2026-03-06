<script lang="ts">
  import { auth } from '$lib/stores/auth.svelte.js';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';

  let { children } = $props();

  // Redirect if not authenticated — root layout already calls checkAuth(),
  // so we only read the current state here (no second checkAuth() call,
  // which would set loading=true and cause an unmount/remount loop).
  $effect(() => {
    if (!auth.loading && !auth.isAuthenticated) {
      goto(`/login?redirect=${encodeURIComponent(page.url.pathname)}`);
    }
  });
</script>

{@render children()}
