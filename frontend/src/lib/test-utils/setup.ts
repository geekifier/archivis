import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';

vi.mock('$app/navigation', () => ({
  goto: vi.fn(),
  invalidate: vi.fn(),
  invalidateAll: vi.fn(),
  afterNavigate: vi.fn(),
  beforeNavigate: vi.fn(),
  onNavigate: vi.fn(),
  preloadData: vi.fn(),
  preloadCode: vi.fn(),
  pushState: vi.fn(),
  replaceState: vi.fn()
}));

vi.mock('$app/environment', () => ({
  browser: true,
  dev: true,
  building: false,
  version: 'test'
}));

vi.mock('$app/state', () => {
  const page = {
    url: new URL('http://localhost'),
    params: {},
    route: { id: '/' },
    status: 200,
    error: null,
    data: {},
    form: null
  };
  return { page };
});
