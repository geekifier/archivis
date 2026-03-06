/** Return a Tailwind background class for the confidence score bar. */
export function scoreColor(score: number): string {
  if (score >= 0.8) return 'bg-green-500';
  if (score >= 0.5) return 'bg-amber-500';
  return 'bg-red-500';
}

/** Format a 0-1 confidence score as a percentage string. */
export function formatScore(score: number): string {
  return `${Math.round(score * 100)}%`;
}

/** Return a Tailwind class string for a provider badge based on name. */
export function providerColorClass(provider: string): string {
  const lower = provider.toLowerCase();
  if (lower.includes('open library'))
    return 'bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400';
  if (lower.includes('hardcover'))
    return 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400';
  return 'bg-muted text-muted-foreground';
}

/** True when the candidate value differs from the current book value. */
export function hasChange(
  candidateValue: string | undefined | null,
  bookValue: string | undefined | null
): boolean {
  const cv = candidateValue ?? '';
  const bv = bookValue ?? '';
  return cv !== '' && cv !== bv;
}

/** Collect field names the user has deselected (unchecked). */
export function getExcludedFields(
  fieldSelections: Record<string, Record<string, boolean>>,
  candidateId: string
): string[] {
  const sel = fieldSelections[candidateId];
  if (!sel) return [];
  return Object.entries(sel)
    .filter(([, included]) => !included)
    .map(([field]) => field);
}
