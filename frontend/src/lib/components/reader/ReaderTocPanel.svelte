<script lang="ts">
	import type { TocItem } from '$lib/api/types.js';

	interface Props {
		toc: TocItem[];
		currentHref: string | null;
		open: boolean;
		onClose: () => void;
		onNavigate: (href: string) => void;
	}

	let { toc, currentHref, open, onClose, onNavigate }: Props = $props();

	function handleItemClick(href: string): void {
		onNavigate(href);
		onClose();
	}

	function handleOverlayKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			onClose();
		}
	}

	function isCurrent(href: string): boolean {
		if (!currentHref) return false;
		return currentHref === href;
	}
</script>

{#snippet tocItems(items: TocItem[], level: number)}
	{#each items as item (item.href + item.label)}
		<button
			onclick={() => handleItemClick(item.href)}
			class="w-full text-left text-sm transition-colors hover:bg-accent {isCurrent(item.href) ? 'bg-primary/10 font-medium text-primary' : 'text-foreground'}"
			style:padding-left="{1 + level * 0.75}rem"
			style:padding-right="1rem"
			style:padding-top="0.625rem"
			style:padding-bottom="0.625rem"
			style:min-height="44px"
		>
			{item.label}
		</button>
		{#if item.subitems && item.subitems.length > 0}
			{@render tocItems(item.subitems, level + 1)}
		{/if}
	{/each}
{/snippet}

{#if open}
	<!-- Background overlay -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed inset-0 z-40 bg-black/40"
		onclick={onClose}
		onkeydown={handleOverlayKeydown}
	></div>

	<!-- TOC panel -->
	<div
		class="fixed inset-0 z-50 flex flex-col bg-background shadow-lg sm:bottom-0 sm:left-0 sm:right-auto sm:top-0 sm:w-80 sm:border-r sm:border-border"
	>
		<!-- Header -->
		<div class="flex items-center justify-between border-b border-border px-4 py-3">
			<h2 class="text-sm font-semibold">Table of Contents</h2>
			<button
				onclick={onClose}
				class="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
				aria-label="Close table of contents"
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

		<!-- TOC items -->
		<div class="flex-1 overflow-y-auto">
			{#if toc.length === 0}
				<div class="px-4 py-8 text-center text-sm text-muted-foreground">
					No table of contents available
				</div>
			{:else}
				{@render tocItems(toc, 0)}
			{/if}
		</div>
	</div>
{/if}
