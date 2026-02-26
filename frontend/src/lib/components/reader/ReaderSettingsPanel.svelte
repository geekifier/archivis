<script lang="ts">
	import { reader, type ReaderPreferences } from '$lib/stores/reader.svelte.js';
	import { Button } from '$lib/components/ui/button/index.js';

	interface Props {
		open: boolean;
		onClose: () => void;
	}

	let { open, onClose }: Props = $props();

	const prefs = $derived(reader.preferences);

	function update<K extends keyof ReaderPreferences>(key: K, value: ReaderPreferences[K]): void {
		reader.updatePreference(key, value);
	}

	function handleOverlayKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			onClose();
		}
	}

	// Font size helpers
	function decreaseFontSize(): void {
		const next = Math.max(80, prefs.fontSize - 10);
		update('fontSize', next);
	}

	function increaseFontSize(): void {
		const next = Math.min(200, prefs.fontSize + 10);
		update('fontSize', next);
	}

	// Font family options
	const fontFamilies: Array<{ value: string; label: string }> = [
		{ value: 'default', label: 'Default' },
		{ value: 'serif', label: 'Serif' },
		{ value: 'sans-serif', label: 'Sans' },
		{ value: 'monospace', label: 'Mono' }
	];

	// Theme options
	const themeOptions: Array<{ value: ReaderPreferences['theme']; label: string; swatch: string; border: boolean }> = [
		{ value: 'light', label: 'Light', swatch: '#ffffff', border: true },
		{ value: 'dark', label: 'Dark', swatch: '#1a1a1a', border: false },
		{ value: 'sepia', label: 'Sepia', swatch: '#f4ecd8', border: false }
	];

	// Flow options
	const flowOptions: Array<{ value: ReaderPreferences['flow']; label: string }> = [
		{ value: 'paginated', label: 'Paginated' },
		{ value: 'scrolled', label: 'Scrolled' }
	];

	// Column options
	const columnOptions: Array<{ value: number; label: string }> = [
		{ value: 1, label: '1' },
		{ value: 2, label: '2' }
	];
</script>

{#if open}
	<!-- Background overlay -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed inset-0 z-40 bg-black/40"
		onclick={onClose}
		onkeydown={handleOverlayKeydown}
	></div>

	<!-- Settings panel (bottom sheet on mobile, side panel on desktop) -->
	<div
		class="fixed inset-x-0 bottom-0 z-50 flex max-h-[80vh] flex-col rounded-t-xl border-t border-border bg-background shadow-lg sm:inset-x-auto sm:inset-y-0 sm:right-0 sm:max-h-none sm:w-80 sm:rounded-none sm:border-l sm:border-t-0"
	>
		<!-- Header -->
		<div class="flex items-center justify-between border-b border-border px-4 py-3">
			<h2 class="text-sm font-semibold">Reader Settings</h2>
			<button
				onclick={onClose}
				class="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
				aria-label="Close settings"
			>
				<svg
					class="size-4"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<path d="M18 6 6 18" />
					<path d="m6 6 12 12" />
				</svg>
			</button>
		</div>

		<!-- Settings controls -->
		<div class="flex-1 space-y-5 overflow-y-auto px-4 py-4">
			<!-- 1. Font Size -->
			<div class="space-y-2">
				<div class="flex items-center justify-between">
					<label for="reader-font-size" class="text-xs font-medium text-muted-foreground">Font Size</label>
					<span class="text-xs tabular-nums text-foreground">{prefs.fontSize}%</span>
				</div>
				<div class="flex items-center gap-2">
					<Button
						variant="outline"
						size="icon-sm"
						onclick={decreaseFontSize}
						disabled={prefs.fontSize <= 80}
						aria-label="Decrease font size"
					>
						<svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M5 12h14" />
						</svg>
					</Button>
					<input
						id="reader-font-size"
						type="range"
						min="80"
						max="200"
						step="10"
						value={prefs.fontSize}
						oninput={(e) => update('fontSize', Number(e.currentTarget.value))}
						class="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-border accent-primary [&::-moz-range-thumb]:size-3.5 [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-primary [&::-webkit-slider-thumb]:size-3.5 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-primary"
					/>
					<Button
						variant="outline"
						size="icon-sm"
						onclick={increaseFontSize}
						disabled={prefs.fontSize >= 200}
						aria-label="Increase font size"
					>
						<svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<path d="M5 12h14" />
							<path d="M12 5v14" />
						</svg>
					</Button>
				</div>
			</div>

			<!-- 2. Font Family -->
			<div class="space-y-2">
				<span class="text-xs font-medium text-muted-foreground">Font Family</span>
				<div class="flex gap-1">
					{#each fontFamilies as ff (ff.value)}
						<Button
							variant={prefs.fontFamily === ff.value ? 'default' : 'outline'}
							size="sm"
							class="flex-1 text-xs"
							onclick={() => update('fontFamily', ff.value)}
						>
							{ff.label}
						</Button>
					{/each}
				</div>
			</div>

			<!-- 3. Line Height -->
			<div class="space-y-2">
				<div class="flex items-center justify-between">
					<label for="reader-line-height" class="text-xs font-medium text-muted-foreground">Line Height</label>
					<span class="text-xs tabular-nums text-foreground">{prefs.lineHeight.toFixed(1)}</span>
				</div>
				<input
					id="reader-line-height"
					type="range"
					min="1.0"
					max="2.5"
					step="0.1"
					value={prefs.lineHeight}
					oninput={(e) => update('lineHeight', Number(e.currentTarget.value))}
					class="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-border accent-primary [&::-moz-range-thumb]:size-3.5 [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-primary [&::-webkit-slider-thumb]:size-3.5 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-primary"
				/>
			</div>

			<!-- 4. Theme -->
			<div class="space-y-2">
				<span class="text-xs font-medium text-muted-foreground">Theme</span>
				<div class="flex gap-3">
					{#each themeOptions as t (t.value)}
						<button
							onclick={() => update('theme', t.value)}
							class="flex flex-col items-center gap-1.5"
							aria-label="{t.label} theme"
						>
							<span
								class="inline-flex size-8 items-center justify-center rounded-full {prefs.theme === t.value ? 'ring-2 ring-primary ring-offset-2 ring-offset-background' : ''} {t.border ? 'border border-border' : ''}"
								style:background-color={t.swatch}
							></span>
							<span class="text-xs {prefs.theme === t.value ? 'font-medium text-foreground' : 'text-muted-foreground'}">{t.label}</span>
						</button>
					{/each}
				</div>
			</div>

			<!-- 5. Layout (Flow) -->
			<div class="space-y-2">
				<span class="text-xs font-medium text-muted-foreground">Layout</span>
				<div class="flex gap-1">
					{#each flowOptions as fo (fo.value)}
						<Button
							variant={prefs.flow === fo.value ? 'default' : 'outline'}
							size="sm"
							class="flex-1 text-xs"
							onclick={() => update('flow', fo.value)}
						>
							{fo.label}
						</Button>
					{/each}
				</div>
			</div>

			<!-- 6. Margins -->
			<div class="space-y-2">
				<div class="flex items-center justify-between">
					<label for="reader-margins" class="text-xs font-medium text-muted-foreground">Margins</label>
					<span class="text-xs tabular-nums text-foreground">{prefs.margins}px</span>
				</div>
				<input
					id="reader-margins"
					type="range"
					min="0"
					max="100"
					step="5"
					value={prefs.margins}
					oninput={(e) => update('margins', Number(e.currentTarget.value))}
					class="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-border accent-primary [&::-moz-range-thumb]:size-3.5 [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-primary [&::-webkit-slider-thumb]:size-3.5 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-primary"
				/>
			</div>

			<!-- 7. Max Width -->
			<div class="space-y-2">
				<div class="flex items-center justify-between">
					<label for="reader-max-width" class="text-xs font-medium text-muted-foreground">Max Width</label>
					<span class="text-xs tabular-nums text-foreground">{prefs.maxWidth}px</span>
				</div>
				<input
					id="reader-max-width"
					type="range"
					min="600"
					max="1200"
					step="50"
					value={prefs.maxWidth}
					oninput={(e) => update('maxWidth', Number(e.currentTarget.value))}
					class="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-border accent-primary [&::-moz-range-thumb]:size-3.5 [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-primary [&::-webkit-slider-thumb]:size-3.5 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-primary"
				/>
			</div>

			<!-- 8. Columns (only visible when paginated) -->
			{#if prefs.flow === 'paginated'}
				<div class="space-y-2">
					<span class="text-xs font-medium text-muted-foreground">Columns</span>
					<div class="flex gap-1">
						{#each columnOptions as co (co.value)}
							<Button
								variant={prefs.maxColumns === co.value ? 'default' : 'outline'}
								size="sm"
								class="flex-1 text-xs"
								onclick={() => update('maxColumns', co.value)}
							>
								{co.label}
							</Button>
						{/each}
					</div>
				</div>
			{/if}
		</div>
	</div>
{/if}
