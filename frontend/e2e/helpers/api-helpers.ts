import { APIRequestContext } from '@playwright/test';
import { API_BASE, TEST_ADMIN } from '../fixtures/test-data';
import { resolve, dirname } from 'path';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export async function getAuthToken(request: APIRequestContext): Promise<string> {
	const response = await request.post(`${API_BASE}/auth/login`, {
		data: {
			username: TEST_ADMIN.username,
			password: TEST_ADMIN.password
		}
	});
	if (!response.ok()) {
		throw new Error(`Login failed: ${response.status()} ${await response.text()}`);
	}
	const body = await response.json();
	return body.token;
}

export async function seedBookFromFixture(
	request: APIRequestContext,
	token: string
): Promise<{ taskId: string }> {
	const fixturePath = resolve(__dirname, '../fixtures/test-data/mary-shelley_frankenstein.epub');
	const fileBuffer = readFileSync(fixturePath);

	const response = await request.post(`${API_BASE}/import/upload`, {
		headers: {
			Authorization: `Bearer ${token}`
		},
		multipart: {
			files: {
				name: 'mary-shelley_frankenstein.epub',
				mimeType: 'application/epub+zip',
				buffer: fileBuffer
			}
		}
	});
	if (!response.ok()) {
		throw new Error(`Upload failed: ${response.status()} ${await response.text()}`);
	}
	const body = await response.json();
	const taskId = body.tasks[0].task_id;
	return { taskId };
}

export async function waitForTask(
	request: APIRequestContext,
	token: string,
	taskId: string,
	timeoutMs = 30_000
): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	const terminalStatuses = ['completed', 'failed', 'cancelled'];

	while (Date.now() < deadline) {
		const response = await request.get(`${API_BASE}/tasks/${taskId}`, {
			headers: { Authorization: `Bearer ${token}` }
		});
		if (!response.ok()) {
			throw new Error(`Task poll failed: ${response.status()}`);
		}
		const task = await response.json();
		if (terminalStatuses.includes(task.status)) {
			if (task.status === 'failed') {
				throw new Error(`Task failed: ${task.error_message || 'unknown'}`);
			}
			return;
		}
		await new Promise((r) => setTimeout(r, 500));
	}
	throw new Error(`Task ${taskId} did not complete within ${timeoutMs}ms`);
}

export async function listBooks(
	request: APIRequestContext,
	token: string
): Promise<{ items: Array<{ id: string; title: string }>; total: number }> {
	const response = await request.get(`${API_BASE}/books`, {
		headers: { Authorization: `Bearer ${token}` }
	});
	if (!response.ok()) {
		throw new Error(`List books failed: ${response.status()}`);
	}
	return response.json();
}
