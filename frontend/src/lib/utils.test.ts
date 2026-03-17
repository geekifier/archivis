import { describe, it, expect } from 'vitest';
import {
  placeholderHue,
  formatFileSize,
  formatMetadataSource
} from './utils.js';

describe('placeholderHue', () => {
  it('returns a number in [0, 360)', () => {
    const hue = placeholderHue('test-id');
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });

  it('is deterministic (same input produces same output)', () => {
    const a = placeholderHue('my-book-id');
    const b = placeholderHue('my-book-id');
    expect(a).toBe(b);
  });

  it('produces different outputs for different inputs', () => {
    const a = placeholderHue('book-1');
    const b = placeholderHue('book-2');
    const c = placeholderHue('book-3');
    // At least two of three should differ
    expect(a === b && b === c).toBe(false);
  });

  it('handles empty string', () => {
    const hue = placeholderHue('');
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
    // hash of empty string is 0, so 0 % 360 = 0
    expect(hue).toBe(0);
  });
});

describe('formatFileSize', () => {
  it('formats 0 bytes', () => {
    expect(formatFileSize(0)).toBe('0 B');
  });

  it('formats bytes below 1 KB', () => {
    expect(formatFileSize(500)).toBe('500 B');
  });

  it('formats exactly 1 KB', () => {
    expect(formatFileSize(1024)).toBe('1.0 KB');
  });

  it('formats fractional KB', () => {
    expect(formatFileSize(1536)).toBe('1.5 KB');
  });

  it('formats exactly 1 MB', () => {
    expect(formatFileSize(1048576)).toBe('1.0 MB');
  });

  it('formats exactly 1 GB', () => {
    expect(formatFileSize(1073741824)).toBe('1.0 GB');
  });
});

describe('formatMetadataSource', () => {
  it('formats embedded source', () => {
    expect(formatMetadataSource({ type: 'embedded' })).toBe('Embedded');
  });

  it('formats filename source', () => {
    expect(formatMetadataSource({ type: 'filename' })).toBe('Filename');
  });

  it('formats provider source with name via providerLabel', () => {
    expect(formatMetadataSource({ type: 'provider', name: 'open_library' })).toBe('Open Library');
  });

  it('formats provider source for loc as Library of Congress', () => {
    expect(formatMetadataSource({ type: 'provider', name: 'loc' })).toBe('Library of Congress');
  });

  it('formats provider source without name', () => {
    expect(formatMetadataSource({ type: 'provider' })).toBe('Provider');
  });

  it('formats user source', () => {
    expect(formatMetadataSource({ type: 'user' })).toBe('User');
  });

  it('formats content_scan source', () => {
    expect(formatMetadataSource({ type: 'content_scan' })).toBe('Content Scan');
  });

  it('falls back to type string for unknown types', () => {
    expect(formatMetadataSource({ type: 'unknown' })).toBe('unknown');
  });
});
