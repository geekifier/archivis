<script lang="ts">
	import { Button } from '$lib/components/ui/button/index.js';
</script>

<div class="space-y-6">
	<div>
		<h1 class="text-3xl font-bold tracking-tight">Library</h1>
		<p class="text-muted-foreground">Your e-book collection</p>
	</div>

	<div class="flex items-center justify-center rounded-lg border border-dashed border-border p-12">
		<div class="text-center">
			<p class="text-muted-foreground">No books in your library yet.</p>
			<Button variant="outline" class="mt-4" href="/import">Import books</Button>
		</div>
	</div>
</div>
