import { test, expect } from '@playwright/test';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

test.describe('Import page', () => {
	test('shows Upload Files and Scan Directory mode tabs', async ({ page }) => {
		await page.goto('/import');
		await expect(page.getByRole('button', { name: 'Upload Files' })).toBeVisible({ timeout: 10_000 });
		await expect(page.getByRole('button', { name: 'Scan Directory' })).toBeVisible();
	});

	test('switch between upload and scan modes', async ({ page }) => {
		await page.goto('/import');

		// Default is upload mode - should see upload card
		await expect(page.getByText('Upload E-books')).toBeVisible({ timeout: 10_000 });

		// Switch to scan mode
		await page.getByRole('button', { name: 'Scan Directory' }).click();
		await expect(page.getByPlaceholder('/path/to/ebooks')).toBeVisible();

		// Switch back to upload mode
		await page.getByRole('button', { name: 'Upload Files' }).click();
		await expect(page.getByText('Upload E-books')).toBeVisible();
	});

	test('upload a file via file input', async ({ page }) => {
		await page.goto('/import');
		await expect(page.getByRole('button', { name: 'Upload Files' })).toBeVisible({ timeout: 10_000 });

		const fixturePath = resolve(__dirname, '../fixtures/test-data/mary-shelley_frankenstein.epub');

		// Set file on the hidden input
		const fileInput = page.locator('#file-input');
		await fileInput.setInputFiles(fixturePath);

		// Should show the file in the selected list
		await expect(page.getByText('mary-shelley_frankenstein.epub')).toBeVisible();

		// Click upload
		await page.getByRole('button', { name: /Upload 1 file/ }).click();

		// Task progress should appear (ActiveTaskPanel or recent activity)
		await expect(
			page.getByText(/Processing|Import|Running|Completed|completed/i).first()
		).toBeVisible({ timeout: 30_000 });
	});
});
