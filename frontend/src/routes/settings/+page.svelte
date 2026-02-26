<script lang="ts">
	import { goto } from '$app/navigation';
	import { SvelteMap, SvelteSet } from 'svelte/reactivity';
	import { auth } from '$lib/stores/auth.svelte.js';
	import { api } from '$lib/api/index.js';
	import type { SettingEntry } from '$lib/api/types.js';
	import { Button } from '$lib/components/ui/button/index.js';

	// Admin guard
	$effect(() => {
		if (!auth.loading && auth.user && auth.user.role !== 'admin') {
			goto('/');
		}
	});

	let settings = $state<SettingEntry[]>([]);
	let loading = $state(true);
	let saving = $state(false);
	let error = $state<string | null>(null);
	let successMessage = $state<string | null>(null);
	let restartRequired = $state(false);

	// Track edited values (key -> new value)
	let editedValues = $state<Record<string, unknown>>({});
	// Track which sensitive fields are visible
	let visibleSecrets = new SvelteSet<string>();

	const sections = $derived(groupBySection(settings));
	const hasChanges = $derived(Object.keys(editedValues).length > 0);

	function groupBySection(entries: SettingEntry[]): SvelteMap<string, SettingEntry[]> {
		const map = new SvelteMap<string, SettingEntry[]>();
		for (const entry of entries) {
			const existing = map.get(entry.section);
			if (existing) {
				existing.push(entry);
			} else {
				map.set(entry.section, [entry]);
			}
		}
		return map;
	}

	function sectionLabel(section: string): string {
		const labels: Record<string, string> = {
			server: 'Server',
			metadata: 'Metadata Providers',
			'metadata.open_library': 'Open Library',
			'metadata.hardcover': 'Hardcover',
			isbn_scan: 'ISBN Scanning'
		};
		return labels[section] ?? section;
	}

	function isSubsection(section: string): boolean {
		return section.startsWith('metadata.') && section !== 'metadata';
	}

	function isBootstrapSection(entries: SettingEntry[]): boolean {
		return entries.length > 0 && entries.every((e) => e.scope === 'bootstrap');
	}

	function bootstrapSource(entry: SettingEntry): { label: string; detail?: string } {
		if (entry.override) {
			if (entry.override.env_var) return { label: 'env', detail: entry.override.env_var };
			return { label: 'cli' };
		}
		if (entry.source === 'file') return { label: 'config file' };
		if (entry.source === 'database') return { label: 'database' };
		return { label: 'default' };
	}

	function getCurrentValue(entry: SettingEntry): unknown {
		if (entry.key in editedValues) {
			return editedValues[entry.key];
		}
		return entry.value;
	}

	function handleChange(key: string, value: unknown, originalValue: unknown) {
		// Compare with original to determine if dirty
		if (value === originalValue || (value === '' && originalValue === null)) {
			editedValues = Object.fromEntries(
				Object.entries(editedValues).filter(([k]) => k !== key)
			);
		} else {
			editedValues = { ...editedValues, [key]: value };
		}
	}

	function handleInputChange(entry: SettingEntry, event: Event) {
		const target = event.target as HTMLInputElement;
		let value: unknown;

		switch (entry.value_type) {
			case 'bool':
				value = target.checked;
				break;
			case 'integer':
				value = target.value === '' ? null : parseInt(target.value, 10);
				break;
			case 'float':
				value = target.value === '' ? null : parseFloat(target.value);
				break;
			case 'optional_string':
				value = target.value === '' ? null : target.value;
				break;
			default:
				value = target.value;
		}

		handleChange(entry.key, value, entry.value);
	}

	function toggleSecret(key: string) {
		if (visibleSecrets.has(key)) {
			visibleSecrets.delete(key);
		} else {
			visibleSecrets.add(key);
		}
	}

	function resetField(key: string) {
		// Setting to null means "reset to default" in the API
		editedValues = { ...editedValues, [key]: null };
	}

	function cancelChanges() {
		editedValues = {};
	}

	async function saveChanges() {
		if (!hasChanges) return;

		saving = true;
		error = null;
		successMessage = null;

		try {
			const result = await api.settings.update(editedValues);
			editedValues = {};

			if (result.requires_restart) {
				restartRequired = true;
			}

			successMessage = `Updated ${result.updated.length} setting${result.updated.length === 1 ? '' : 's'} successfully.`;

			// Refetch to get updated values
			await fetchSettings();

			// Clear success after 5 seconds
			setTimeout(() => {
				successMessage = null;
			}, 5000);
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to save settings';
		} finally {
			saving = false;
		}
	}

	async function fetchSettings() {
		try {
			const response = await api.settings.get();
			settings = response.settings;
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to load settings';
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (auth.isAuthenticated) {
			fetchSettings();
		}
	});
</script>

<svelte:head>
	<title>Settings - Archivis</title>
</svelte:head>

<div class="mx-auto max-w-3xl space-y-6">
	<div>
		<h1 class="text-2xl font-bold tracking-tight">Settings</h1>
		<p class="mt-1 text-sm text-muted-foreground">
			Manage your Archivis instance configuration
		</p>
	</div>

	{#if restartRequired}
		<div
			class="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-700 dark:text-amber-400"
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
				<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" />
				<path d="M12 9v4" />
				<path d="M12 17h.01" />
			</svg>
			<span>Some changes require a server restart to take effect.</span>
		</div>
	{/if}

	{#if successMessage}
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
			<span>{successMessage}</span>
		</div>
	{/if}

	{#if error}
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
		</div>
	{/if}

	{#if loading}
		<div class="flex items-center justify-center py-12">
			<span class="text-muted-foreground">Loading settings...</span>
		</div>
	{:else}
		{#each [...sections] as [section, entries] (section)}
			<div class="rounded-lg border border-border bg-card">
				<div class="border-b border-border px-6 py-4">
					<h2
						class={isSubsection(section)
							? 'text-sm font-semibold text-muted-foreground'
							: 'text-base font-semibold'}
					>
						{sectionLabel(section)}
					</h2>
				</div>

				{#if isBootstrapSection(entries)}
					<!-- Compact table for read-only bootstrap settings -->
					<div class="px-6 py-3">
						<table class="w-full text-sm">
							<thead>
								<tr class="text-left text-xs text-muted-foreground">
									<th class="pb-2 font-medium">Setting</th>
									<th class="pb-2 font-medium">Value</th>
									<th class="pb-2 text-right font-medium">Source</th>
								</tr>
							</thead>
							<tbody class="divide-y divide-border/50">
								{#each entries as entry (entry.key)}
									{@const source = bootstrapSource(entry)}
									<tr>
										<td class="py-2 pr-4 font-medium">{entry.label}</td>
										<td class="py-2 pr-4 font-mono text-xs text-muted-foreground">
											{entry.sensitive
												? '***'
												: String(entry.effective_value ?? entry.value ?? '\u2014')}
										</td>
										<td class="py-2 text-right">
											<span
												class="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
												title={source.detail
													? source.detail
													: source.label === 'cli'
														? 'CLI flag'
														: ''}
											>
												{source.label}
											</span>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
						<p class="mt-3 text-xs text-muted-foreground">
							Server settings are read-only. Change them via config file, environment
							variables, or CLI flags.
						</p>
					</div>
				{:else}
					<!-- Editable runtime settings -->
					<div class="divide-y divide-border">
						{#each entries as entry (entry.key)}
							<div class="px-6 py-4">
								<div class="flex items-start justify-between gap-4">
									<div class="min-w-0 flex-1">
										<div class="flex items-center gap-2">
											<label for={entry.key} class="text-sm font-medium">
												{entry.label}
											</label>
											{#if entry.requires_restart}
												<span
													class="inline-flex items-center rounded-full bg-amber-500/10 px-2 py-0.5 text-xs font-medium text-amber-600 dark:text-amber-400"
													title="Requires server restart"
												>
													restart
												</span>
											{/if}
											{#if entry.override?.source === 'env' || entry.override?.source === 'cli'}
												<span
													class="inline-flex items-center rounded-full bg-amber-500/10 px-2 py-0.5 text-xs font-medium text-amber-600 dark:text-amber-400"
												>
													{entry.override.source}
												</span>
											{:else if entry.source === 'database'}
												<span
													class="inline-flex items-center rounded-full bg-blue-500/10 px-2 py-0.5 text-xs font-medium text-blue-600 dark:text-blue-400"
												>
													modified
												</span>
											{:else if entry.source === 'file'}
												<span
													class="inline-flex items-center rounded-full bg-zinc-500/10 px-2 py-0.5 text-xs font-medium text-zinc-600 dark:text-zinc-400"
												>
													config file
												</span>
											{/if}
										</div>
										<p class="mt-0.5 text-xs text-muted-foreground">
											{entry.description}
										</p>
									</div>

									<div class="flex shrink-0 items-center gap-2">
										{#if entry.value_type === 'bool'}
											<button
												type="button"
												role="switch"
												aria-checked={getCurrentValue(entry) === true}
												aria-label={entry.label}
												class="relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors
												{getCurrentValue(entry) === true
													? 'bg-primary'
													: 'bg-muted'}"
												onclick={() =>
													handleChange(
														entry.key,
														getCurrentValue(entry) !== true,
														entry.value
													)}
											>
												<span
													class="pointer-events-none inline-block size-5 rounded-full bg-background shadow-lg ring-0 transition-transform
													{getCurrentValue(entry) === true
														? 'translate-x-5'
														: 'translate-x-0'}"
												></span>
											</button>
										{:else if entry.sensitive}
											<div class="flex items-center gap-1">
												<input
													id={entry.key}
													type={visibleSecrets.has(entry.key) ? 'text' : 'password'}
													class="h-9 w-56 rounded-md border border-input bg-background px-3 text-sm"
													value={entry.key in editedValues
														? (editedValues[entry.key] ?? '')
														: ''}
													placeholder={entry.is_set ? '••••••••' : 'Not set'}
													oninput={(e) => handleInputChange(entry, e)}
												/>
												<Button
													variant="ghost"
													size="icon-sm"
													onclick={() => toggleSecret(entry.key)}
													aria-label={visibleSecrets.has(entry.key)
														? 'Hide value'
														: 'Show value'}
												>
													{#if visibleSecrets.has(entry.key)}
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
																d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"
															/>
															<line x1="1" x2="23" y1="1" y2="23" />
														</svg>
													{:else}
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
																d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"
															/>
															<circle cx="12" cy="12" r="3" />
														</svg>
													{/if}
												</Button>
											</div>
										{:else if entry.value_type === 'integer'}
											<input
												id={entry.key}
												type="number"
												class="h-9 w-32 rounded-md border border-input bg-background px-3 text-sm"
												value={getCurrentValue(entry) ?? ''}
												oninput={(e) => handleInputChange(entry, e)}
											/>
										{:else if entry.value_type === 'float'}
											<input
												id={entry.key}
												type="number"
												step="0.01"
												class="h-9 w-32 rounded-md border border-input bg-background px-3 text-sm"
												value={getCurrentValue(entry) ?? ''}
												oninput={(e) => handleInputChange(entry, e)}
											/>
										{:else if entry.value_type === 'select' && entry.options}
											<select
												id={entry.key}
												class="h-9 w-56 rounded-md border border-input bg-background px-3 text-sm"
												value={getCurrentValue(entry) ?? ''}
												onchange={(e) => {
													const target = e.target as HTMLSelectElement;
													handleChange(entry.key, target.value, entry.value);
												}}
											>
												{#each entry.options as option (option)}
													<option
														value={option}
														selected={getCurrentValue(entry) === option}
													>
														{option}
													</option>
												{/each}
											</select>
										{:else}
											<input
												id={entry.key}
												type="text"
												class="h-9 w-56 rounded-md border border-input bg-background px-3 text-sm"
												value={getCurrentValue(entry) ?? ''}
												oninput={(e) => handleInputChange(entry, e)}
											/>
										{/if}

										{#if entry.source === 'database'}
											<Button
												variant="ghost"
												size="icon-sm"
												onclick={() => resetField(entry.key)}
												title="Reset to default"
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
														d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"
													/>
													<path d="M3 3v5h5" />
												</svg>
											</Button>
										{/if}
									</div>
								</div>

								{#if entry.override}
									<div
										class="mt-2 flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400"
									>
										<svg
											class="size-3.5 shrink-0"
											xmlns="http://www.w3.org/2000/svg"
											viewBox="0 0 24 24"
											fill="none"
											stroke="currentColor"
											stroke-width="2"
											stroke-linecap="round"
											stroke-linejoin="round"
										>
											<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z" />
										</svg>
										<span>
											Overridden by
											{#if entry.override.env_var}
												<code
													class="rounded bg-amber-500/10 px-1 py-0.5 font-mono text-[11px]"
													>{entry.override.env_var}</code
												>
											{:else}
												CLI flag
											{/if}
											— effective value: <strong
												>{entry.sensitive
													? '***'
													: String(entry.effective_value)}</strong
											>
										</span>
									</div>
								{/if}
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/each}

		<!-- Action buttons -->
		<div class="flex items-center justify-end gap-3 pb-8">
			{#if hasChanges}
				<Button variant="outline" onclick={cancelChanges} disabled={saving}>Cancel</Button>
			{/if}
			<Button onclick={saveChanges} disabled={!hasChanges || saving}>
				{#if saving}
					Saving...
				{:else}
					Save Changes
				{/if}
			</Button>
		</div>
	{/if}
</div>
