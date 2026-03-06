import { test, expect } from '@playwright/test';
import { TEST_ADMIN } from '../fixtures/test-data';

test.describe('Login flow', () => {
  test('login form has expected elements', async ({ page }) => {
    await page.goto('/login');
    await expect(page.locator('label[for="username"]')).toBeVisible();
    await expect(page.locator('#username')).toBeVisible();
    await expect(page.locator('label[for="password"]')).toBeVisible();
    await expect(page.locator('#password')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible();
  });

  test('logout redirects to /login', async ({ page }) => {
    await page.goto('/');
    // Wait for the authenticated app shell to render (sidebar "Library" link)
    await expect(page.getByRole('link', { name: 'Library', exact: true })).toBeVisible({
      timeout: 10_000
    });

    // Click logout button
    await page.getByRole('button', { name: 'Log out' }).click();
    await expect(page).toHaveURL(/\/login/);
  });

  test('login with valid credentials reaches library', async ({ page }) => {
    // Go directly to login page (clears auth state via evaluate)
    await page.goto('/login');
    await page.evaluate(() => localStorage.removeItem('archivis-session'));
    await page.goto('/login');
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible({ timeout: 10_000 });

    // Login with valid credentials
    await page.locator('#username').fill(TEST_ADMIN.username);
    await page.locator('#password').fill(TEST_ADMIN.password);
    await page.getByRole('button', { name: 'Sign in' }).click();

    // Should reach library
    await expect(page).toHaveURL('/');
    await expect(page.getByRole('link', { name: 'Library', exact: true })).toBeVisible({
      timeout: 10_000
    });
  });
});
