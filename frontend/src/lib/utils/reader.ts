/**
 * Wraps a Blob as a File with a synthetic name so that foliate-js
 * can detect the book format via `name.endsWith(...)`.
 */
export function blobToBookFile(blob: Blob, format: string): File {
	return new File([blob], `book.${format}`, { type: blob.type });
}
