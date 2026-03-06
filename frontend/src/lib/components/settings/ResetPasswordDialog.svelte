<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type { User } from '$lib/api/types.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';

  interface Props {
    open: boolean;
    user: User;
  }

  let { open = $bindable(), user }: Props = $props();

  let password = $state('');
  let confirmPassword = $state('');
  let submitting = $state(false);
  let submitError = $state<string | null>(null);
  let success = $state(false);

  const passwordMismatch = $derived(confirmPassword.length > 0 && password !== confirmPassword);
  const passwordTooShort = $derived(password.length > 0 && password.length < 8);
  const canSubmit = $derived(password.length >= 8 && password === confirmPassword && !submitting);

  function reset() {
    password = '';
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
      await api.users.resetPassword(user.id, { new_password: password });
      success = true;
      password = '';
      confirmPassword = '';
      setTimeout(() => {
        open = false;
      }, 1500);
    } catch (err) {
      submitError = err instanceof Error ? err.message : 'Failed to reset password';
    } finally {
      submitting = false;
    }
  }
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>Reset Password</Dialog.Title>
      <Dialog.Description>
        Set a new password for {user.username}.
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
          <span>Password reset successfully.</span>
        </div>
      {:else}
        <div>
          <Label for="reset-password" class="mb-1.5 block text-sm font-medium">New Password</Label>
          <Input
            id="reset-password"
            type="password"
            placeholder="Minimum 8 characters"
            bind:value={password}
            disabled={submitting}
          />
          {#if passwordTooShort}
            <p class="mt-1 text-xs text-destructive">Password must be at least 8 characters</p>
          {/if}
        </div>

        <div>
          <Label for="reset-confirm-password" class="mb-1.5 block text-sm font-medium">
            Confirm Password
          </Label>
          <Input
            id="reset-confirm-password"
            type="password"
            placeholder="Re-enter password"
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
            Resetting...
          {:else}
            Reset Password
          {/if}
        </Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
