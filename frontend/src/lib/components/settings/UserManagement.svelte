<script lang="ts">
  import { api } from '$lib/api/index.js';
  import type { User } from '$lib/api/types.js';
  import { auth } from '$lib/stores/auth.svelte.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import CreateUserDialog from './CreateUserDialog.svelte';
  import EditUserDialog from './EditUserDialog.svelte';
  import ResetPasswordDialog from './ResetPasswordDialog.svelte';

  let users = $state<User[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let createDialogOpen = $state(false);
  let editDialogOpen = $state(false);
  let resetPwDialogOpen = $state(false);
  let editingUser = $state<User | null>(null);
  let resetPwUser = $state<User | null>(null);

  const adminCount = $derived(users.filter((u) => u.role === 'admin' && u.is_active).length);

  async function fetchUsers() {
    loading = true;
    error = null;
    try {
      users = await api.users.list();
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to load users';
    } finally {
      loading = false;
    }
  }

  function handleCreated(user: User) {
    users = [...users, user];
    createDialogOpen = false;
  }

  function handleUpdated(user: User) {
    users = users.map((u) => (u.id === user.id ? user : u));
    editDialogOpen = false;
    editingUser = null;
  }

  function openEdit(user: User) {
    editingUser = user;
    editDialogOpen = true;
  }

  function openResetPassword(user: User) {
    resetPwUser = user;
    resetPwDialogOpen = true;
  }

  async function toggleActive(user: User) {
    if (user.id === auth.user?.id) return;
    if (!user.is_active) {
      // Reactivate
      try {
        const updated = await api.users.update(user.id, { is_active: true });
        users = users.map((u) => (u.id === updated.id ? updated : u));
      } catch (err) {
        error = err instanceof Error ? err.message : 'Failed to activate user';
      }
    } else {
      // Deactivate
      if (user.role === 'admin' && adminCount <= 1) {
        error = 'Cannot deactivate the last admin user';
        return;
      }
      try {
        await api.users.delete(user.id);
        users = users.map((u) => (u.id === user.id ? { ...u, is_active: false } : u));
      } catch (err) {
        error = err instanceof Error ? err.message : 'Failed to deactivate user';
      }
    }
  }

  function formatDate(dateString: string): string {
    return new Date(dateString).toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric'
    });
  }

  $effect(() => {
    fetchUsers();
  });
</script>

<div class="rounded-lg border border-border bg-card">
  <div class="border-b border-border px-6 py-4">
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-2">
        <h2 class="text-base font-semibold">Users</h2>
        {#if !loading && users.length > 0}
          <span
            class="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground"
          >
            {users.length}
          </span>
        {/if}
      </div>
      {#if !loading}
        <Button variant="outline" size="sm" onclick={() => (createDialogOpen = true)}>
          <svg
            class="mr-1.5 size-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M5 12h14" />
            <path d="M12 5v14" />
          </svg>
          Add User
        </Button>
      {/if}
    </div>
  </div>

  {#if error}
    <div class="px-6 py-4">
      <div
        class="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
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
          <circle cx="12" cy="12" r="10" />
          <line x1="12" x2="12" y1="8" y2="12" />
          <line x1="12" x2="12.01" y1="16" y2="16" />
        </svg>
        <span>{error}</span>
        <button class="ml-auto text-xs underline" onclick={() => (error = null)}> dismiss </button>
      </div>
    </div>
  {/if}

  {#if loading}
    <div class="flex items-center justify-center px-6 py-8">
      <span class="text-sm text-muted-foreground">Loading users...</span>
    </div>
  {:else if users.length === 0}
    <div class="px-6 py-8 text-center">
      <p class="text-sm text-muted-foreground">No users found.</p>
    </div>
  {:else}
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-border text-left text-xs text-muted-foreground">
            <th class="px-6 py-3 font-medium">Username</th>
            <th class="px-6 py-3 font-medium">Email</th>
            <th class="px-6 py-3 font-medium">Role</th>
            <th class="px-6 py-3 font-medium">Status</th>
            <th class="px-6 py-3 font-medium">Created</th>
            <th class="px-6 py-3 text-right font-medium">Actions</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-border/50">
          {#each users as user (user.id)}
            <tr class={user.id === auth.user?.id ? 'bg-primary/5' : ''}>
              <td class="px-6 py-3 font-medium">
                {user.username}
                {#if user.id === auth.user?.id}
                  <span class="ml-1 text-xs text-muted-foreground">(you)</span>
                {/if}
              </td>
              <td class="px-6 py-3 text-muted-foreground">
                {user.email ?? '\u2014'}
              </td>
              <td class="px-6 py-3">
                <span
                  class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium
									{user.role === 'admin'
                    ? 'bg-blue-500/10 text-blue-700 dark:text-blue-400'
                    : 'bg-muted text-muted-foreground'}"
                >
                  {user.role}
                </span>
              </td>
              <td class="px-6 py-3">
                <span
                  class="inline-flex items-center gap-1.5 text-xs
									{user.is_active ? 'text-green-700 dark:text-green-400' : 'text-muted-foreground'}"
                >
                  <span
                    class="size-1.5 rounded-full
										{user.is_active ? 'bg-green-500' : 'bg-gray-400'}"
                  ></span>
                  {user.is_active ? 'Active' : 'Inactive'}
                </span>
              </td>
              <td class="px-6 py-3 text-muted-foreground">
                {formatDate(user.created_at)}
              </td>
              <td class="px-6 py-3 text-right">
                <div class="flex items-center justify-end gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onclick={() => openEdit(user)}
                    aria-label="Edit {user.username}"
                    title="Edit"
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
                      <path
                        d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"
                      />
                    </svg>
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onclick={() => openResetPassword(user)}
                    aria-label="Reset password for {user.username}"
                    title="Reset password"
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
                      <circle cx="7.5" cy="15.5" r="5.5" />
                      <path d="m21 2-9.6 9.6" />
                      <path d="m15.5 7.5 3 3L22 7l-3-3" />
                    </svg>
                  </Button>
                  {#if user.id !== auth.user?.id}
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onclick={() => toggleActive(user)}
                      aria-label={user.is_active
                        ? `Deactivate ${user.username}`
                        : `Activate ${user.username}`}
                      title={user.is_active ? 'Deactivate' : 'Activate'}
                    >
                      {#if user.is_active}
                        <svg
                          class="size-4 text-destructive"
                          xmlns="http://www.w3.org/2000/svg"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        >
                          <circle cx="12" cy="12" r="10" />
                          <path d="m15 9-6 6" />
                          <path d="m9 9 6 6" />
                        </svg>
                      {:else}
                        <svg
                          class="size-4 text-green-600 dark:text-green-400"
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
                      {/if}
                    </Button>
                  {/if}
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<CreateUserDialog bind:open={createDialogOpen} oncreated={handleCreated} />

{#if editingUser}
  <EditUserDialog
    bind:open={editDialogOpen}
    user={editingUser}
    isSelf={editingUser.id === auth.user?.id}
    onupdated={handleUpdated}
  />
{/if}

{#if resetPwUser}
  <ResetPasswordDialog bind:open={resetPwDialogOpen} user={resetPwUser} />
{/if}
