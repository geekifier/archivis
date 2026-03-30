import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import SearchWarnings from './SearchWarnings.svelte';
import type { QueryWarning } from '$lib/api/types.js';

describe('SearchWarnings', () => {
  it('renders no_searchable_terms warning without field', () => {
    const warnings: QueryWarning[] = [{ type: 'no_searchable_terms', text: '--' }];
    render(SearchWarnings, { props: { warnings } });
    expect(screen.getByText('--')).toBeInTheDocument();
    expect(screen.getByText('— no searchable terms')).toBeInTheDocument();
  });

  it('renders no_searchable_terms warning with field', () => {
    const warnings: QueryWarning[] = [
      { type: 'no_searchable_terms', text: '--', field: 'title' }
    ];
    render(SearchWarnings, { props: { warnings } });
    expect(screen.getByText('title:--')).toBeInTheDocument();
    expect(screen.getByText('— no searchable terms')).toBeInTheDocument();
  });

  it('renders nothing when warnings array is empty', () => {
    const { container } = render(SearchWarnings, { props: { warnings: [] } });
    expect(container.textContent).toBe('');
  });
});
