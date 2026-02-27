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

test.describe('Reader font adjustment & keyboard shortcuts', () => {
	let bookId: string;
	let fileId: string;

	test.beforeAll(async ({ request }) => {
		const token = await getAuthToken(request);

		const books = await listBooks(request, token);
		if (books.items.length > 0) {
			bookId = books.items[0].id;
		} else {
			const { taskId } = await seedBookFromFixture(request, token);
			await waitForTask(request, token, taskId);
			const seededBooks = await listBooks(request, token);
			bookId = seededBooks.items[0].id;
		}

		const detail = await getBookDetail(request, token, bookId);
		const epubFile = detail.files.find((f) => f.format === 'epub') ?? detail.files[0];
		fileId = epubFile.id;
	});

	test.beforeEach(async ({ page }) => {
		// Clear reader preferences from localStorage before each test
		await page.goto(`/read/${bookId}/${fileId}`);
		await page.evaluate(() => localStorage.removeItem('archivis-reader-prefs'));
		// Reload to start with default preferences
		await page.reload();
		await expect(page.locator('foliate-view')).toBeAttached({ timeout: 15_000 });
	});

	test('+ key increases font size', async ({ page }) => {
		// Open settings panel to see font size value
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });

		// Default should be 100%
		await expect(page.getByText('100%')).toBeVisible();

		// Close settings, press +, reopen
		await page.keyboard.press('Escape');
		await page.keyboard.press('+');
		await page.keyboard.press('s');

		await expect(page.getByText('110%')).toBeVisible();
	});

	test('- key decreases font size', async ({ page }) => {
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });
		await expect(page.getByText('100%')).toBeVisible();

		await page.keyboard.press('Escape');
		await page.keyboard.press('-');
		await page.keyboard.press('s');

		await expect(page.getByText('90%')).toBeVisible();
	});

	test('settings panel increase/decrease buttons work', async ({ page }) => {
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });

		// Click the increase button
		await page.getByLabel('Increase font size').click();
		await expect(page.getByText('110%')).toBeVisible();

		// Click the decrease button
		await page.getByLabel('Decrease font size').click();
		await expect(page.getByText('100%')).toBeVisible();
	});

	test('font size persists to localStorage', async ({ page }) => {
		// Increase font size
		await page.keyboard.press('+');
		await page.keyboard.press('+');

		// Check localStorage
		const stored = await page.evaluate(() => localStorage.getItem('archivis-reader-prefs'));
		expect(stored).toBeTruthy();
		const parsed = JSON.parse(stored!);
		expect(parsed.fontSize).toBe(120);
	});

	test('font size preference survives page reload', async ({ page }) => {
		// Increase font size
		await page.keyboard.press('+');
		await page.keyboard.press('+');

		// Reload
		await page.reload();
		await expect(page.locator('foliate-view')).toBeAttached({ timeout: 15_000 });

		// Open settings and verify
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });
		await expect(page.getByText('120%')).toBeVisible();
	});

	test('font size setting persists after section navigation', async ({ page }) => {
		// Increase font size
		await page.keyboard.press('+');

		// Navigate forward several times to trigger new section load
		for (let i = 0; i < 5; i++) {
			await page.keyboard.press('ArrowRight');
			await page.waitForTimeout(300);
		}

		// Open settings panel and verify font size is still 110%
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });
		await expect(page.getByText('110%')).toBeVisible();
	});

	test('s toggles settings panel', async ({ page }) => {
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });

		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).not.toBeVisible();
	});

	test('b toggles bookmarks panel', async ({ page }) => {
		await page.keyboard.press('b');
		await expect(page.getByText('Bookmarks')).toBeVisible({ timeout: 5_000 });

		await page.keyboard.press('b');
		await expect(page.getByText('Bookmarks')).not.toBeVisible();
	});

	test('Escape closes open panel', async ({ page }) => {
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });

		await page.keyboard.press('Escape');
		await expect(page.getByText('Reader Settings')).not.toBeVisible();
	});

	test('theme switch changes container background color', async ({ page }) => {
		// Open settings
		await page.keyboard.press('s');
		await expect(page.getByText('Reader Settings')).toBeVisible({ timeout: 5_000 });

		// Click the Dark theme swatch
		await page.getByLabel('Dark theme').click();

		// The outer container should now have the dark background
		const container = page.locator('div.relative.flex.h-screen');
		await expect(container).toHaveCSS('background-color', 'rgb(26, 26, 26)');
	});
});
