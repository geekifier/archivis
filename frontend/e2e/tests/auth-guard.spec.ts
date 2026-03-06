import { test, expect } from '@playwright/test';

test.describe('Auth guard redirects', () => {
  test('redirects / to /login', async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveURL(/\/login/);
  });

  test('redirects /import to /login with redirect param', async ({ page }) => {
    await page.goto('/import');
    await expect(page).toHaveURL(/\/login\?redirect=%2Fimport/);
  });

  test('redirects /settings to /login with redirect param', async ({ page }) => {
    await page.goto('/settings');
    await expect(page).toHaveURL(/\/login\?redirect=%2Fsettings/);
  });

  test('redirects /books/some-id to /login', async ({ page }) => {
    await page.goto('/books/some-id');
    // The book detail page may eagerly fetch and trigger a 401 hard redirect
    // (via window.location.href) before the layout auth guard's goto runs,
    // so the redirect query param is not guaranteed here.
    await expect(page).toHaveURL(/\/login/);
  });

  test('login page is accessible without auth', async ({ page }) => {
    await page.goto('/login');
    await expect(page.getByText('Log in')).toBeVisible();
  });
});
