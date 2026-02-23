<script lang="ts">
	import type { BookSummary } from '$lib/api/index.js';
	import { placeholderHue } from '$lib/utils.js';

	interface Props {
		book: BookSummary;
		selectionMode?: boolean;
		selected?: boolean;
		onselect?: (bookId: string, event: MouseEvent) => void;
	}

	let { book, selectionMode = false, selected = false, onselect }: Props = $props();

	let coverLoaded = $state(false);

	const authors = $derived(
		book.authors?.map((a) => a.name).join(', ') ?? ''
	);

	const coverSm = $derived(`/api/books/${book.id}/cover?size=sm`);
	const coverMd = $derived(`/api/books/${book.id}/cover?size=md`);

	const hue = $derived(placeholderHue(book.id));

	/** Primary format badge extracted from the first file. */
	const formatLabel = $derived(
		book.files && book.files.length > 0
			? book.files[0].format.toUpperCase()
			: null
	);

	/** Status indicator: only shown for non-identified books. */
	const statusIndicator = $derived<{ color: string; title: string } | null>(
		book.metadata_status === 'needs_review'
			? { color: 'bg-amber-500', title: 'Needs review' }
			: book.metadata_status === 'unidentified'
				? { color: 'bg-red-500', title: 'Unidentified' }
				: null
	);

	function handleClick(e: MouseEvent) {
		if (selectionMode && onselect) {
			e.preventDefault();
			onselect(book.id, e);
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (selectionMode && onselect && (e.key === 'Enter' || e.key === ' ')) {
			e.preventDefault();
			// Create a synthetic MouseEvent for keyboard-triggered selection (no shift key)
			onselect(book.id, new MouseEvent('click'));
		}
	}
</script>

{#if selectionMode}
	<button
		type="button"
		class="group block w-full rounded-lg text-left transition-shadow focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {selected ? 'ring-2 ring-primary shadow-md' : 'hover:shadow-md'}"
		onclick={handleClick}
		onkeydown={handleKeydown}
	>
		<div class="relative aspect-[2/3] w-full overflow-hidden rounded-lg bg-muted">
			{#if book.has_cover}
				{#if !coverLoaded}
					<div class="absolute inset-0 animate-pulse bg-muted"></div>
				{/if}
				<img
					src={coverSm}
					srcset="{coverSm} 1x, {coverMd} 2x"
					alt="Cover of {book.title}"
					loading="lazy"
					onload={() => (coverLoaded = true)}
					class="absolute inset-0 h-full w-full object-cover transition-opacity duration-200 {coverLoaded ? 'opacity-100' : 'opacity-0'}"
				/>
			{:else}
				<div
					class="flex h-full w-full items-center justify-center p-3"
					style="background-color: hsl({hue}, 30%, 25%);"
				>
					<span class="line-clamp-4 text-center text-sm font-medium text-white/80">
						{book.title}
					</span>
				</div>
			{/if}
			<!-- Selection checkbox overlay -->
			<div class="absolute top-1.5 left-1.5 flex items-center justify-center">
				<div
					class="flex size-5 items-center justify-center rounded border-2 bg-background/80 backdrop-blur-sm transition-colors {selected ? 'border-primary bg-primary' : 'border-muted-foreground/50'}"
				>
					{#if selected}
						<svg class="size-3.5 text-primary-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
							<polyline points="20 6 9 17 4 12" />
						</svg>
					{/if}
				</div>
			</div>
			{#if formatLabel}
				<span class="absolute bottom-1.5 right-1.5 rounded bg-black/60 px-1.5 py-0.5 text-[10px] font-semibold uppercase leading-none text-white/90 backdrop-blur-sm">
					{formatLabel}
				</span>
			{/if}
			{#if statusIndicator}
				<span
					class="absolute top-1.5 right-1.5 size-2.5 rounded-full ring-2 ring-background {statusIndicator.color}"
					title={statusIndicator.title}
				></span>
			{/if}
		</div>

		<div class="mt-1.5 px-0.5">
			<p class="line-clamp-2 text-sm font-medium leading-tight group-hover:text-primary">
				{book.title}
			</p>
			{#if authors}
				<p class="line-clamp-1 text-xs text-muted-foreground">{authors}</p>
			{/if}
		</div>
	</button>
{:else}
	<a
		href="/books/{book.id}"
		class="group block rounded-lg transition-shadow hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
	>
		<div class="relative aspect-[2/3] w-full overflow-hidden rounded-lg bg-muted">
			{#if book.has_cover}
				{#if !coverLoaded}
					<div class="absolute inset-0 animate-pulse bg-muted"></div>
				{/if}
				<img
					src={coverSm}
					srcset="{coverSm} 1x, {coverMd} 2x"
					alt="Cover of {book.title}"
					loading="lazy"
					onload={() => (coverLoaded = true)}
					class="absolute inset-0 h-full w-full object-cover transition-opacity duration-200 {coverLoaded ? 'opacity-100' : 'opacity-0'}"
				/>
			{:else}
				<div
					class="flex h-full w-full items-center justify-center p-3"
					style="background-color: hsl({hue}, 30%, 25%);"
				>
					<span class="line-clamp-4 text-center text-sm font-medium text-white/80">
						{book.title}
					</span>
				</div>
			{/if}
			{#if formatLabel}
				<span class="absolute bottom-1.5 right-1.5 rounded bg-black/60 px-1.5 py-0.5 text-[10px] font-semibold uppercase leading-none text-white/90 backdrop-blur-sm">
					{formatLabel}
				</span>
			{/if}
			{#if statusIndicator}
				<span
					class="absolute top-1.5 right-1.5 size-2.5 rounded-full ring-2 ring-background {statusIndicator.color}"
					title={statusIndicator.title}
				></span>
			{/if}
		</div>

		<div class="mt-1.5 px-0.5">
			<p class="line-clamp-2 text-sm font-medium leading-tight group-hover:text-primary">
				{book.title}
			</p>
			{#if authors}
				<p class="line-clamp-1 text-xs text-muted-foreground">{authors}</p>
			{/if}
		</div>
	</a>
{/if}
