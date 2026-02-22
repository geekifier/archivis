<script lang="ts">
	import { Button } from '$lib/components/ui/button/index.js';

	interface Props {
		page: number;
		totalPages: number;
		onPageChange: (page: number) => void;
	}

	let { page, totalPages, onPageChange }: Props = $props();

	/** Build a truncated list of page numbers with ellipsis gaps. */
	const visiblePages = $derived.by(() => {
		if (totalPages <= 7) {
			return Array.from({ length: totalPages }, (_, i) => i + 1);
		}

		const pages: (number | '...')[] = [1];

		if (page > 3) {
			pages.push('...');
		}

		const start = Math.max(2, page - 1);
		const end = Math.min(totalPages - 1, page + 1);

		for (let i = start; i <= end; i++) {
			pages.push(i);
		}

		if (page < totalPages - 2) {
			pages.push('...');
		}

		pages.push(totalPages);
		return pages;
	});
</script>

{#if totalPages > 1}
	<nav aria-label="Pagination" class="flex items-center justify-center gap-1">
		<Button
			variant="outline"
			size="sm"
			disabled={page <= 1}
			onclick={() => onPageChange(page - 1)}
		>
			Previous
		</Button>

		{#each visiblePages as item, i (i)}
			{#if item === '...'}
				<span class="px-2 text-sm text-muted-foreground">...</span>
			{:else}
				<Button
					variant={item === page ? 'default' : 'outline'}
					size="sm"
					onclick={() => onPageChange(item)}
					aria-current={item === page ? 'page' : undefined}
				>
					{item}
				</Button>
			{/if}
		{/each}

		<Button
			variant="outline"
			size="sm"
			disabled={page >= totalPages}
			onclick={() => onPageChange(page + 1)}
		>
			Next
		</Button>
	</nav>
{/if}
