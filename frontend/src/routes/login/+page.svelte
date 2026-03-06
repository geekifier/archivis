<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { auth } from '$lib/stores/auth.svelte.js';
  import { ApiError } from '$lib/api/errors.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import * as Card from '$lib/components/ui/card/index.js';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let submitting = $state(false);

  // Redirect destination after login
  const redirectTo = $derived(page.url.searchParams.get('redirect') || '/');

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    submitting = true;

    try {
      await auth.login(username, password);
      goto(redirectTo);
    } catch (err) {
      if (err instanceof ApiError) {
        error = err.userMessage;
      } else {
        error = 'An unexpected error occurred. Please try again.';
      }
    } finally {
      submitting = false;
    }
  }
</script>

<div class="flex min-h-[80vh] items-center justify-center">
  <Card.Card class="w-full max-w-sm">
    <Card.CardHeader>
      <Card.CardTitle class="text-2xl">Log in</Card.CardTitle>
      <Card.CardDescription>Sign in to your Archivis account</Card.CardDescription>
    </Card.CardHeader>
    <Card.CardContent>
      <form onsubmit={handleSubmit} class="space-y-4">
        {#if error}
          <div class="rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        {/if}

        <div class="space-y-2">
          <Label for="username">Username</Label>
          <Input
            id="username"
            type="text"
            bind:value={username}
            required
            autocomplete="username"
            disabled={submitting}
          />
        </div>

        <div class="space-y-2">
          <Label for="password">Password</Label>
          <Input
            id="password"
            type="password"
            bind:value={password}
            required
            autocomplete="current-password"
            disabled={submitting}
          />
        </div>

        <Button type="submit" class="w-full" disabled={submitting}>
          {#if submitting}
            Signing in...
          {:else}
            Sign in
          {/if}
        </Button>
      </form>
    </Card.CardContent>
  </Card.Card>
</div>
