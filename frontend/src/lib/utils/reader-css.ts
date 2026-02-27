import type { ReaderPreferences } from '$lib/stores/reader.svelte.js';

export const READER_THEMES: Record<string, { bg: string; fg: string; link: string }> = {
	light: { bg: '#ffffff', fg: '#1a1a1a', link: '#0066cc' },
	dark: { bg: '#1a1a1a', fg: '#e0e0e0', link: '#6db3f2' },
	sepia: { bg: '#f4ecd8', fg: '#5b4636', link: '#7b5b3a' }
};

export const FONT_FAMILY_MAP: Record<string, string> = {
	default: 'inherit',
	serif: "'Georgia', 'Times New Roman', serif",
	'sans-serif': "'Inter', 'Helvetica Neue', sans-serif",
	monospace: "'Consolas', 'JetBrains Mono', monospace"
};

export function buildReaderCSS(prefs: ReaderPreferences): string {
	const theme = READER_THEMES[prefs.theme] ?? READER_THEMES.light;
	const fontFamily = FONT_FAMILY_MAP[prefs.fontFamily] ?? 'inherit';

	return `
		@namespace epub "http://www.idpf.org/2007/ops";
		html {
			background-color: ${theme.bg} !important;
			color: ${theme.fg} !important;
		}
		body {
			background-color: ${theme.bg} !important;
			color: ${theme.fg} !important;
			font-family: ${fontFamily} !important;
			font-size: ${prefs.fontSize}% !important;
		}
		a:link {
			color: ${theme.link};
		}
		a:visited {
			color: ${theme.link};
			opacity: 0.8;
		}
		p, li, blockquote, dd {
			line-height: ${prefs.lineHeight};
			hanging-punctuation: allow-end last;
			widows: 2;
		}
		pre {
			white-space: pre-wrap !important;
		}
		aside[epub|type~="endnote"],
		aside[epub|type~="footnote"],
		aside[epub|type~="note"],
		aside[epub|type~="rearnote"] {
			display: none;
		}
	`;
}
