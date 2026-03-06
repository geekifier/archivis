import { test, expect } from '@playwright/test';
import { getAuthToken, seedBookFromFixture, waitForTask, listBooks } from '../helpers/api-helpers';

test.describe('Book detail page', () => {
  let bookId: string;

  test.beforeAll(async ({ request }) => {
    const token = await getAuthToken(request);

    // Check if there are already books from library test suite
    const books = await listBooks(request, token);
    if (books.items.length > 0) {
      bookId = books.items[0].id;
    } else {
      const { taskId } = await seedBookFromFixture(request, token);
      await waitForTask(request, token, taskId);
      const seededBooks = await listBooks(request, token);
      bookId = seededBooks.items[0].id;
    }
  });

  test('navigate from library to book detail', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('a[href^="/books/"]').first()).toBeVisible({ timeout: 10_000 });

    // Click the first book link
    await page.locator('a[href^="/books/"]').first().click();
    await expect(page).toHaveURL(/\/books\/.+/);
  });

  test('book detail shows title and files section', async ({ page }) => {
    await page.goto(`/books/${bookId}`);

    // Title should be visible (the h1 on the detail page)
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.getByRole('button', { name: 'Refresh Metadata' })).toBeVisible();
    await expect(page.getByText('Metadata controls')).toBeVisible();

    // Files section should be present
    await expect(page.getByText('Files')).toBeVisible();
  });

  test('file actions stay readable on narrow viewport and download exposes size tooltip', async ({
    page
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/books/${bookId}`);
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

    const downloadLink = page.getByRole('link', { name: /download/i }).first();
    const readLink = page.getByRole('link', { name: 'Read' }).first();

    await expect(downloadLink).toBeVisible();
    await expect(readLink).toBeVisible();
    await expect(downloadLink).toHaveAttribute('title', /^Download \(.+\)$/);

    const downloadBox = await downloadLink.boundingBox();
    const readBox = await readLink.boundingBox();
    expect(downloadBox).not.toBeNull();
    expect(readBox).not.toBeNull();

    if (!downloadBox || !readBox) {
      return;
    }

    const overlapWidth =
      Math.min(downloadBox.x + downloadBox.width, readBox.x + readBox.width) -
      Math.max(downloadBox.x, readBox.x);
    const overlapHeight =
      Math.min(downloadBox.y + downloadBox.height, readBox.y + readBox.height) -
      Math.max(downloadBox.y, readBox.y);

    expect(overlapWidth > 0 && overlapHeight > 0).toBe(false);
  });

  test('edit mode: modify title and save', async ({ page }) => {
    await page.goto(`/books/${bookId}`);
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

    // Get original title
    const originalTitle = await page.locator('h1').first().textContent();

    // Click Edit button
    await page.getByRole('button', { name: 'Edit' }).click();

    // Find the title input and modify it
    const titleInput = page.locator('input[name="title"], input#title').first();
    if (await titleInput.isVisible()) {
      await titleInput.clear();
      await titleInput.fill('E2E Test Title');

      // Save
      await page.getByRole('button', { name: 'Save' }).click();

      // Verify title updated
      await expect(page.locator('h1').first()).toContainText('E2E Test Title');

      // Restore original title
      await page.getByRole('button', { name: 'Edit' }).click();
      await titleInput.clear();
      await titleInput.fill(originalTitle || 'Frankenstein');
      await page.getByRole('button', { name: 'Save' }).click();
    }
  });

  test('can navigate back to library', async ({ page }) => {
    await page.goto(`/books/${bookId}`);
    await expect(page.locator('h1').first()).toBeVisible({ timeout: 10_000 });

    // Click the Library nav link in sidebar (exact match to avoid "Back to Library")
    await page.getByRole('link', { name: 'Library', exact: true }).click();
    await expect(page).toHaveURL('/');
  });
});
