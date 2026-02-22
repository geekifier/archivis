<script lang="ts">
	import { api, ApiError } from '$lib/api/index.js';
	import type { BookDetail, CandidateResponse } from '$lib/api/index.js';
	import { Button } from '$lib/components/ui/button/index.js';

	interface Props {
		book: BookDetail;
		candidates: CandidateResponse[];
		onapply: (updated: BookDetail) => void;
		onreject: (candidateId: string) => void;
	}

	let { book, candidates, onapply, onreject }: Props = $props();

	let applyingId = $state<string | null>(null);
	let rejectingId = $state<string | null>(null);
	let actionError = $state<string | null>(null);

	const pendingCandidates = $derived(candidates.filter((c) => c.status === 'pending'));
	const rejectedCandidates = $derived(candidates.filter((c) => c.status === 'rejected'));
	const appliedCandidates = $derived(candidates.filter((c) => c.status === 'applied'));

	async function handleApply(candidateId: string) {
		applyingId = candidateId;
		actionError = null;
		try {
			const updated = await api.identify.applyCandidate(book.id, candidateId);
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

	function providerColorClass(provider: string): string {
		const lower = provider.toLowerCase();
		if (lower.includes('open library'))
			return 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400';
		if (lower.includes('hardcover'))
			return 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400';
		return 'bg-muted text-muted-foreground';
	}

	function scoreColor(score: number): string {
		if (score >= 0.8) return 'bg-green-500';
		if (score >= 0.5) return 'bg-amber-500';
		return 'bg-red-500';
	}

	function formatScore(score: number): string {
		return `${Math.round(score * 100)}%`;
	}

	function hasChange(
		candidateValue: string | undefined | null,
		bookValue: string | undefined | null
	): boolean {
		const cv = candidateValue ?? '';
		const bv = bookValue ?? '';
		return cv !== '' && cv !== bv;
	}
</script>

<div class="space-y-4">
	<div class="flex items-center justify-between">
		<h3 class="text-sm font-semibold text-muted-foreground">
			Identification Candidates
			{#if pendingCandidates.length > 0}
				<span class="ml-1 text-xs font-normal">({pendingCandidates.length} pending)</span>
			{/if}
		</h3>
	</div>

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
			<div class="rounded-lg border border-border bg-card shadow-sm">
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
					</div>
				</div>

				<!-- Candidate metadata comparison -->
				<div class="px-4 py-3">
					<!-- Match reasons -->
					{#if candidate.match_reasons.length > 0}
						<div class="mb-3 flex flex-wrap gap-1.5">
							{#each candidate.match_reasons as reason (reason)}
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
									<th class="pb-2 pr-3 font-medium">Field</th>
									<th class="pb-2 pr-3 font-medium">Current</th>
									<th class="pb-2 font-medium">Candidate</th>
								</tr>
							</thead>
							<tbody class="divide-y divide-border/50">
								<!-- Title -->
								<tr>
									<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Title</td>
									<td class="py-1.5 pr-3 text-xs">{book.title}</td>
									<td
										class="py-1.5 text-xs {hasChange(candidate.title, book.title) ? 'font-medium text-primary' : ''}"
									>
										{candidate.title ?? '--'}
									</td>
								</tr>
								<!-- Authors -->
								<tr>
									<td class="py-1.5 pr-3 text-xs font-medium text-muted-foreground">Authors</td>
									<td class="py-1.5 pr-3 text-xs">
										{book.authors.map((a) => a.name).join(', ') || '--'}
									</td>
									<td
										class="py-1.5 text-xs {candidate.authors.length > 0 && candidate.authors.join(', ') !== book.authors.map((a) => a.name).join(', ') ? 'font-medium text-primary' : ''}"
									>
										{candidate.authors.join(', ') || '--'}
									</td>
								</tr>
								<!-- Publisher -->
								{#if candidate.publisher || book.publisher_name}
									<tr>
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
									<tr>
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
									<tr>
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
									<tr>
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
									<tr>
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
						<div class="mt-3 flex items-start gap-3 border-t border-border pt-3">
							<span class="text-xs font-medium text-muted-foreground">Cover:</span>
							<img
								src={candidate.cover_url}
								alt="Candidate cover"
								class="h-20 rounded object-cover shadow-sm"
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
