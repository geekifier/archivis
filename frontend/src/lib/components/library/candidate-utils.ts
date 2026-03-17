import { ApiError } from '$lib/api/index.js';

/** Field names that can be included/excluded in a candidate apply. */
export type CandidateFieldName =
  | 'title'
  | 'subtitle'
  | 'authors'
  | 'publisher'
  | 'publication_year'
  | 'language'
  | 'page_count'
  | 'identifiers'
  | 'series'
  | 'description'
  | 'cover';

/** Extract a user-facing message from an unknown error. */
export function extractErrorMessage(err: unknown, fallback = 'An error occurred'): string {
  if (err instanceof ApiError) return err.userMessage;
  if (err instanceof Error) return err.message;
  return fallback;
}

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

/** Order-independent, case-insensitive comparison of two name arrays. */
export function namesMatch(a: string[], b: string[]): boolean {
  const normalize = (names: string[]) =>
    new Set(names.map((n) => n.trim().toLowerCase()));
  const setA = normalize(a);
  const setB = normalize(b);
  if (setA.size !== setB.size) return false;
  for (const name of setA) {
    if (!setB.has(name)) return false;
  }
  return true;
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

/** Return a Tailwind class string for a tier badge. */
export function tierColorClass(tier: string | undefined): string {
    switch (tier) {
        case 'strong_id_match':
            return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400';
        case 'probable_match':
            return 'bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400';
        case 'weak_match':
        default:
            return 'bg-muted text-muted-foreground';
    }
}

/** Return a human-readable label for a tier value. */
export function tierLabel(tier: string | undefined): string {
    switch (tier) {
        case 'strong_id_match':
            return 'Strong ID match';
        case 'probable_match':
            return 'Probable match';
        case 'weak_match':
            return 'Weak match';
        default:
            return tier ?? '';
    }
}

/** True when a match reason represents a warning (mismatch / penalty). */
export function isWarningReason(reason: string): boolean {
  const lower = reason.toLowerCase();
  return lower.includes('mismatch') || lower.includes('contradiction');
}

/** Map warning-type match reasons to the field they affect. */
export function warningFields(reasons: string[]): Set<CandidateFieldName> {
  const fields = new Set<CandidateFieldName>();
  for (const r of reasons) {
    const lower = r.toLowerCase();
    if (lower.includes('title contradiction')) fields.add('title');
    if (lower.includes('description language mismatch')) fields.add('description');
  }
  return fields;
}

/** Collect field names the user has deselected (unchecked). */
export function getExcludedFields(
  fieldSelections: Record<string, Partial<Record<CandidateFieldName, boolean>>>,
  candidateId: string
): string[] {
  const sel = fieldSelections[candidateId];
  if (!sel) return [];
  return Object.entries(sel)
    .filter(([, included]) => !included)
    .map(([field]) => field);
}
