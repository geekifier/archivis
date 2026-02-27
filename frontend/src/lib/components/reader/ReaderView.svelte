<script lang="ts">
	import { onMount } from 'svelte';
	import { reader } from '$lib/stores/reader.svelte.js';
	import type { TocItem } from '$lib/api/types.js';
	import { buildReaderCSS } from '$lib/utils/reader-css.js';

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
		onError?: (error: Error) => void;
	}

	let { bookBlob, savedLocation = null, onRelocate, onTocLoaded, onLoad, onError }: Props = $props();

	let container = $state<HTMLDivElement | null>(null);
	let view: FoliateView | null = null;
	let bookOpened = false;

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
			const css = buildReaderCSS(reader.preferences);
			view.renderer.setStyles(css);
		} catch {
			// Style injection may fail for some formats
		}
	}

	/**
	 * Apply reader CSS directly to a section's document.
	 *
	 * foliate-js creates two <style> elements per section ($styleBefore at top,
	 * $style at bottom of <head>) and re-applies stored styles via setStyles()
	 * in its onLoad callback. However, setStyles() looks up the current view's
	 * document via this.#view, which still points to the *previous* section at
	 * callback time — so the styles silently fail to apply to the new section.
	 *
	 * We work around this by finding foliate's own $style element (last <style>
	 * without attributes in <head>) and writing our CSS into it. This avoids a
	 * duplicate <style> element, so subsequent setStyles() calls (triggered by
	 * $effect when preferences change) update the same element with no conflict.
	 */
	function applyStylesToDocument(doc: Document): void {
		try {
			const css = buildReaderCSS(reader.preferences);
			// Find the last bare <style> in <head> — that's foliate's $style element.
			const styles = doc.head?.querySelectorAll('style:not([data-archivis])');
			const target = styles?.[styles.length - 1];
			if (target) {
				target.textContent = css;
			}
		} catch {
			// Style injection may fail for some formats
		}
	}

	// Reactively apply preference changes.
	// IMPORTANT: read all reactive dependencies BEFORE the early-return guard.
	// `view` and `bookOpened` are plain variables (not $state) — Svelte can't
	// track them, so if the effect returns before reaching reactive reads it
	// will never re-run.  By subscribing to preferences unconditionally the
	// effect stays alive and fires whenever any preference changes.
	$effect(() => {
		const _prefs = reader.preferences;
		void _prefs.fontSize;
		void _prefs.fontFamily;
		void _prefs.lineHeight;
		void _prefs.theme;
		void _prefs.flow;
		void _prefs.margins;
		void _prefs.maxWidth;
		void _prefs.maxColumns;

		if (!view || !bookOpened) return;

		applyRendererAttributes();
		injectCSS();

		// Also apply directly to currently loaded section documents as a
		// fallback — setStyles() may silently fail if the paginator's
		// internal view reference is stale (see applyStylesToDocument docs).
		try {
			for (const { doc } of view.renderer.getContents()) {
				applyStylesToDocument(doc);
			}
		} catch {
			// getContents may not be available for some renderers
		}
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

		try {
			// Load foliate-js view.js as a module script
			await loadScript('/vendor/foliate-js/view.js');
		} catch (err: unknown) {
			onError?.(err instanceof Error ? err : new Error('Failed to load script: /vendor/foliate-js/view.js'));
			return;
		}

		try {
			// Create the custom element
			const el = document.createElement('foliate-view') as FoliateView;
			el.style.width = '100%';
			el.style.height = '100%';

			// Listen for relocate events
			el.addEventListener('relocate', (e: Event) => {
				const detail = (e as CustomEvent).detail as RelocateDetail;
				onRelocate?.(detail);
			});

			// Apply reader styles to each newly loaded section.
			// foliate-js's internal setStyles() has a timing issue where
			// the view reference hasn't been updated yet, so we apply
			// styles directly to the document's existing <style> element.
			el.addEventListener('load', (e: Event) => {
				const { doc } = (e as CustomEvent).detail as { doc: Document; index: number };
				applyStylesToDocument(doc);
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
		} catch (err: unknown) {
			const msg = err instanceof Error ? err.message : 'Unknown error opening book';
			onError?.(new Error(msg));
		}
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
