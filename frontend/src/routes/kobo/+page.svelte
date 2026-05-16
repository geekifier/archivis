<script lang="ts">
  import { onMount } from 'svelte';
  import { auth } from '$lib/stores/auth.svelte.js';
  import { api } from '$lib/api/index.js';
  import type {
    KoboDeviceResponse,
    KoboStatusResponse,
    PairKoboDeviceResponse
  } from '$lib/api/types.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';

  let devices = $state<KoboDeviceResponse[]>([]);
  let loading = $state(true);
  let pairing = $state(false);
  let displayName = $state('');
  let newDevice = $state<PairKoboDeviceResponse | null>(null);
  let status = $state<KoboStatusResponse | null>(null);
  let error = $state<string | null>(null);

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [devicesResp, statusResp] = await Promise.all([
        api.kobo.listDevices(),
        api.kobo.status()
      ]);
      devices = devicesResp;
      status = statusResp;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function pair() {
    if (status?.enabled === false) return;

    pairing = true;
    error = null;
    try {
      newDevice = await api.kobo.pairDevice({ display_name: displayName.trim() || undefined });
      displayName = '';
      await load();
      window.dispatchEvent(new CustomEvent('archivis:kobo-status-updated'));
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      pairing = false;
    }
  }

  async function revoke(id: string) {
    error = null;
    try {
      await api.kobo.revokeDevice(id);
      await load();
      window.dispatchEvent(new CustomEvent('archivis:kobo-status-updated'));
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function dismissNewDevice() {
    newDevice = null;
  }

  function fmtDate(s: string | null): string {
    if (!s) return '—';
    try {
      return new Date(s).toLocaleString();
    } catch {
      return s;
    }
  }
</script>

<svelte:head>
  <title>Kobo Sync · Archivis</title>
</svelte:head>

<div class="mx-auto max-w-3xl space-y-6 p-6">
  <header>
    <h1 class="text-2xl font-bold">Kobo Sync</h1>
    <p class="text-sm text-muted-foreground">
      Pair a Kobo eReader to deliver KEPUB-converted books from your Archivis library.
    </p>
  </header>

  {#if !auth.isAuthenticated}
    <p class="text-sm">Sign in to manage Kobo devices.</p>
  {:else}
    {@const koboEnabled = status?.enabled !== false}
    <section class="rounded-lg border bg-card p-4">
      <h2 class="text-lg font-semibold">Pair a new device</h2>
      <p class="mb-3 text-sm text-muted-foreground">
        Point your Kobo's "API server" setting at the URL shown after pairing.
      </p>
      {#if !koboEnabled}
        <p class="mb-3 rounded border border-amber-500/30 bg-amber-500/5 p-2 text-sm text-amber-700 dark:text-amber-400">
          Kobo Sync is disabled in Settings.
        </p>
      {/if}
      <div class="flex flex-col gap-2 sm:flex-row sm:items-end">
        <div class="flex-1">
          <Label for="kobo-display-name">Display name</Label>
          <Input
            id="kobo-display-name"
            placeholder="Kobo Libra"
            bind:value={displayName}
            disabled={pairing || !koboEnabled}
          />
        </div>
        <Button onclick={pair} disabled={pairing || !koboEnabled}>
          {pairing ? 'Pairing…' : 'Pair device'}
        </Button>
      </div>
    </section>

    {#if newDevice}
      <section class="rounded-lg border border-amber-500/30 bg-amber-500/5 p-4">
        <h2 class="text-base font-semibold">Device paired — copy now</h2>
        <p class="mb-2 text-sm text-muted-foreground">
          The token below is shown only once. Save it somewhere safe; you'll need it to configure
          the device.
        </p>
        <dl class="space-y-2 text-sm">
          <div>
            <dt class="font-medium">API endpoint</dt>
            <dd>
              <code class="block break-all rounded bg-muted px-2 py-1 font-mono text-xs">
                {newDevice.api_endpoint}
              </code>
            </dd>
          </div>
          <div>
            <dt class="font-medium">Token</dt>
            <dd>
              <code class="block break-all rounded bg-muted px-2 py-1 font-mono text-xs">
                {newDevice.token}
              </code>
            </dd>
          </div>
        </dl>
        <Button class="mt-3" variant="outline" onclick={dismissNewDevice}>I copied it</Button>
      </section>
    {/if}

    <section class="rounded-lg border bg-card p-4">
      <h2 class="mb-3 text-lg font-semibold">Paired devices</h2>
      {#if loading}
        <p class="text-sm text-muted-foreground">Loading…</p>
      {:else if devices.length === 0}
        <p class="text-sm text-muted-foreground">No devices paired yet.</p>
      {:else}
        <ul class="divide-y">
          {#each devices as device (device.id)}
            <li class="flex flex-col gap-2 py-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div class="font-medium">{device.display_name}</div>
                <div class="text-xs text-muted-foreground">
                  Created {fmtDate(device.created_at)} · Last seen {fmtDate(device.last_seen_at)}
                  {#if device.revoked_at}
                    · <span class="text-destructive">Revoked {fmtDate(device.revoked_at)}</span>
                  {/if}
                </div>
              </div>
              {#if !device.revoked_at}
                <Button variant="destructive" size="sm" onclick={() => revoke(device.id)}>
                  Revoke
                </Button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if error}
      <p class="rounded border border-destructive/30 bg-destructive/5 p-2 text-sm text-destructive">
        {error}
      </p>
    {/if}
  {/if}
</div>
