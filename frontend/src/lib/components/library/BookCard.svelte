<script lang="ts">
	import type { BookSummary } from '$lib/api/index.js';

	interface Props {
		book: BookSummary;
	}

	let { book }: Props = $props();

	let coverLoaded = $state(false);

	const authors = $derived(
		book.authors?.map((a) => a.name).join(', ') ?? ''
	);

	const coverSm = $derived(`/api/books/${book.id}/cover?size=sm`);
	const coverMd = $derived(`/api/books/${book.id}/cover?size=md`);

	/** Generate a deterministic hue from book ID for the placeholder. */
	function placeholderHue(id: string): number {
		let hash = 0;
		for (let i = 0; i < id.length; i++) {
			hash = (hash * 31 + id.charCodeAt(i)) | 0;
		}
		return Math.abs(hash) % 360;
	}

	const hue = $derived(placeholderHue(book.id));
</script>

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
