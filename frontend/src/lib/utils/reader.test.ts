import { describe, it, expect } from 'vitest';
import { blobToBookFile } from './reader.js';

describe('blobToBookFile', () => {
  it('creates a File with correct name for epub', () => {
    const blob = new Blob(['fake epub content'], { type: 'application/epub+zip' });
    const file = blobToBookFile(blob, 'epub');
    expect(file.name).toBe('book.epub');
    expect(file.type).toBe('application/epub+zip');
  });

  it('preserves blob size', () => {
    const content = 'some binary data here';
    const blob = new Blob([content], { type: 'application/pdf' });
    const file = blobToBookFile(blob, 'pdf');
    expect(file.size).toBe(blob.size);
  });

  it('works for all readable formats', () => {
    const formats = ['epub', 'pdf', 'mobi', 'azw3', 'fb2', 'cbz'];
    for (const fmt of formats) {
      const blob = new Blob(['data'], { type: 'application/octet-stream' });
      const file = blobToBookFile(blob, fmt);
      expect(file.name).toBe(`book.${fmt}`);
      expect(file).toBeInstanceOf(File);
      expect(file).toBeInstanceOf(Blob);
    }
  });

  it('preserves the original blob MIME type', () => {
    const blob = new Blob(['data'], { type: 'application/x-mobipocket-ebook' });
    const file = blobToBookFile(blob, 'mobi');
    expect(file.type).toBe('application/x-mobipocket-ebook');
  });
});
