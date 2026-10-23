// Dragging recordings onto sidebar folders (pointer based: the native file
// drop of the window disables HTML drag and drop in WebView2).

import { api } from './api';
import { app } from './store.svelte';

const THRESHOLD = 6;

class RecordingDrag {
  ids = $state<string[]>([]);
  label = $state('');
  x = $state(0);
  y = $state(0);
  active = $state(false);
  /** Folder under the pointer ("" = "no folder" is not a target). */
  overFolder = $state<string | null>(null);
  /** A file dragged in from Explorer is over this folder. */
  fileOverFolder = $state<string | null>(null);

  private startX = 0;
  private startY = 0;
  private pending = false;

  begin(event: PointerEvent, ids: string[], label: string) {
    if (event.button !== 0) return;
    this.pending = true;
    this.ids = ids;
    this.label = label;
    this.startX = event.clientX;
    this.startY = event.clientY;
    window.addEventListener('pointermove', this.move);
    window.addEventListener('pointerup', this.end, { once: true });
  }

  private move = (event: PointerEvent) => {
    if (!this.pending) return;
    if (!this.active && Math.hypot(event.clientX - this.startX, event.clientY - this.startY) < THRESHOLD) return;
    this.active = true;
    this.x = event.clientX;
    this.y = event.clientY;
    this.overFolder = folderAt(event.clientX, event.clientY);
  };

  private end = async () => {
    window.removeEventListener('pointermove', this.move);
    const target = this.active ? this.overFolder : null;
    const ids = this.ids;
    this.pending = false;
    this.active = false;
    this.overFolder = null;
    if (target) await app.perform(() => api.moveToFolder(ids, target));
  };

  /** True right after a drag, so the row's click doesn't select. */
  get dragging(): boolean {
    return this.active;
  }
}

/** The folder row at a point of the page, if any. */
export function folderAt(x: number, y: number): string | null {
  const element = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-folder-id]');
  return element?.dataset.folderId ?? null;
}

export const recordingDrag = new RecordingDrag();
