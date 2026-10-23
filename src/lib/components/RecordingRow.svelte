<script lang="ts">
  import { recordingMenu, remove } from '../actions';
  import { recordingDrag } from '../drag.svelte';
  import { duration, listDate } from '../format';
  import { t } from '../i18n';
  import { menu } from '../menu.svelte';
  import { app } from '../store.svelte';
  import type { RecordingSummary } from '../types';
  import Icon from './Icon.svelte';
  import SourceIcon from './SourceIcon.svelte';
  import StatusBadge from './StatusBadge.svelte';

  let { recording, selected }: { recording: RecordingSummary; selected: boolean } = $props();

  /** Swipe left (touchpad, touch or pen) reveals the delete button. */
  const REVEAL = 88;
  let offset = $state(0);
  let swipe: { x: number; start: number; id: number } | null = null;
  let wheelTimer = 0;

  function settle() {
    offset = offset > REVEAL / 2 ? REVEAL : 0;
  }

  function wheel(event: WheelEvent) {
    if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    event.preventDefault();
    offset = Math.max(0, Math.min(REVEAL * 1.4, offset + event.deltaX));
    clearTimeout(wheelTimer);
    wheelTimer = window.setTimeout(settle, 120);
  }

  function pointerdown(event: PointerEvent) {
    if (event.pointerType === 'mouse') {
      if (offset > 0) {
        offset = 0;
        return;
      }
      recordingDrag.begin(event, [recording.id], recording.title);
      return;
    }
    swipe = { x: event.clientX, start: offset, id: event.pointerId };
  }

  function pointermove(event: PointerEvent) {
    if (!swipe || event.pointerId !== swipe.id) return;
    offset = Math.max(0, Math.min(REVEAL * 1.4, swipe.start + swipe.x - event.clientX));
  }

  function pointerup() {
    if (!swipe) return;
    swipe = null;
    settle();
  }
</script>

<div class="wrap" onwheel={wheel}>
  <button class="delete" tabindex="-1" style:visibility={offset > 0 ? 'visible' : 'hidden'} onclick={() => { offset = 0; void remove(recording); }}>
    <Icon name="delete" />
    <span class="caption">{t('Löschen')}</span>
  </button>
  <div
    class="row"
    class:selected
    role="option"
    aria-selected={selected}
    tabindex="-1"
    data-recording-id={recording.id}
    style:transform="translateX({-offset}px)"
    onpointerdown={pointerdown}
    onpointermove={pointermove}
    onpointerup={pointerup}
    onpointercancel={pointerup}
    onclick={() => {
      if (offset === 0 && !recordingDrag.dragging) app.select(recording.id);
    }}
    ondblclick={() => {
      app.select(recording.id);
      setTimeout(() => app.player.play(), 150);
    }}
    oncontextmenu={(event) => {
      app.select(recording.id);
      menu.at(event, recordingMenu(recording));
    }}
    onkeydown={() => {}}
  >
    <SourceIcon source={recording.source} />
    <div class="text">
      <div class="row-title">{recording.title}</div>
      <div class="meta caption secondary">
        <span>{listDate(recording.createdAt)}</span>
        <span>·</span>
        <span class="tabular">{duration(recording.duration)}</span>
      </div>
    </div>
    {#if recording.status === 'done'}
      <span class="done" title={t('Fertig')}><Icon name="checkmarkCircle" /></span>
    {:else}
      <StatusBadge status={recording.status} progress={app.progress[recording.id] ?? 0} compact />
    {/if}
  </div>
</div>

<style>
  .wrap {
    position: relative;
    border-radius: var(--radius);
    overflow: hidden;
    margin: 2px 0;
  }
  .delete {
    position: absolute;
    inset: 0 0 0 auto;
    width: 88px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    border: none;
    background: var(--red);
    color: white;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px 8px 12px;
    border-radius: var(--radius);
    background: var(--layer-solid);
    transition: transform 0.18s ease-out;
    touch-action: pan-y;
  }
  :global(html.mica) .row {
    background: color-mix(in srgb, var(--layer-solid) 92%, transparent);
  }
  .row:hover {
    background: color-mix(in srgb, var(--layer-solid), var(--text) 4%);
  }
  .row.selected {
    background: color-mix(in srgb, var(--layer-solid), var(--text) 7%);
  }
  .row.selected::before {
    content: '';
    position: absolute;
    left: 0;
    top: 50%;
    width: 3px;
    height: 16px;
    margin-top: -8px;
    border-radius: 2px;
    background: var(--accent);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .row-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    gap: 4px;
    white-space: nowrap;
  }
  .done {
    display: inline-flex;
    color: var(--text-3);
  }
</style>
