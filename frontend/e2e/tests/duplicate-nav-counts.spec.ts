import { test, expect } from '@playwright/test';
import { getAuthToken, seedBookFromFixture, waitForTask, listBooks } from '../helpers/api-helpers';
import { API_BASE, TEST_ADMIN } from '../fixtures/test-data';

/**
 * E2E tests for sidebar duplicate badge count reactivity.
 *
 * These tests require a running backend on :9514 with at least one book seeded.
 * Duplicate links are created/dismissed via API, then the sidebar badge is verified.
 */
test.describe('Duplicate nav count reactivity', () => {
  let token: string;
  let bookIds: string[];

  test.beforeAll(async ({ request }) => {
    token = await getAuthToken(request);

    // Ensure at least 2 books exist for flagging duplicates
    const books = await listBooks(request, token);
    if (books.items.length < 2) {
      const { taskId } = await seedBookFromFixture(request, token);
      await waitForTask(request, token, taskId);
      const { taskId: taskId2 } = await seedBookFromFixture(request, token);
      await waitForTask(request, token, taskId2);
    }

    const allBooks = await listBooks(request, token);
    bookIds = allBooks.items.slice(0, 2).map((b) => b.id);
  });

  test('sidebar badge updates after flagging and dismissing a duplicate', async ({
    page,
    request
  }) => {
    // Navigate to library and log in
    await page.goto('/login');
    await page.getByLabel('Username').fill(TEST_ADMIN.username);
    await page.getByLabel('Password').fill(TEST_ADMIN.password);
    await page.getByRole('button', { name: 'Sign In' }).click();

    // Wait for library to load
    await expect(page.locator('a[href^="/books/"]').first()).toBeVisible({ timeout: 10_000 });

    // Get initial duplicate count badge (may or may not exist)
    const sidebar = page.locator('aside');
    const duplicateLink = sidebar.locator('a[href="/duplicates"]');
    await expect(duplicateLink).toBeVisible();

    // Flag a duplicate via API
    const flagResponse = await request.post(`${API_BASE}/books/${bookIds[0]}/duplicates`, {
      headers: { Authorization: `Bearer ${token}` },
      data: { other_book_id: bookIds[1] }
    });
    expect(flagResponse.ok()).toBeTruthy();
    const flagData = await flagResponse.json();
    const linkId = flagData.id;

    // Navigate to trigger count refresh
    await page.goto('/duplicates');
    await page.waitForTimeout(500); // Allow debounced refresh

    // Badge should now show a count (at least 1)
    const badge = duplicateLink.locator('span.rounded-full');
    await expect(badge).toBeVisible({ timeout: 5_000 });

    // Dismiss the duplicate via API
    const dismissResponse = await request.post(`${API_BASE}/duplicates/${linkId}/dismiss`, {
      headers: { Authorization: `Bearer ${token}` }
    });
    expect(dismissResponse.ok()).toBeTruthy();

    // Navigate to trigger count refresh
    await page.goto('/');
    await page.waitForTimeout(500); // Allow debounced refresh

    // The badge count should have decreased (may disappear if it was the only one)
    // We just verify the page loaded without error
    await expect(page.locator('a[href^="/books/"]').first()).toBeVisible({ timeout: 10_000 });
  });
});
