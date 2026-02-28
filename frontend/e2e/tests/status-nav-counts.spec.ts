import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { getAuthToken, seedBookFromFixture, waitForTask, listBooks } from '../helpers/api-helpers';
import { API_BASE } from '../fixtures/test-data';

type TargetStatus = 'needs_review' | 'unidentified';

async function setBookStatus(
	request: APIRequestContext,
	token: string,
	bookId: string,
	status: 'identified' | TargetStatus
) {
	const response = await request.put(`${API_BASE}/books/${bookId}`, {
		headers: {
			Authorization: `Bearer ${token}`
		},
		data: {
			metadata_status: status
		}
	});
	expect(response.ok()).toBeTruthy();
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

async function verifyBadgeDecrementsOnMarkIdentified(
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
	await page.getByRole('button', { name: 'Mark as Identified' }).click();
	await expect(page).toHaveURL(currentUrl);

	await expect
		.poll(() => readStatusBadgeCount(page, statusLabel), { timeout: 10_000 })
		.toBe(Math.max(beforeCount - expectedDelta, 0));
}

test.describe('Status nav count reactivity', () => {
	let token: string;
	let bookId: string;

	test.beforeAll(async ({ request }) => {
		token = await getAuthToken(request);

		let books = await listBooks(request, token);
		if (books.items.length === 0) {
			const { taskId } = await seedBookFromFixture(request, token);
			await waitForTask(request, token, taskId);
			books = await listBooks(request, token);
		}

		bookId = books.items[0].id;
	});

	test('needs_review badge decrements after mark identified on same page', async ({
		page,
		request
	}) => {
		await setBookStatus(request, token, bookId, 'needs_review');
		await page.goto(`/books/${bookId}`);
		await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

		await verifyBadgeDecrementsOnMarkIdentified(page, 'Needs Review', 1);
	});

	test('unidentified badge decrements after mark identified on same page', async ({
		page,
		request
	}) => {
		await setBookStatus(request, token, bookId, 'unidentified');
		await page.goto(`/books/${bookId}`);
		await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

		await verifyBadgeDecrementsOnMarkIdentified(page, 'Unidentified', 1);
	});
});
