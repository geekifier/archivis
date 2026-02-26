import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
	testDir: './e2e/tests',
	fullyParallel: false,
	forbidOnly: !!process.env.CI,
	retries: 0,
	workers: 1,
	reporter: 'html',
	use: {
		baseURL: 'http://localhost:5173',
		trace: 'on-first-retry'
	},
	projects: [
		{
			name: 'setup',
			testMatch: /global-setup\.ts/,
			testDir: './e2e'
		},
		{
			name: 'chromium',
			use: {
				...devices['Desktop Chrome'],
				storageState: './e2e/.auth/admin.json'
			},
			dependencies: ['setup'],
			testIgnore: /auth-guard\.spec\.ts/
		},
		{
			name: 'unauthenticated',
			use: {
				...devices['Desktop Chrome'],
				storageState: { cookies: [], origins: [] }
			},
			testMatch: /auth-guard\.spec\.ts/
		}
	],
	webServer: [
		{
			command:
				'cd .. && rm -rf .local/e2e && mkdir -p .local/e2e && cargo run --package archivis-server -- --data-dir .local/e2e',
			url: 'http://localhost:9514/api/auth/status',
			timeout: 120_000,
			reuseExistingServer: true,
			stdout: 'pipe',
			stderr: 'pipe'
		},
		{
			command: 'npm run dev',
			url: 'http://localhost:5173',
			timeout: 15_000,
			reuseExistingServer: true,
			stdout: 'pipe',
			stderr: 'pipe'
		}
	]
});
