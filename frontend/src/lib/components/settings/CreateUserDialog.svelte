<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { User } from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import * as Select from '$lib/components/ui/select/index.js';

	interface Props {
		open: boolean;
		oncreated: (user: User) => void;
	}

	let { open = $bindable(), oncreated }: Props = $props();

	let username = $state('');
	let email = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let role = $state<string>('user');
	let submitting = $state(false);
	let submitError = $state<string | null>(null);

	const passwordMismatch = $derived(
		confirmPassword.length > 0 && password !== confirmPassword
	);
	const passwordTooShort = $derived(
		password.length > 0 && password.length < 8
	);
	const canSubmit = $derived(
		username.trim().length > 0 &&
			password.length >= 8 &&
			password === confirmPassword &&
			!submitting
	);

	function reset() {
		username = '';
		email = '';
		password = '';
		confirmPassword = '';
		role = 'user';
		submitError = null;
	}

	function handleOpenChange(isOpen: boolean) {
		open = isOpen;
		if (!isOpen) {
			reset();
		}
	}

	async function handleSubmit() {
		if (!canSubmit) return;

		submitting = true;
		submitError = null;

		try {
			const user = await api.users.create({
				username: username.trim(),
				password,
				email: email.trim() || undefined,
				role
			});
			reset();
			oncreated(user);
		} catch (err) {
			submitError = err instanceof Error ? err.message : 'Failed to create user';
		} finally {
			submitting = false;
		}
	}
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Add User</Dialog.Title>
			<Dialog.Description>
				Create a new user account.
			</Dialog.Description>
		</Dialog.Header>

		<div class="space-y-4">
			<div>
				<Label for="create-username" class="mb-1.5 block text-sm font-medium">
					Username
				</Label>
				<Input
					id="create-username"
					type="text"
					placeholder="Enter username"
					bind:value={username}
					disabled={submitting}
				/>
			</div>

			<div>
				<Label for="create-email" class="mb-1.5 block text-sm font-medium">
					Email <span class="text-muted-foreground">(optional)</span>
				</Label>
				<Input
					id="create-email"
					type="email"
					placeholder="user@example.com"
					bind:value={email}
					disabled={submitting}
				/>
			</div>

			<div>
				<Label for="create-password" class="mb-1.5 block text-sm font-medium">
					Password
				</Label>
				<Input
					id="create-password"
					type="password"
					placeholder="Minimum 8 characters"
					bind:value={password}
					disabled={submitting}
				/>
				{#if passwordTooShort}
					<p class="mt-1 text-xs text-destructive">
						Password must be at least 8 characters
					</p>
				{/if}
			</div>

			<div>
				<Label for="create-confirm-password" class="mb-1.5 block text-sm font-medium">
					Confirm Password
				</Label>
				<Input
					id="create-confirm-password"
					type="password"
					placeholder="Re-enter password"
					bind:value={confirmPassword}
					disabled={submitting}
				/>
				{#if passwordMismatch}
					<p class="mt-1 text-xs text-destructive">
						Passwords do not match
					</p>
				{/if}
			</div>

			<div>
				<Label class="mb-1.5 block text-sm font-medium">
					Role
				</Label>
				<Select.Root type="single" bind:value={role}>
					<Select.Trigger class="w-full">
						{role === 'admin' ? 'Admin' : 'User'}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="user" label="User">User</Select.Item>
						<Select.Item value="admin" label="Admin">Admin</Select.Item>
					</Select.Content>
				</Select.Root>
			</div>

			{#if submitError}
				<div
					class="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
				>
					{submitError}
				</div>
			{/if}
		</div>

		<Dialog.Footer>
			<Dialog.Close>Cancel</Dialog.Close>
			<Button onclick={handleSubmit} disabled={!canSubmit}>
				{#if submitting}
					Creating...
				{:else}
					Create User
				{/if}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
