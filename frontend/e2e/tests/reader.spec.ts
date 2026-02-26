import { test, expect } from '@playwright/test';
import {
	getAuthToken,
	seedBookFromFixture,
	waitForTask,
	listBooks,
	getBookDetail
} from '../helpers/api-helpers';

test.describe('EPUB reader', () => {
	let bookId: string;
	let fileId: string;

	test.beforeAll(async ({ request }) => {
		const token = await getAuthToken(request);

		// Ensure a book exists
		const books = await listBooks(request, token);
		if (books.items.length > 0) {
			bookId = books.items[0].id;
		} else {
			const { taskId } = await seedBookFromFixture(request, token);
			await waitForTask(request, token, taskId);
			const seededBooks = await listBooks(request, token);
			bookId = seededBooks.items[0].id;
		}

		// Get fileId from book detail
		const detail = await getBookDetail(request, token, bookId);
		const epubFile = detail.files.find((f) => f.format === 'epub') ?? detail.files[0];
		fileId = epubFile.id;
	});

	test('opens EPUB without error', async ({ page }) => {
		await page.goto(`/read/${bookId}/${fileId}`);

		// The foliate-view custom element should be attached once the book opens
		await expect(page.locator('foliate-view')).toBeAttached({ timeout: 15_000 });

		// No error messages should be visible
		await expect(page.getByText('Something Went Wrong')).not.toBeVisible();
		await expect(page.getByText('Unable to Open Book')).not.toBeVisible();
	});

	test('TOC panel opens', async ({ page }) => {
		await page.goto(`/read/${bookId}/${fileId}`);
		await expect(page.locator('foliate-view')).toBeAttached({ timeout: 15_000 });

		// Press 't' to toggle TOC panel
		await page.keyboard.press('t');

		// TOC panel should become visible (look for nav/list inside the panel)
		await expect(page.getByText('Table of Contents')).toBeVisible({ timeout: 5_000 });
	});

	test('navigation works without errors', async ({ page }) => {
		await page.goto(`/read/${bookId}/${fileId}`);
		await expect(page.locator('foliate-view')).toBeAttached({ timeout: 15_000 });

		// Navigate forward and back — should not throw
		await page.keyboard.press('ArrowRight');
		await page.waitForTimeout(500);
		await page.keyboard.press('ArrowLeft');
		await page.waitForTimeout(500);

		// Still no error state
		await expect(page.getByText('Something Went Wrong')).not.toBeVisible();
		await expect(page.getByText('Unable to Open Book')).not.toBeVisible();
	});
});
