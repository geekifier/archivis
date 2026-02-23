import { type ClassValue, clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

export type WithElementRef<T, E extends HTMLElement = HTMLElement> = T & {
	ref?: E | null;
};

export type { WithoutChild, WithoutChildrenOrChild } from 'bits-ui';

/** Generate a deterministic hue (0-359) from a string ID for cover placeholders. */
export function placeholderHue(id: string): number {
	let hash = 0;
	for (let i = 0; i < id.length; i++) {
		hash = (hash * 31 + id.charCodeAt(i)) | 0;
	}
	return Math.abs(hash) % 360;
}

/** Format a byte count into a human-readable string. */
export function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Format an identifier type slug into a display label. */
export function formatIdentifierType(type: string): string {
	const labels: Record<string, string> = {
		isbn13: 'ISBN-13',
		isbn10: 'ISBN-10',
		asin: 'ASIN',
		google_books: 'Google Books',
		open_library: 'Open Library',
		hardcover: 'Hardcover'
	};
	return labels[type] ?? type;
}

/** Format a MetadataSource object into a display string. */
export function formatMetadataSource(source: { type: string; name?: string }): string {
	switch (source.type) {
		case 'embedded':
			return 'Embedded';
		case 'filename':
			return 'Filename';
		case 'provider':
			return source.name ?? 'Provider';
		case 'user':
			return 'User';
		case 'content_scan':
			return 'Content Scan';
		default:
			return source.type;
	}
}
