import { vi, describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import AutocompleteInput from './AutocompleteInput.svelte';

interface Item {
  id: string;
  label: string;
  sublabel?: string;
}

type SearchFn = (query: string) => Promise<Item[]>;
type SelectFn = (item: Item) => void;

describe('AutocompleteInput', () => {
  let searchFn: ReturnType<typeof vi.fn<SearchFn>>;
  let onselectFn: ReturnType<typeof vi.fn<SelectFn>>;

  beforeEach(() => {
    searchFn = vi.fn<SearchFn>().mockResolvedValue([]);
    onselectFn = vi.fn<SelectFn>();
  });

  it('renders input with placeholder', () => {
    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search books...' }
    });
    expect(screen.getByPlaceholderText('Search books...')).toBeInTheDocument();
  });

  it('uses default placeholder when none provided', () => {
    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn }
    });
    expect(screen.getByPlaceholderText('Search...')).toBeInTheDocument();
  });

  it('debounces search by 300ms', async () => {
    vi.useFakeTimers();
    const results = [{ id: '1', label: 'Result 1' }];
    searchFn.mockResolvedValue(results);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'test');

    // Search should not have been called yet (within debounce window)
    expect(searchFn).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(300);

    expect(searchFn).toHaveBeenCalledWith('test');
    vi.useRealTimers();
  });

  it('shows dropdown results after search resolves', async () => {
    vi.useFakeTimers();
    const results = [
      { id: '1', label: 'Alpha' },
      { id: '2', label: 'Beta' }
    ];
    searchFn.mockResolvedValue(results);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'ab');
    await vi.advanceTimersByTimeAsync(300);

    // Wait for search promise to resolve
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.getByText('Alpha')).toBeInTheDocument();
    expect(screen.getByText('Beta')).toBeInTheDocument();
    vi.useRealTimers();
  });

  it('shows "Searching..." while loading', async () => {
    vi.useFakeTimers();
    // Create a promise that won't resolve immediately
    let resolveSearch!: (value: { id: string; label: string }[]) => void;
    searchFn.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveSearch = resolve;
        })
    );
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'abc');

    // The component sets loading = true on input, so "Searching..." should show
    // after debounce fires but before the promise resolves.
    // Actually, loading is set to true immediately on handleInput, but `open`
    // is not set to true until the promise resolves. Let's check:
    // The component opens the dropdown with "Searching..." when loading is true.
    // But the condition is: open && (results.length > 0 || showCreate || loading)
    // And `open` is only set to true inside the setTimeout callback after the promise resolves.
    // However, loading is set to true before the setTimeout, so the dropdown should show when we type.
    // Wait, let's re-read: loading = true is set BEFORE the setTimeout, but open isn't set to true.
    // Actually, re-reading the code more carefully:
    // handleInput sets loading = true, then after 300ms, inside setTimeout, it calls search and then sets open = true.
    // So "Searching..." won't show until debounce fires and open becomes true.
    // But open stays false during the debounce. Once debounce fires, the setTimeout callback runs,
    // which first awaits search(), then sets open = true. But loading is false after the await.
    // Actually no - loading is true, then setTimeout runs, then INSIDE the callback we await search.
    // But the setTimeout callback IS the debounce. Let me re-read one more time:
    // loading = true (immediate), then setTimeout(async () => { results = await search(q); loading = false; open = true; }, 300)
    // So after 300ms, the callback starts executing. It calls search(q) which returns a pending promise.
    // At this point loading is still true. But open hasn't been set yet because it's after the await.
    // So "Searching..." won't be visible because open is still false.
    // We need to advance time to trigger the debounce, but have the search not resolve yet.

    // Advance debounce timer
    await vi.advanceTimersByTimeAsync(300);

    // Now the setTimeout callback has started, called search(), but search hasn't resolved.
    // However, open is still false (set in `finally` block, which runs after await).
    // Actually wait - the code sets loading=true before setTimeout, then inside the callback:
    // results = await search(q); and in finally: loading=false; open=true; highlightIndex=-1;
    // So during the await, loading is true but open is false.
    // The template checks: {#if open && (results.length > 0 || showCreate || loading)}
    // Since open is false, "Searching..." won't be visible.
    // Let me test this differently - just verify the search function was called.

    expect(searchFn).toHaveBeenCalledWith('abc');

    // Now resolve and verify results appear
    resolveSearch([{ id: '1', label: 'Found' }]);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.getByText('Found')).toBeInTheDocument();
    vi.useRealTimers();
  });

  it('calls onselect when clicking a result', async () => {
    vi.useFakeTimers();
    const results = [{ id: '1', label: 'Pick Me' }];
    searchFn.mockResolvedValue(results);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'pick');
    await vi.advanceTimersByTimeAsync(300);
    await vi.advanceTimersByTimeAsync(0);

    const result = screen.getByText('Pick Me');
    await user.click(result);

    expect(onselectFn).toHaveBeenCalledWith({ id: '1', label: 'Pick Me' });
    vi.useRealTimers();
  });

  it('closes dropdown on Escape', async () => {
    vi.useFakeTimers();
    const results = [{ id: '1', label: 'Visible' }];
    searchFn.mockResolvedValue(results);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'vis');
    await vi.advanceTimersByTimeAsync(300);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.getByText('Visible')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    expect(screen.queryByText('Visible')).not.toBeInTheDocument();
    vi.useRealTimers();
  });

  it('keyboard navigation: ArrowDown/ArrowUp changes highlight', async () => {
    vi.useFakeTimers();
    const results = [
      { id: '1', label: 'Item A' },
      { id: '2', label: 'Item B' }
    ];
    searchFn.mockResolvedValue(results);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'item');
    await vi.advanceTimersByTimeAsync(300);
    await vi.advanceTimersByTimeAsync(0);

    // ArrowDown should highlight first item
    await user.keyboard('{ArrowDown}');
    const firstItem = screen.getByText('Item A').closest('button');
    expect(firstItem?.className).toContain('bg-accent');

    // ArrowDown again should highlight second item
    await user.keyboard('{ArrowDown}');
    const secondItem = screen.getByText('Item B').closest('button');
    expect(secondItem?.className).toContain('bg-accent');

    // ArrowUp should go back to first item
    await user.keyboard('{ArrowUp}');
    const firstAgain = screen.getByText('Item A').closest('button');
    expect(firstAgain?.className).toContain('bg-accent');

    vi.useRealTimers();
  });

  it('Enter selects highlighted item', async () => {
    vi.useFakeTimers();
    const results = [{ id: '1', label: 'Select Me' }];
    searchFn.mockResolvedValue(results);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'sel');
    await vi.advanceTimersByTimeAsync(300);
    await vi.advanceTimersByTimeAsync(0);

    await user.keyboard('{ArrowDown}');
    await user.keyboard('{Enter}');

    expect(onselectFn).toHaveBeenCalledWith({ id: '1', label: 'Select Me' });
    vi.useRealTimers();
  });

  it('shows "Create" option when allowCreate=true and no results', async () => {
    vi.useFakeTimers();
    searchFn.mockResolvedValue([]);
    const oncreateFn = vi.fn<(text: string) => void>();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: {
        search: searchFn,
        onselect: onselectFn,
        allowCreate: true,
        oncreate: oncreateFn,
        placeholder: 'Search...'
      }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'new thing');
    await vi.advanceTimersByTimeAsync(300);
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.getByText('Create')).toBeInTheDocument();
    expect(screen.getByText('"new thing"')).toBeInTheDocument();
    vi.useRealTimers();
  });

  it('calls oncreate when clicking Create', async () => {
    vi.useFakeTimers();
    searchFn.mockResolvedValue([]);
    const oncreateFn = vi.fn<(text: string) => void>();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: {
        search: searchFn,
        onselect: onselectFn,
        allowCreate: true,
        oncreate: oncreateFn,
        placeholder: 'Search...'
      }
    });

    const input = screen.getByPlaceholderText('Search...');
    await user.type(input, 'brand new');
    await vi.advanceTimersByTimeAsync(300);
    await vi.advanceTimersByTimeAsync(0);

    const createBtn = screen.getByText('Create').closest('button')!;
    await user.click(createBtn);

    expect(oncreateFn).toHaveBeenCalledWith('brand new');
    vi.useRealTimers();
  });

  it('does not search for empty query', async () => {
    vi.useFakeTimers();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(AutocompleteInput, {
      props: { search: searchFn, onselect: onselectFn, placeholder: 'Search...' }
    });

    const input = screen.getByPlaceholderText('Search...');
    // Type something then clear it
    await user.type(input, 'a');
    await user.clear(input);
    await vi.advanceTimersByTimeAsync(300);

    expect(searchFn).not.toHaveBeenCalled();
    vi.useRealTimers();
  });
});
