type Theme = 'light' | 'dark';

function createTheme() {
	let current = $state<Theme>('light');

	function apply(theme: Theme) {
		current = theme;
		if (typeof document !== 'undefined') {
			document.documentElement.classList.toggle('dark', theme === 'dark');
			localStorage.setItem('archivis-theme', theme);
		}
	}

	function init() {
		if (typeof window === 'undefined') return;
		const stored = localStorage.getItem('archivis-theme') as Theme | null;
		if (stored) {
			apply(stored);
		} else {
			const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
			apply(prefersDark ? 'dark' : 'light');
		}
	}

	function toggle() {
		apply(current === 'dark' ? 'light' : 'dark');
	}

	return {
		get current() {
			return current;
		},
		init,
		toggle
	};
}

export const theme = createTheme();
