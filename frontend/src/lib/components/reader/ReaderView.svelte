<script lang="ts">
	import { onMount } from 'svelte';
	import { reader } from '$lib/stores/reader.svelte.js';
	import type { TocItem } from '$lib/api/types.js';

	interface RelocateDetail {
		cfi: string | null;
		fraction: number;
		tocItem: { label: string; href: string } | null;
		pageItem: { label: string } | null;
		location: { current: number; total: number } | null;
	}

	interface FoliateView extends HTMLElement {
		open: (blob: Blob) => Promise<void>;
		close: () => void;
		init: (opts: { lastLocation?: string; showTextStart?: boolean }) => Promise<void>;
		goTo: (target: string | number) => Promise<void>;
		goToFraction: (frac: number) => Promise<void>;
		next: (distance?: number) => Promise<void>;
		prev: (distance?: number) => Promise<void>;
		renderer: {
			setStyles: (css: string) => void;
			setAttribute: (name: string, value: string) => void;
			getContents: () => Array<{ doc: Document; index: number }>;
		};
		book: {
			toc?: TocItem[];
			dir?: string;
			metadata?: { language?: string };
		};
	}

	interface Props {
		bookBlob: Blob;
		savedLocation?: string | null;
		onRelocate?: (detail: RelocateDetail) => void;
		onTocLoaded?: (toc: TocItem[]) => void;
		onLoad?: () => void;
	}

	let { bookBlob, savedLocation = null, onRelocate, onTocLoaded, onLoad }: Props = $props();

	let container = $state<HTMLDivElement | null>(null);
	let view: FoliateView | null = null;
	let bookOpened = false;

	const themes: Record<string, { bg: string; fg: string; link: string }> = {
		light: { bg: '#ffffff', fg: '#1a1a1a', link: '#0066cc' },
		dark: { bg: '#1a1a1a', fg: '#e0e0e0', link: '#6db3f2' },
		sepia: { bg: '#f4ecd8', fg: '#5b4636', link: '#7b5b3a' }
	};

	const fontMap: Record<string, string> = {
		default: 'inherit',
		serif: "'Georgia', 'Times New Roman', serif",
		'sans-serif': "'Inter', 'Helvetica Neue', sans-serif",
		monospace: "'Consolas', 'JetBrains Mono', monospace"
	};

	function buildReaderCSS(): string {
		const prefs = reader.preferences;
		const theme = themes[prefs.theme] ?? themes.light;
		const fontFamily = fontMap[prefs.fontFamily] ?? 'inherit';

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

	function applyRendererAttributes(): void {
		if (!view?.renderer) return;
		const prefs = reader.preferences;
		try {
			view.renderer.setAttribute('flow', prefs.flow);
			view.renderer.setAttribute('margin', `${prefs.margins}px`);
			view.renderer.setAttribute('max-inline-size', `${prefs.maxWidth}px`);
			view.renderer.setAttribute('max-column-count', String(prefs.maxColumns));
		} catch {
			// Renderer may not support all attributes
		}
	}

	function injectCSS(): void {
		if (!view?.renderer) return;
		try {
			const css = buildReaderCSS();
			view.renderer.setStyles(css);
		} catch {
			// Style injection may fail for some formats
		}
	}

	// Reactively apply preference changes
	$effect(() => {
		if (!view || !bookOpened) return;
		// Access all preference properties to subscribe
		const _prefs = reader.preferences;
		void _prefs.fontSize;
		void _prefs.fontFamily;
		void _prefs.lineHeight;
		void _prefs.theme;
		void _prefs.flow;
		void _prefs.margins;
		void _prefs.maxWidth;
		void _prefs.maxColumns;

		applyRendererAttributes();
		injectCSS();
	});

	onMount(() => {
		void initReader();

		return () => {
			if (view) {
				try {
					view.close();
				} catch {
					// May already be cleaned up
				}
				view = null;
				bookOpened = false;
			}
		};
	});

	async function initReader(): Promise<void> {
		if (!container) return;

		// Load foliate-js view.js as a module script
		await loadScript('/vendor/foliate-js/view.js');

		// Create the custom element
		const el = document.createElement('foliate-view') as FoliateView;
		el.style.width = '100%';
		el.style.height = '100%';

		// Listen for relocate events
		el.addEventListener('relocate', (e: Event) => {
			const detail = (e as CustomEvent).detail as RelocateDetail;
			onRelocate?.(detail);
		});

		// Listen for load events (fired per section)
		el.addEventListener('load', (e: Event) => {
			const { doc } = (e as CustomEvent).detail as { doc: Document; index: number };
			// Inject theme CSS into each loaded document
			injectDocumentCSS(doc);
		});

		// eslint-disable-next-line svelte/no-dom-manipulating -- foliate-js custom element must be mounted imperatively
		container.append(el);
		view = el;

		// Open the book
		await el.open(bookBlob);
		bookOpened = true;

		// Extract TOC from the book object
		if (el.book?.toc) {
			onTocLoaded?.(el.book.toc);
		}

		// Apply initial renderer attributes and styles
		applyRendererAttributes();
		injectCSS();

		// Initialize position
		if (savedLocation) {
			await el.init({ lastLocation: savedLocation });
		} else {
			await el.init({ showTextStart: true });
		}

		onLoad?.();
	}

	function injectDocumentCSS(doc: Document): void {
		const prefs = reader.preferences;
		const theme = themes[prefs.theme] ?? themes.light;
		const fontFamily = fontMap[prefs.fontFamily] ?? 'inherit';

		const style = doc.createElement('style');
		style.setAttribute('data-archivis-reader', 'true');
		style.textContent = `
			html {
				background-color: ${theme.bg} !important;
				color: ${theme.fg} !important;
			}
			body {
				background-color: ${theme.bg} !important;
				color: ${theme.fg} !important;
				font-family: ${fontFamily} !important;
				font-size: ${prefs.fontSize}% !important;
				line-height: ${prefs.lineHeight} !important;
			}
			a:link { color: ${theme.link}; }
			a:visited { color: ${theme.link}; opacity: 0.8; }
		`;
		doc.head.append(style);
	}

	function loadScript(src: string): Promise<void> {
		return new Promise((resolve, reject) => {
			if (document.querySelector(`script[src="${src}"]`)) {
				resolve();
				return;
			}
			const script = document.createElement('script');
			script.type = 'module';
			script.src = src;
			script.onload = () => resolve();
			script.onerror = () => reject(new Error(`Failed to load script: ${src}`));
			document.head.append(script);
		});
	}

	// Public methods exposed to parent via bind:this
	export function next(): void {
		view?.next();
	}

	export function prev(): void {
		view?.prev();
	}

	export function goTo(target: string | number): void {
		view?.goTo(target);
	}

	export function goToFraction(f: number): void {
		view?.goToFraction(f);
	}
</script>

<div bind:this={container} class="h-full w-full overflow-hidden"></div>
