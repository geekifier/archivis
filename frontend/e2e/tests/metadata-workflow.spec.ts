import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';

import { test, expect, type Page } from '@playwright/test';

import { getAuthToken, seedBookFromFixture, waitForTask, listBooks } from '../helpers/api-helpers';

type TargetStatus = 'needs_review' | 'unidentified';

const E2E_DB_PATH = fileURLToPath(new URL('../../../.local/e2e/archivis.db', import.meta.url));

function sqlString(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

function seedCandidateReviewState(bookId: string, title: string, status: TargetStatus) {
  const runId = randomUUID();
  const candidateId = randomUUID();
  const timestamp = new Date().toISOString();
  const metadata = JSON.stringify({
    provider_name: 'e2e_provider',
    title,
    subtitle: null,
    authors: [],
    description: null,
    language: null,
    publisher: null,
    publication_year: null,
    identifiers: [],
    subjects: [],
    series: null,
    page_count: null,
    cover_url: null,
    rating: null,
    confidence: 0.92
  });
  const sql = `
BEGIN;
DELETE FROM identification_candidates WHERE book_id = ${sqlString(bookId)};
DELETE FROM resolution_runs WHERE book_id = ${sqlString(bookId)};
UPDATE books
SET metadata_status = ${sqlString(status)},
	resolution_state = 'done',
	resolution_outcome = 'disputed',
	resolution_requested_at = ${sqlString(timestamp)},
	resolution_requested_reason = 'e2e_seed',
	last_resolved_at = ${sqlString(timestamp)},
	last_resolution_run_id = ${sqlString(runId)},
	metadata_locked = 0
WHERE id = ${sqlString(bookId)};
INSERT INTO resolution_runs (
	id, book_id, trigger, state, outcome, query_json, decision_code,
	candidate_count, best_candidate_id, best_score, best_tier, error, started_at, finished_at
) VALUES (
	${sqlString(runId)},
	${sqlString(bookId)},
	'e2e_seed',
	'done',
	'disputed',
	'{}',
	'e2e_seed',
	1,
	${sqlString(candidateId)},
	0.92,
	'title_author',
	NULL,
	${sqlString(timestamp)},
	${sqlString(timestamp)}
);
INSERT INTO identification_candidates (
	id, book_id, run_id, provider_name, score, metadata, match_reasons,
	disputes, status, created_at
) VALUES (
	${sqlString(candidateId)},
	${sqlString(bookId)},
	${sqlString(runId)},
	'e2e_provider',
	0.92,
	${sqlString(metadata)},
	'["e2e seeded review"]',
	'["Title differs from provider''s suggestion"]',
	'pending',
	${sqlString(timestamp)}
);
COMMIT;
`;

  execFileSync('sqlite3', [E2E_DB_PATH, sql], { stdio: 'pipe' });
}

async function readStatusBadgeCount(page: Page, label: string): Promise<number> {
  const statusButton = page.locator('button', { hasText: label }).first();
  await expect(statusButton).toBeVisible();
  const badge = statusButton.locator('span.min-w-5.rounded-full').first();
  if ((await badge.count()) === 0) return 0;
  if (!(await badge.isVisible())) return 0;
  const text = (await badge.textContent())?.trim() ?? '';
  const parsed = Number.parseInt(text, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

async function verifyBadgeDecrementsOnCandidateApply(
  page: Page,
  statusLabel: string,
  expectedDelta: number
) {
  let beforeCount = 0;
  await expect
    .poll(
      async () => {
        beforeCount = await readStatusBadgeCount(page, statusLabel);
        return beforeCount;
      },
      { timeout: 10_000 }
    )
    .toBeGreaterThan(0);

  const currentUrl = page.url();
  await expect(page.getByRole('button', { name: 'Apply' }).first()).toBeVisible({
    timeout: 10_000
  });
  await page.getByRole('button', { name: 'Apply' }).first().click();
  await expect(page).toHaveURL(currentUrl);

  await expect
    .poll(() => readStatusBadgeCount(page, statusLabel), { timeout: 10_000 })
    .toBe(Math.max(beforeCount - expectedDelta, 0));
}

test.describe('Metadata workflow', () => {
  let token: string;
  let bookId: string;
  let bookTitle: string;

  test.beforeAll(async ({ request }) => {
    token = await getAuthToken(request);

    let books = await listBooks(request, token);
    if (books.items.length === 0) {
      const { taskId } = await seedBookFromFixture(request, token);
      await waitForTask(request, token, taskId);
      books = await listBooks(request, token);
    }

    bookId = books.items[0].id;
    bookTitle = books.items[0].title;
  });

  test('book detail prefers refresh and review over manual status buttons', async ({ page }) => {
    await page.goto(`/books/${bookId}`);
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

    await expect(page.getByRole('button', { name: 'Refresh Metadata' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Metadata controls' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Mark as Identified' })).toHaveCount(0);
  });

  test('needs_review badge decrements after applying a review candidate on the same page', async ({
    page
  }) => {
    seedCandidateReviewState(bookId, bookTitle, 'needs_review');
    await page.goto(`/books/${bookId}`);
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

    await verifyBadgeDecrementsOnCandidateApply(page, 'Needs Review', 1);
  });

  test('unidentified badge decrements after applying a review candidate on the same page', async ({
    page
  }) => {
    seedCandidateReviewState(bookId, bookTitle, 'unidentified');
    await page.goto(`/books/${bookId}`);
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

    await verifyBadgeDecrementsOnCandidateApply(page, 'Unidentified', 1);
  });
});
