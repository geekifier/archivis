<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { User, UpdateUserRequest } from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import * as Select from '$lib/components/ui/select/index.js';
	import { Switch } from '$lib/components/ui/switch/index.js';

	interface Props {
		open: boolean;
		user: User;
		isSelf: boolean;
		onupdated: (user: User) => void;
	}

	let { open = $bindable(), user, isSelf, onupdated }: Props = $props();

	let username = $state('');
	let email = $state('');
	let role = $state<string>('user');
	let isActive = $state(true);
	let submitting = $state(false);
	let submitError = $state<string | null>(null);

	// Initialize form fields whenever user changes
	$effect(() => {
		if (user) {
			username = user.username;
			email = user.email ?? '';
			role = user.role;
			isActive = user.is_active;
		}
	});

	const hasChanges = $derived(
		username.trim() !== user.username ||
			(email.trim() || null) !== (user.email ?? null) ||
			role !== user.role ||
			isActive !== user.is_active
	);

	const canSubmit = $derived(
		hasChanges && username.trim().length > 0 && !submitting
	);

	function handleOpenChange(isOpen: boolean) {
		open = isOpen;
		if (!isOpen) {
			submitError = null;
		}
	}

	async function handleSubmit() {
		if (!canSubmit) return;

		submitting = true;
		submitError = null;

		const updates: UpdateUserRequest = {};
		if (username.trim() !== user.username) {
			updates.username = username.trim();
		}
		const newEmail = email.trim() || null;
		if (newEmail !== (user.email ?? null)) {
			updates.email = newEmail;
		}
		if (role !== user.role) {
			updates.role = role;
		}
		if (isActive !== user.is_active) {
			updates.is_active = isActive;
		}

		try {
			const updated = await api.users.update(user.id, updates);
			onupdated(updated);
		} catch (err) {
			submitError = err instanceof Error ? err.message : 'Failed to update user';
		} finally {
			submitting = false;
		}
	}
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Edit User</Dialog.Title>
			<Dialog.Description>
				Update account details for {user.username}.
			</Dialog.Description>
		</Dialog.Header>

		<div class="space-y-4">
			<div>
				<Label for="edit-username" class="mb-1.5 block text-sm font-medium">
					Username
				</Label>
				<Input
					id="edit-username"
					type="text"
					bind:value={username}
					disabled={submitting}
				/>
			</div>

			<div>
				<Label for="edit-email" class="mb-1.5 block text-sm font-medium">
					Email
				</Label>
				<Input
					id="edit-email"
					type="email"
					placeholder="user@example.com"
					bind:value={email}
					disabled={submitting}
				/>
			</div>

			<div>
				<Label class="mb-1.5 block text-sm font-medium">
					Role
					{#if isSelf}
						<span class="text-xs text-muted-foreground">(cannot change own role)</span>
					{/if}
				</Label>
				<Select.Root type="single" bind:value={role} disabled={isSelf}>
					<Select.Trigger class="w-full">
						{role === 'admin' ? 'Admin' : 'User'}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="user" label="User">User</Select.Item>
						<Select.Item value="admin" label="Admin">Admin</Select.Item>
					</Select.Content>
				</Select.Root>
			</div>

			{#if !isSelf}
				<div class="flex items-center justify-between gap-4">
					<div class="space-y-0.5">
						<Label for="edit-active" class="text-sm font-medium">
							Active
						</Label>
						<p class="text-xs text-muted-foreground">
							Inactive users cannot log in.
						</p>
					</div>
					<Switch
						id="edit-active"
						checked={isActive}
						onCheckedChange={(checked) => (isActive = checked)}
						disabled={submitting}
					/>
				</div>
			{/if}

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
					Saving...
				{:else}
					Save Changes
				{/if}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
