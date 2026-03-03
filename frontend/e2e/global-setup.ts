import { test as setup, expect } from '@playwright/test';
import type { APIRequestContext } from '@playwright/test';
import { execSync, spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { API_BASE, FRONTEND_BASE, TEST_ADMIN } from './fixtures/test-data';

const PROJECT_ROOT = fileURLToPath(new URL('../..', import.meta.url));

async function waitForServer(request: APIRequestContext, timeoutMs = 120_000) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		try {
			const res = await request.get(`${API_BASE}/auth/status`);
			if (res.ok()) return;
		} catch {
			/* not ready yet */
		}
		await new Promise((resolve) => setTimeout(resolve, 1_000));
	}
	throw new Error(`Server not ready within ${timeoutMs / 1_000}s`);
}

/** Kill the reused server and start a fresh E2E instance. */
function recycleServer() {
	try {
		execSync('lsof -ti :9514 | xargs kill', { stdio: 'ignore' });
	} catch {
		/* nothing to kill */
	}

	// Mirrors the Playwright webServer command in playwright.config.ts
	execSync('rm -rf .local/e2e && mkdir -p .local/e2e', { cwd: PROJECT_ROOT });
	const proc = spawn(
		'cargo',
		['run', '--package', 'archivis-server', '--', '--data-dir', '.local/e2e'],
		{ cwd: PROJECT_ROOT, detached: true, stdio: 'ignore' }
	);
	proc.unref();
}

setup('create admin and authenticate', async ({ request, page }) => {
	// Create admin account via setup endpoint
	const setupResponse = await request.post(`${API_BASE}/auth/setup`, {
		data: {
			username: TEST_ADMIN.username,
			password: TEST_ADMIN.password
		}
	});

	// 201 = created, 403 = admin already exists (reused server) — both OK
	if (!setupResponse.ok() && setupResponse.status() !== 403) {
		throw new Error(
			`/api/auth/setup returned unexpected status ${setupResponse.status()}`
		);
	}

	// Log in to get a token
	let loginResponse = await request.post(`${API_BASE}/auth/login`, {
		data: {
			username: TEST_ADMIN.username,
			password: TEST_ADMIN.password
		}
	});

	// Login failed — reused server has a different admin. Recycle it.
	if (!loginResponse.ok()) {
		recycleServer();
		await waitForServer(request);

		const retrySetup = await request.post(`${API_BASE}/auth/setup`, {
			data: { username: TEST_ADMIN.username, password: TEST_ADMIN.password }
		});
		if (!retrySetup.ok()) {
			throw new Error(`Setup failed after server recycle: ${retrySetup.status()}`);
		}

		loginResponse = await request.post(`${API_BASE}/auth/login`, {
			data: { username: TEST_ADMIN.username, password: TEST_ADMIN.password }
		});
		if (!loginResponse.ok()) {
			throw new Error(`Login failed after server recycle: ${loginResponse.status()}`);
		}
	}

	const { token } = await loginResponse.json();
	expect(token).toBeTruthy();

	// Set localStorage token via the default page fixture
	await page.goto(FRONTEND_BASE + '/login');
	await page.evaluate(
		({ token }) => {
			localStorage.setItem('archivis-session', token);
		},
		{ token }
	);

	// Save storage state for authenticated tests
	await page.context().storageState({ path: './e2e/.auth/admin.json' });
});
