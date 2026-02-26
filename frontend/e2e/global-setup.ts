import { test as setup, expect } from '@playwright/test';
import { API_BASE, FRONTEND_BASE, TEST_ADMIN } from './fixtures/test-data';

setup('create admin and authenticate', async ({ request, browser }) => {
	// Create admin account via setup endpoint
	const setupResponse = await request.post(`${API_BASE}/auth/setup`, {
		data: {
			username: TEST_ADMIN.username,
			password: TEST_ADMIN.password
		}
	});
	expect(setupResponse.ok()).toBeTruthy();

	// Log in to get a token
	const loginResponse = await request.post(`${API_BASE}/auth/login`, {
		data: {
			username: TEST_ADMIN.username,
			password: TEST_ADMIN.password
		}
	});
	expect(loginResponse.ok()).toBeTruthy();
	const { token } = await loginResponse.json();
	expect(token).toBeTruthy();

	// Open browser context and set localStorage token
	const context = await browser.newContext();
	const page = await context.newPage();
	await page.goto(FRONTEND_BASE + '/login');
	await page.evaluate(
		({ token }) => {
			localStorage.setItem('archivis-session', token);
		},
		{ token }
	);

	// Save storage state for authenticated tests
	await context.storageState({ path: './e2e/.auth/admin.json' });
	await context.close();
});
