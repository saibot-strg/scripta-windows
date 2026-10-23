<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    width = 480,
    onclose,
    children,
    footer,
  }: {
    title?: string;
    width?: number;
    /** Escape and the backdrop close the dialog if set. */
    onclose?: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === 'Escape' && onclose) {
      event.preventDefault();
      onclose();
    }
  }}
/>

<div class="smoke" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} style:width="{width}px">
    <div class="content">
      {#if title}<h2 class="subtitle">{title}</h2>{/if}
      {@render children()}
    </div>
    {#if footer}
      <div class="footer">{@render footer()}</div>
    {/if}
  </div>
</div>

<style>
  .smoke {
    position: fixed;
    inset: 0;
    z-index: 800;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--smoke);
    animation: fade 0.15s ease-out;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .dialog {
    max-width: calc(100vw - 48px);
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    background: var(--card-solid);
    border: 1px solid var(--stroke-strong);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-dialog);
    overflow: hidden;
    animation: pop 0.18s cubic-bezier(0, 0, 0, 1);
  }
  @keyframes pop {
    from {
      transform: scale(1.05);
      opacity: 0;
    }
  }
  .content {
    padding: 24px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0;
  }
  .footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
    padding: 24px;
    background: var(--solid-bg);
    border-top: 1px solid var(--stroke);
  }
  .footer :global(.button) {
    min-width: 120px;
  }
</style>
