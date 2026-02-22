<script lang="ts">
	import { goto } from '$app/navigation';
	import { auth } from '$lib/stores/auth.svelte.js';
	import { ApiError } from '$lib/api/errors.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import * as Card from '$lib/components/ui/card/index.js';

	let username = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let email = $state('');
	let error = $state('');
	let submitting = $state(false);

	const passwordMismatch = $derived(
		confirmPassword.length > 0 && password !== confirmPassword
	);
	const passwordTooShort = $derived(password.length > 0 && password.length < 8);

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		error = '';

		if (password !== confirmPassword) {
			error = 'Passwords do not match.';
			return;
		}

		if (password.length < 8) {
			error = 'Password must be at least 8 characters.';
			return;
		}

		submitting = true;

		try {
			await auth.setup(username, password, email || undefined);
			goto('/');
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
			<Card.CardTitle class="text-2xl">Welcome to Archivis</Card.CardTitle>
			<Card.CardDescription>Create your admin account to get started</Card.CardDescription>
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
					<Label for="email">Email <span class="text-muted-foreground">(optional)</span></Label>
					<Input
						id="email"
						type="email"
						bind:value={email}
						autocomplete="email"
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
						minlength={8}
						autocomplete="new-password"
						disabled={submitting}
					/>
					{#if passwordTooShort}
						<p class="text-xs text-destructive">Must be at least 8 characters</p>
					{/if}
				</div>

				<div class="space-y-2">
					<Label for="confirm-password">Confirm password</Label>
					<Input
						id="confirm-password"
						type="password"
						bind:value={confirmPassword}
						required
						autocomplete="new-password"
						disabled={submitting}
					/>
					{#if passwordMismatch}
						<p class="text-xs text-destructive">Passwords do not match</p>
					{/if}
				</div>

				<Button type="submit" class="w-full" disabled={submitting || passwordMismatch || passwordTooShort}>
					{#if submitting}
						Creating account...
					{:else}
						Create account
					{/if}
				</Button>
			</form>
		</Card.CardContent>
	</Card.Card>
</div>
