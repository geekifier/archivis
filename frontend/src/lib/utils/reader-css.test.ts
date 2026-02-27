import { describe, it, expect } from 'vitest';
import { buildReaderCSS, READER_THEMES, FONT_FAMILY_MAP } from './reader-css.js';
import type { ReaderPreferences } from '$lib/stores/reader.svelte.js';

function defaultPrefs(): ReaderPreferences {
	return {
		fontSize: 100,
		fontFamily: 'default',
		lineHeight: 1.4,
		theme: 'light',
		flow: 'paginated',
		margins: 40,
		maxWidth: 800,
		maxColumns: 1
	};
}

describe('buildReaderCSS', () => {
	it('produces correct CSS for default preferences', () => {
		const css = buildReaderCSS(defaultPrefs());

		expect(css).toContain('font-size: 100%');
		expect(css).toContain(`background-color: ${READER_THEMES.light.bg}`);
		expect(css).toContain(`color: ${READER_THEMES.light.fg}`);
		expect(css).toContain(`color: ${READER_THEMES.light.link}`);
		expect(css).toContain('font-family: inherit');
		expect(css).toContain('line-height: 1.4');
	});

	it('reflects a changed fontSize', () => {
		const prefs = { ...defaultPrefs(), fontSize: 150 };
		const css = buildReaderCSS(prefs);

		expect(css).toContain('font-size: 150%');
		expect(css).not.toContain('font-size: 100%');
	});

	it('maps serif fontFamily correctly', () => {
		const prefs = { ...defaultPrefs(), fontFamily: 'serif' };
		const css = buildReaderCSS(prefs);

		expect(css).toContain(FONT_FAMILY_MAP.serif);
	});

	it('maps sans-serif fontFamily correctly', () => {
		const prefs = { ...defaultPrefs(), fontFamily: 'sans-serif' };
		const css = buildReaderCSS(prefs);

		expect(css).toContain(FONT_FAMILY_MAP['sans-serif']);
	});

	it('maps monospace fontFamily correctly', () => {
		const prefs = { ...defaultPrefs(), fontFamily: 'monospace' };
		const css = buildReaderCSS(prefs);

		expect(css).toContain(FONT_FAMILY_MAP.monospace);
	});

	it('produces correct colors for dark theme', () => {
		const prefs = { ...defaultPrefs(), theme: 'dark' as const };
		const css = buildReaderCSS(prefs);

		expect(css).toContain(`background-color: ${READER_THEMES.dark.bg}`);
		expect(css).toContain(`color: ${READER_THEMES.dark.fg}`);
		expect(css).toContain(`color: ${READER_THEMES.dark.link}`);
	});

	it('produces correct colors for sepia theme', () => {
		const prefs = { ...defaultPrefs(), theme: 'sepia' as const };
		const css = buildReaderCSS(prefs);

		expect(css).toContain(`background-color: ${READER_THEMES.sepia.bg}`);
		expect(css).toContain(`color: ${READER_THEMES.sepia.fg}`);
		expect(css).toContain(`color: ${READER_THEMES.sepia.link}`);
	});

	it('falls back to light theme for unknown theme', () => {
		const prefs = { ...defaultPrefs(), theme: 'neon' as ReaderPreferences['theme'] };
		const css = buildReaderCSS(prefs);

		expect(css).toContain(`background-color: ${READER_THEMES.light.bg}`);
		expect(css).toContain(`color: ${READER_THEMES.light.fg}`);
	});

	it('falls back to inherit for unknown fontFamily', () => {
		const prefs = { ...defaultPrefs(), fontFamily: 'comic-sans' };
		const css = buildReaderCSS(prefs);

		expect(css).toContain('font-family: inherit');
	});

	it('uses !important on critical properties', () => {
		const css = buildReaderCSS(defaultPrefs());

		expect(css).toContain('background-color: #ffffff !important');
		expect(css).toContain('color: #1a1a1a !important');
		expect(css).toContain('font-family: inherit !important');
		expect(css).toContain('font-size: 100% !important');
		expect(css).toContain('white-space: pre-wrap !important');
	});

	it('includes epub namespace declaration', () => {
		const css = buildReaderCSS(defaultPrefs());

		expect(css).toContain('@namespace epub "http://www.idpf.org/2007/ops"');
	});

	it('includes aside footnote/endnote hiding rules', () => {
		const css = buildReaderCSS(defaultPrefs());

		expect(css).toContain('aside[epub|type~="endnote"]');
		expect(css).toContain('aside[epub|type~="footnote"]');
		expect(css).toContain('display: none');
	});
});

describe('READER_THEMES', () => {
	it('has light, dark, and sepia themes', () => {
		expect(READER_THEMES).toHaveProperty('light');
		expect(READER_THEMES).toHaveProperty('dark');
		expect(READER_THEMES).toHaveProperty('sepia');
	});

	it('each theme has bg, fg, and link colors', () => {
		for (const [, theme] of Object.entries(READER_THEMES)) {
			expect(theme).toHaveProperty('bg');
			expect(theme).toHaveProperty('fg');
			expect(theme).toHaveProperty('link');
			expect(theme.bg).toMatch(/^#[0-9a-f]{6}$/);
			expect(theme.fg).toMatch(/^#[0-9a-f]{6}$/);
			expect(theme.link).toMatch(/^#[0-9a-f]{6}$/);
		}
	});
});

describe('FONT_FAMILY_MAP', () => {
	it('has default, serif, sans-serif, and monospace entries', () => {
		expect(FONT_FAMILY_MAP).toHaveProperty('default');
		expect(FONT_FAMILY_MAP).toHaveProperty('serif');
		expect(FONT_FAMILY_MAP).toHaveProperty('sans-serif');
		expect(FONT_FAMILY_MAP).toHaveProperty('monospace');
	});

	it('default maps to inherit', () => {
		expect(FONT_FAMILY_MAP.default).toBe('inherit');
	});
});
