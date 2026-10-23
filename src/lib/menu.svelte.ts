// Context menus and dropdown menus (one open at a time).

import type { IconName } from './icons';

export type MenuEntry =
  | {
      label: string;
      icon?: IconName;
      /** A colored dot instead of an icon (speakers). */
      dot?: string;
      action?: () => void;
      disabled?: boolean;
      checked?: boolean;
      danger?: boolean;
      hint?: string;
      submenu?: MenuEntry[];
    }
  | { separator: true }
  | { header: string };

interface OpenMenu {
  x: number;
  y: number;
  /** Opened from a button: align below it. */
  below?: DOMRect;
  entries: MenuEntry[];
  onClose?: () => void;
}

class MenuState {
  current = $state<OpenMenu | null>(null);

  /** At the mouse position (context menu). */
  at(event: MouseEvent, entries: MenuEntry[]) {
    event.preventDefault();
    event.stopPropagation();
    this.current = { x: event.clientX, y: event.clientY, entries };
  }

  /** Below a button (dropdown). */
  below(element: Element, entries: MenuEntry[], onClose?: () => void) {
    const rect = element.getBoundingClientRect();
    this.current = { x: rect.left, y: rect.bottom + 4, below: rect, entries, onClose };
  }

  close() {
    const onClose = this.current?.onClose;
    this.current = null;
    onClose?.();
  }
}

export const menu = new MenuState();
