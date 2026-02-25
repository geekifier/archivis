<script lang="ts">
	import { untrack } from 'svelte';
	import { api, ApiError } from '$lib/api/index.js';
	import type { BookDetail, CandidateResponse } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { scoreColor, formatScore, providerColorClass, hasChange, getExcludedFields } from './candidate-utils.js';

	interface Props {
		book: BookDetail;
		candidates: CandidateResponse[];
		onapply: (updated: BookDetail) => void;
		onreject: (candidateId: string) => void;
		onundo: (updated: BookDetail) => void;
	}

	let { book, candidates, onapply, onreject, onundo }: Props = $props();

	let applyingId = $state<string | null>(null);
	let rejectingId = $state<string | null>(null);
	let undoingId = $state<string | null>(null);
	let actionError = $state<string | null>(null);

	/** Per-candidate field selections: candidateId -> fieldName -> included. */
	let fieldSelections = $state<Record<string, Record<string, boolean>>>({});

	const pendingCandidates = $derived(candidates.filter((c) => c.status === 'pending'));
	const rejectedCandidates = $derived(candidates.filter((c) => c.status === 'rejected'));
	const appliedCandidates = $derived(candidates.filter((c) => c.status === 'applied'));
	const hasAppliedCandidate = $derived(appliedCandidates.length > 0);

	/** Initialize default selections for any new pending candidate. */
	$effect(() => {
		for (const candidate of pendingCandidates) {
			if (!untrack(() => fieldSelections[candidate.id])) {
				const sel: Record<string, boolean> = {};
				if (candidate.title != null) sel.title = true;
				if (candidate.authors.length > 0) sel.authors = true;
				if (candidate.publication_date != null) sel.publication_date = true;
				if (candidate.isbn != null) sel.identifiers = true;
				if (candidate.series != null) sel.series = true;
				if (candidate.description != null) sel.description = true;
				if (candidate.cover_url != null) sel.cover = true;
				fieldSelections[candidate.id] = sel;
			}
		}
	});

	function isFieldIncluded(candidateId: string, field: string): boolean {
		return fieldSelections[candidateId]?.[field] ?? true;
	}

	function toggleField(candidateId: string, field: string) {
		if (!fieldSelections[candidateId]) return;
		fieldSelections[candidateId][field] = !fieldSelections[candidateId][field];
	}

	async function handleApply(candidateId: string) {
		applyingId = candidateId;
		actionError = null;
		try {
			const excluded = getExcludedFields(fieldSelections, candidateId);
			const updated = await api.identify.applyCandidate(
				book.id,
				candidateId,
				excluded.length > 0 ? excluded : undefined
			);
			onapply(updated);
		} catch (err) {
			actionError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to apply candidate';
		} finally {
			applyingId = null;
		}
	}

	async function handleReject(candidateId: string) {
		rejectingId = candidateId;
		actionError = null;
		try {
			await api.identify.rejectCandidate(book.id, candidateId);
			onreject(candidateId);
		} catch (err) {
			actionError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to reject candidate';
		} finally {
			rejectingId = null;
		}
	}

	async function handleUndo(candidateId: string) {
		undoingId = candidateId;
		actionError = null;
		try {
			const updated = await api.identify.undoCandidate(book.id, candidateId);
			onundo(updated);
		} catch (err) {
			actionError =
				err instanceof ApiError
					? err.userMessage
					: err instanceof Error
						? err.message
						: 'Failed to undo candidate';
		} finally {
			undoingId = null;
		}
	}

</script>

<div class="space-y-4">
	{#if actionError}
		<div class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
			{actionError}
		</div>
	{/if}

	{#if candidates.length === 0}
		<div class="rounded-lg border border-dashed border-border p-6 text-center">
			<p class="text-sm text-muted-foreground">No candidates found for this book.</p>
		</div>
	{:else}
		<!-- Pending candidates -->
		{#each pendingCandidates as candidate (candidate.id)}
			<div class="rounded-lg border border-border bg-card shadow-sm {hasAppliedCandidate ? 'opacity-50' : ''}">
				<!-- Candidate header -->
				<div class="flex items-center justify-between border-b border-border px-4 py-3">
					<div class="flex items-center gap-2">
						<span
							class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {providerColorClass(candidate.provider_name)}"
						>
							{candidate.provider_name}
						</span>
						<div class="flex items-center gap-2">
							<span class="text-xs font-medium text-muted-foreground">Confidence:</span>
							<div class="flex items-center gap-1.5">
								<div class="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
									<div
										class="h-full rounded-full transition-all {scoreColor(candidate.score)}"
										style="width: {candidate.score * 100}%"
									></div>
								</div>
								<span class="text-xs font-semibold">{formatScore(candidate.score)}</span>
							</div>
						</div>
					</div>
					<div class="flex items-center gap-2">
						{#if hasAppliedCandidate}
							<span class="text-xs text-muted-foreground italic">Another candidate was applied</span>
						{:else}
							<Button
								size="sm"
								variant="outline"
								class="h-7 px-2 text-xs"
								disabled={rejectingId === candidate.id || applyingId !== null}
								onclick={() => handleReject(candidate.id)}
							>
								{#if rejectingId === candidate.id}
									Rejecting...
								{:else}
									Reject
								{/if}
							</Button>
							<Button
								size="sm"
								class="h-7 px-2 text-xs"
								disabled={applyingId === candidate.id || rejectingId !== null}
								onclick={() => handleApply(candidate.id)}
							>
								{#if applyingId === candidate.id}
									Applying...
								{:else}
									Apply
								{/if}
							</Button>
						{/if}
					</div>
				</div>

				<!-- Candidate metadata comparison -->
				<div class="px-4 py-3">
					<!-- Match reasons -->
					{#if candidate.match_reasons.length > 0}
						<div class="mb-3 flex flex-wrap gap-1.5">
							{#each candidate.match_reasons as reason, i (i)}
								<span
									class="inline-flex rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
								>
									{reason}
								</span>
							{/each}
						</div>
					{/if}

					<!-- Side-by-side comparison -->
					<div class="overflow-x-auto">
						<table class="w-full text-sm">
							<thead>
								<tr class="border-b border-border text-left text-xs text-muted-foreground">
									<th class="w-6 pb-2 pr-1"></th>
									<th class="pb-2 pr-3 font-medium">Field</th>
									<th class="pb-2 pr-3 font-medium">Current</th>
									<th class="pb-2 font-medium">Candidate</th>
								</tr>
							</thead>
							<tbody class="divide-y divide-border/50">
								<!-- Title -->
								{#if candidate.title != null}
									<tr class="{!isFieldIncluded(candidate.id, 'title') ? 'opacity-40' : ''}">
										<td class="py-1.5 pr-1">
											<input
												type="checkbox"
												checked={isFieldIncluded(candidate.id, 'title')}
												onchange={() => toggleField(candidate.id, 'title')}
												class="h-3.5 w-3.5 rounded border-border"
											/>
										</td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Title</td>
										<td class="py-1.5 pr-3 text-xs">{book.title}</td>
										<td
											class="py-1.5 text-xs {hasChange(candidate.title, book.title) ? 'font-medium text-primary' : ''}"
										>
											{candidate.title}
										</td>
									</tr>
								{:else}
									<tr>
										<td class="py-1.5 pr-1"></td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Title</td>
										<td class="py-1.5 pr-3 text-xs">{book.title}</td>
										<td class="py-1.5 text-xs">--</td>
									</tr>
								{/if}
								<!-- Authors -->
								{#if candidate.authors.length > 0}
									<tr class="{!isFieldIncluded(candidate.id, 'authors') ? 'opacity-40' : ''}">
										<td class="py-1.5 pr-1">
											<input
												type="checkbox"
												checked={isFieldIncluded(candidate.id, 'authors')}
												onchange={() => toggleField(candidate.id, 'authors')}
												class="h-3.5 w-3.5 rounded border-border"
											/>
										</td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Authors</td>
										<td class="py-1.5 pr-3 text-xs">
											{book.authors.map((a) => a.name).join(', ') || '--'}
										</td>
										<td
											class="py-1.5 text-xs {candidate.authors.join(', ') !== book.authors.map((a) => a.name).join(', ') ? 'font-medium text-primary' : ''}"
										>
											{candidate.authors.join(', ')}
										</td>
									</tr>
								{:else}
									<tr>
										<td class="py-1.5 pr-1"></td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Authors</td>
										<td class="py-1.5 pr-3 text-xs">
											{book.authors.map((a) => a.name).join(', ') || '--'}
										</td>
										<td class="py-1.5 text-xs">--</td>
									</tr>
								{/if}
								<!-- Publisher (no checkbox — backend doesn't apply publisher) -->
								{#if candidate.publisher || book.publisher_name}
									<tr>
										<td class="py-1.5 pr-1"></td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground"
											>Publisher</td
										>
										<td class="py-1.5 pr-3 text-xs">{book.publisher_name ?? '--'}</td>
										<td
											class="py-1.5 text-xs {hasChange(candidate.publisher, book.publisher_name) ? 'font-medium text-primary' : ''}"
										>
											{candidate.publisher ?? '--'}
										</td>
									</tr>
								{/if}
								<!-- Publication Date -->
								{#if candidate.publication_date || book.publication_date}
									<tr class="{candidate.publication_date != null && !isFieldIncluded(candidate.id, 'publication_date') ? 'opacity-40' : ''}">
										{#if candidate.publication_date != null}
											<td class="py-1.5 pr-1">
												<input
													type="checkbox"
													checked={isFieldIncluded(candidate.id, 'publication_date')}
													onchange={() => toggleField(candidate.id, 'publication_date')}
													class="h-3.5 w-3.5 rounded border-border"
												/>
											</td>
										{:else}
											<td class="py-1.5 pr-1"></td>
										{/if}
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground"
											>Published</td
										>
										<td class="py-1.5 pr-3 text-xs">{book.publication_date ?? '--'}</td>
										<td
											class="py-1.5 text-xs {hasChange(candidate.publication_date, book.publication_date) ? 'font-medium text-primary' : ''}"
										>
											{candidate.publication_date ?? '--'}
										</td>
									</tr>
								{/if}
								<!-- ISBN -->
								{#if candidate.isbn}
									<tr class="{!isFieldIncluded(candidate.id, 'identifiers') ? 'opacity-40' : ''}">
										<td class="py-1.5 pr-1">
											<input
												type="checkbox"
												checked={isFieldIncluded(candidate.id, 'identifiers')}
												onchange={() => toggleField(candidate.id, 'identifiers')}
												class="h-3.5 w-3.5 rounded border-border"
											/>
										</td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">ISBN</td>
										<td class="py-1.5 pr-3 font-mono text-xs">
											{book.identifiers.find((i) => i.identifier_type === 'isbn13' || i.identifier_type === 'isbn10')?.value ?? '--'}
										</td>
										<td class="py-1.5 font-mono text-xs font-medium text-primary">
											{candidate.isbn}
										</td>
									</tr>
								{/if}
								<!-- Series -->
								{#if candidate.series}
									<tr class="{!isFieldIncluded(candidate.id, 'series') ? 'opacity-40' : ''}">
										<td class="py-1.5 pr-1">
											<input
												type="checkbox"
												checked={isFieldIncluded(candidate.id, 'series')}
												onchange={() => toggleField(candidate.id, 'series')}
												class="h-3.5 w-3.5 rounded border-border"
											/>
										</td>
										<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Series</td>
										<td class="py-1.5 pr-3 text-xs">
											{book.series.length > 0 ? book.series.map((s) => s.position != null ? `${s.name} #${s.position}` : s.name).join(', ') : '--'}
										</td>
										<td class="py-1.5 text-xs font-medium text-primary">
											{candidate.series.name}{candidate.series.position != null ? ` #${candidate.series.position}` : ''}
										</td>
									</tr>
								{/if}
								<!-- Description (show truncated if present) -->
								{#if candidate.description}
									<tr class="{!isFieldIncluded(candidate.id, 'description') ? 'opacity-40' : ''}">
										<td class="py-1.5 pr-1 align-top">
											<input
												type="checkbox"
												checked={isFieldIncluded(candidate.id, 'description')}
												onchange={() => toggleField(candidate.id, 'description')}
												class="mt-0.5 h-3.5 w-3.5 rounded border-border"
											/>
										</td>
										<td class="py-1.5 pr-3 align-top text-xs font-medium text-muted-foreground"
											>Description</td
										>
										<td class="py-1.5 pr-3 text-xs">
											{#if book.description}
												<span class="line-clamp-2">{book.description}</span>
											{:else}
												--
											{/if}
										</td>
										<td
											class="py-1.5 text-xs {hasChange(candidate.description, book.description) ? 'font-medium text-primary' : ''}"
										>
											<span class="line-clamp-2">{candidate.description}</span>
										</td>
									</tr>
								{/if}
							</tbody>
						</table>
					</div>

					<!-- Cover preview -->
					{#if candidate.cover_url}
						<div class="mt-3 flex items-start gap-3 border-t border-border pt-3 {!isFieldIncluded(candidate.id, 'cover') ? 'opacity-40' : ''}">
							<input
								type="checkbox"
								checked={isFieldIncluded(candidate.id, 'cover')}
								onchange={() => toggleField(candidate.id, 'cover')}
								class="mt-0.5 h-3.5 w-3.5 rounded border-border"
							/>
							<span class="text-xs font-medium text-muted-foreground">Cover:</span>
							<img
								src={candidate.cover_url}
								alt="Candidate cover"
								class="h-20 rounded object-contain shadow-sm"
							/>
						</div>
					{/if}
				</div>
			</div>
		{/each}

		<!-- Applied candidates -->
		{#if appliedCandidates.length > 0}
			<div class="space-y-2">
				<h4 class="text-xs font-medium text-muted-foreground">Applied</h4>
				{#each appliedCandidates as candidate (candidate.id)}
					<div class="rounded-lg border border-green-200 bg-green-50/50 px-4 py-2.5 dark:border-green-900/30 dark:bg-green-900/10">
						<div class="flex items-center justify-between">
							<div class="flex items-center gap-2">
								<span
									class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {providerColorClass(candidate.provider_name)}"
								>
									{candidate.provider_name}
								</span>
								<span class="text-xs text-muted-foreground">
									{formatScore(candidate.score)} confidence
								</span>
								<span class="inline-flex rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-800 dark:bg-green-900/30 dark:text-green-400">
									Applied
								</span>
							</div>
							<Button
								size="sm"
								variant="outline"
								class="h-7 px-2 text-xs"
								disabled={undoingId === candidate.id}
								onclick={() => handleUndo(candidate.id)}
							>
								{#if undoingId === candidate.id}
									Undoing...
								{:else}
									Undo
								{/if}
							</Button>
						</div>
					</div>
				{/each}
			</div>
		{/if}

		<!-- Rejected candidates (collapsed) -->
		{#if rejectedCandidates.length > 0}
			<div class="space-y-2">
				<h4 class="text-xs font-medium text-muted-foreground">
					Rejected ({rejectedCandidates.length})
				</h4>
				{#each rejectedCandidates as candidate (candidate.id)}
					<div class="rounded-lg border border-border bg-muted/30 px-4 py-2.5 opacity-60">
						<div class="flex items-center gap-2">
							<span
								class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {providerColorClass(candidate.provider_name)}"
							>
								{candidate.provider_name}
							</span>
							<span class="text-xs text-muted-foreground">
								{candidate.title ?? 'No title'}
							</span>
							<span class="text-xs text-muted-foreground">
								{formatScore(candidate.score)}
							</span>
						</div>
					</div>
				{/each}
			</div>
		{/if}
	{/if}
</div>
