<script lang="ts">
  import { api } from '$lib/api/index.js';
  import { auth } from '$lib/stores/auth.svelte.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';

  interface Props {
    open: boolean;
  }

  let { open = $bindable() }: Props = $props();

  let currentPassword = $state('');
  let newPassword = $state('');
  let confirmPassword = $state('');
  let submitting = $state(false);
  let submitError = $state<string | null>(null);
  let success = $state(false);

  const passwordMismatch = $derived(confirmPassword.length > 0 && newPassword !== confirmPassword);
  const passwordTooShort = $derived(newPassword.length > 0 && newPassword.length < 8);
  const canSubmit = $derived(
    currentPassword.length > 0 &&
      newPassword.length >= 8 &&
      newPassword === confirmPassword &&
      !submitting
  );

  function reset() {
    currentPassword = '';
    newPassword = '';
    confirmPassword = '';
    submitError = null;
    success = false;
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
      await api.auth.changePassword({
        current_password: currentPassword,
        new_password: newPassword
      });
      success = true;
      setTimeout(() => {
        open = false;
        auth.logout();
      }, 2000);
    } catch (err) {
      submitError = err instanceof Error ? err.message : 'Failed to change password';
    } finally {
      submitting = false;
    }
  }
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>Change Password</Dialog.Title>
      <Dialog.Description>
        Update your password. You will be logged out after changing it.
      </Dialog.Description>
    </Dialog.Header>

    <div class="space-y-4">
      {#if success}
        <div
          class="flex items-center gap-2 rounded-lg border border-green-500/30 bg-green-500/10 px-4 py-3 text-sm text-green-700 dark:text-green-400"
        >
          <svg
            class="size-4 shrink-0"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
            <path d="m9 11 3 3L22 4" />
          </svg>
          <span>Password changed. Redirecting to login...</span>
        </div>
      {:else}
        <div>
          <Label for="change-current-password" class="mb-1.5 block text-sm font-medium">
            Current Password
          </Label>
          <Input
            id="change-current-password"
            type="password"
            placeholder="Enter current password"
            bind:value={currentPassword}
            disabled={submitting}
          />
        </div>

        <div>
          <Label for="change-new-password" class="mb-1.5 block text-sm font-medium">
            New Password
          </Label>
          <Input
            id="change-new-password"
            type="password"
            placeholder="Minimum 8 characters"
            bind:value={newPassword}
            disabled={submitting}
          />
          {#if passwordTooShort}
            <p class="mt-1 text-xs text-destructive">Password must be at least 8 characters</p>
          {/if}
        </div>

        <div>
          <Label for="change-confirm-password" class="mb-1.5 block text-sm font-medium">
            Confirm New Password
          </Label>
          <Input
            id="change-confirm-password"
            type="password"
            placeholder="Re-enter new password"
            bind:value={confirmPassword}
            disabled={submitting}
          />
          {#if passwordMismatch}
            <p class="mt-1 text-xs text-destructive">Passwords do not match</p>
          {/if}
        </div>

        {#if submitError}
          <div
            class="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
          >
            {submitError}
          </div>
        {/if}
      {/if}
    </div>

    <Dialog.Footer>
      <Dialog.Close>{success ? 'Close' : 'Cancel'}</Dialog.Close>
      {#if !success}
        <Button onclick={handleSubmit} disabled={!canSubmit}>
          {#if submitting}
            Changing...
          {:else}
            Change Password
          {/if}
        </Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
