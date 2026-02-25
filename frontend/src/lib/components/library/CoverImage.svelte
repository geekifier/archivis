<script lang="ts">
	interface Props {
		src: string;
		alt: string;
		srcset?: string;
		loading?: 'lazy' | 'eager';
		fadeIn?: boolean;
		onload?: () => void;
		onerror?: () => void;
	}

	let {
		src,
		alt,
		srcset,
		loading,
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

<img
	{src}
	{srcset}
	{loading}
	{alt}
	onload={handleLoad}
	onerror={handleError}
	class="block w-full aspect-[2/3] {fadeIn ? `transition-opacity duration-200 ${loaded ? 'opacity-100' : 'opacity-0'}` : ''}"
/>
