<script lang="ts">
  interface Props {
    progress: number;
    currentChapter: string | null;
    toolbarVisible: boolean;
    onScrub: (fraction: number) => void;
  }

  let { progress, currentChapter, toolbarVisible, onScrub }: Props = $props();

  let barElement = $state<HTMLDivElement | null>(null);
  let isDragging = $state(false);
  let dragFraction = $state(0);

  const displayFraction = $derived(isDragging ? dragFraction : progress);
  const percentText = $derived(`${Math.round(displayFraction * 100)}%`);

  function fractionFromEvent(e: MouseEvent | PointerEvent): number {
    if (!barElement) return 0;
    const rect = barElement.getBoundingClientRect();
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    return x / rect.width;
  }

  function handlePointerDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    e.preventDefault();
    isDragging = true;
    dragFraction = fractionFromEvent(e);
    barElement?.setPointerCapture(e.pointerId);
  }

  function handlePointerMove(e: PointerEvent): void {
    if (!isDragging) return;
    e.preventDefault();
    dragFraction = fractionFromEvent(e);
  }

  function handlePointerUp(e: PointerEvent): void {
    if (!isDragging) return;
    e.preventDefault();
    const fraction = fractionFromEvent(e);
    isDragging = false;
    onScrub(fraction);
  }

  function handleClick(e: MouseEvent): void {
    if (isDragging) return;
    const fraction = fractionFromEvent(e);
    onScrub(fraction);
  }

  function handleKeydown(e: KeyboardEvent): void {
    const STEP = 0.02;
    switch (e.key) {
      case 'ArrowRight':
      case 'ArrowUp':
        e.preventDefault();
        onScrub(Math.min(1, displayFraction + STEP));
        break;
      case 'ArrowLeft':
      case 'ArrowDown':
        e.preventDefault();
        onScrub(Math.max(0, displayFraction - STEP));
        break;
      case 'Home':
        e.preventDefault();
        onScrub(0);
        break;
      case 'End':
        e.preventDefault();
        onScrub(1);
        break;
    }
  }
</script>

<div class="absolute inset-x-0 bottom-0 z-20 select-none" class:pb-0={!toolbarVisible}>
  <!-- Clickable/draggable bar area -->
  <div
    bind:this={barElement}
    class="group relative cursor-pointer"
    class:py-1={toolbarVisible}
    onclick={handleClick}
    onkeydown={handleKeydown}
    onpointerdown={handlePointerDown}
    onpointermove={handlePointerMove}
    onpointerup={handlePointerUp}
    onpointercancel={() => {
      isDragging = false;
    }}
    role="slider"
    aria-label="Reading progress"
    aria-valuenow={Math.round(displayFraction * 100)}
    aria-valuemin={0}
    aria-valuemax={100}
    tabindex={0}
  >
    <!-- Track -->
    <div
      class="w-full transition-all duration-200"
      class:h-[3px]={!toolbarVisible}
      class:h-1.5={toolbarVisible}
    >
      <div class="h-full w-full rounded-full bg-primary/20">
        <div
          class="h-full rounded-full bg-primary transition-[width] duration-75"
          style:width="{displayFraction * 100}%"
        ></div>
      </div>
    </div>
  </div>

  <!-- Expanded info below bar when toolbar is visible -->
  {#if toolbarVisible}
    <div
      class="hidden items-center justify-between bg-background/90 px-4 py-1 text-xs backdrop-blur-sm sm:flex"
    >
      <span class="min-w-0 truncate text-muted-foreground">
        {currentChapter ?? ''}
      </span>
      <span class="shrink-0 tabular-nums text-muted-foreground">
        {percentText}
      </span>
    </div>
  {/if}
</div>
