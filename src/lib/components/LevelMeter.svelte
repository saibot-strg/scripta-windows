<script lang="ts">
  /** Scrolling level bars like in Voice Memos: newest level on the right. */
  let {
    levels,
    color = 'var(--red)',
    height = 24,
    bar = 3,
    gap = 2,
    width = 140,
  }: { levels: number[]; color?: string; height?: number; bar?: number; gap?: number; width?: number } = $props();

  const slots = $derived(Math.max(1, Math.floor((width + gap) / (bar + gap))));
  const recent = $derived(levels.slice(-slots));
  const empty = $derived(slots - recent.length);
</script>

<svg {width} {height} aria-hidden="true">
  {#each { length: slots } as _, slot}
    {@const level = slot < empty ? 0 : recent[slot - empty]}
    {@const h = Math.max(bar, level * height)}
    <rect
      x={slot * (bar + gap)}
      y={(height - h) / 2}
      width={bar}
      height={h}
      rx={bar / 2}
      fill={slot < empty ? 'var(--text-3)' : color}
      opacity={slot < empty ? 0.35 : 1}
    />
  {/each}
</svg>
