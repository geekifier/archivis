<script lang="ts">
	import { api } from '$lib/api/index.js';
	import type { BookDetail } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';

	interface Props {
		bookId: string;
		hasCover: boolean;
		open: boolean;
		onupdate: (updated: BookDetail) => void;
	}

	let { bookId, hasCover, open = $bindable(), onupdate }: Props = $props();

	let uploadingCover = $state(false);
	let coverUploadError = $state<string | null>(null);
	let coverFileInput = $state<HTMLInputElement | null>(null);
	let dragging = $state(false);

	function triggerCoverUpload() {
		coverFileInput?.click();
	}

	async function uploadFile(file: File) {
		uploadingCover = true;
		coverUploadError = null;
		try {
			const updated = await api.books.uploadCover(bookId, file);
			onupdate(updated);
			open = false;
		} catch (err) {
			coverUploadError = err instanceof Error ? err.message : 'Failed to upload cover';
		} finally {
			uploadingCover = false;
		}
	}

	function handleFileInput(event: Event) {
		const input = event.target as HTMLInputElement;
		const file = input.files?.[0];
		if (file) uploadFile(file);
		// Reset the input so the same file can be selected again
		input.value = '';
	}

	function handleDrop(event: DragEvent) {
		event.preventDefault();
		dragging = false;
		const file = event.dataTransfer?.files[0];
		if (file) uploadFile(file);
	}

	function handleDragOver(event: DragEvent) {
		event.preventDefault();
		dragging = true;
	}

	function handleDragLeave() {
		dragging = false;
	}

	function handleOpenChange(isOpen: boolean) {
		open = isOpen;
		if (!isOpen) {
			coverUploadError = null;
			dragging = false;
		}
	}
</script>

<!-- Hidden file input for cover upload -->
<input
	type="file"
	accept="image/jpeg,image/png,image/webp"
	class="hidden"
	bind:this={coverFileInput}
	onchange={handleFileInput}
/>

<Dialog.Root open={open} onOpenChange={handleOpenChange}>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>{hasCover ? 'Change Cover' : 'Add Cover'}</Dialog.Title>
			<Dialog.Description>
				The cover image will be updated immediately. This is separate from any pending form
				edits.
			</Dialog.Description>
		</Dialog.Header>
		<div
			class="flex flex-col items-center gap-3 rounded-lg border-2 border-dashed px-4 py-6 transition-colors {dragging
				? 'border-primary bg-primary/5'
				: 'border-muted-foreground/25'}"
			ondrop={handleDrop}
			ondragover={handleDragOver}
			ondragleave={handleDragLeave}
			role="presentation"
		>
			<svg
				class="size-8 text-muted-foreground/50"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				stroke-width="1.5"
				stroke-linecap="round"
				stroke-linejoin="round"
			>
				<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
				<polyline points="17 8 12 3 7 8" />
				<line x1="12" x2="12" y1="3" y2="15" />
			</svg>
			<p class="text-sm text-muted-foreground">
				Drag and drop an image here, or
			</p>
			<Button onclick={triggerCoverUpload} disabled={uploadingCover} size="sm">
				{#if uploadingCover}
					Uploading...
				{:else}
					Choose File
				{/if}
			</Button>
			{#if coverUploadError}
				<p class="text-sm text-destructive">{coverUploadError}</p>
			{/if}
		</div>
		<Dialog.Footer>
			<Dialog.Close disabled={uploadingCover}>Cancel</Dialog.Close>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
