<script lang="ts">
	interface Props {
		src: string;
		alt: string;
		srcset?: string;
		loading?: 'lazy' | 'eager';
		blurFill?: boolean;
		fadeIn?: boolean;
		onload?: () => void;
		onerror?: () => void;
	}

	let {
		src,
		alt,
		srcset,
		loading,
		blurFill = true,
		fadeIn = false,
		onload,
		onerror
	}: Props = $props();

	let loaded = $state(false);

	$effect(() => {
		void src;
		loaded = false;
	});

	function handleLoad() {
		loaded = true;
		onload?.();
	}

	function handleError() {
		onerror?.();
	}
</script>

{#if blurFill}
	<!-- Blur-fill mode: blurred bg + sharp contain fg -->
	{#if fadeIn && !loaded}
		<div class="absolute inset-0 animate-pulse bg-muted"></div>
	{/if}
	<img
		{src}
		{srcset}
		{loading}
		alt=""
		aria-hidden="true"
		onload={handleLoad}
		onerror={handleError}
		class="absolute inset-0 h-full w-full scale-125 object-cover blur-xl {fadeIn ? `transition-opacity duration-200 ${loaded ? 'opacity-100' : 'opacity-0'}` : ''}"
	/>
	<img
		{src}
		{srcset}
		{loading}
		{alt}
		class="absolute inset-[5%] h-[90%] w-[90%] object-contain drop-shadow-lg {fadeIn ? `transition-opacity duration-200 ${loaded ? 'opacity-100' : 'opacity-0'}` : ''}"
	/>
{:else}
	<!-- Thumbnail mode: single object-cover image -->
	<img
		{src}
		{loading}
		{alt}
		onload={handleLoad}
		onerror={handleError}
		class="h-full w-full object-cover {fadeIn ? `transition-opacity duration-200 ${loaded ? 'opacity-100' : 'opacity-0'}` : ''}"
	/>
{/if}
