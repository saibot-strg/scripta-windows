<script lang="ts">
  import { tick } from 'svelte';
  import { menu, type MenuEntry } from '../menu.svelte';
  import Icon from './Icon.svelte';

  let element = $state<HTMLDivElement>();
  let position = $state({ left: 0, top: 0 });
  let active = $state(-1);
  let submenu = $state<{ index: number; left: number; top: number } | null>(null);

  const current = $derived(menu.current);

  $effect(() => {
    if (!current) return;
    active = -1;
    submenu = null;
    position = { left: current.x, top: current.y };
    void tick().then(() => {
      if (!element) return;
      const rect = element.getBoundingClientRect();
      let left = current.x;
      let top = current.y;
      if (left + rect.width > window.innerWidth - 8) left = Math.max(8, window.innerWidth - rect.width - 8);
      if (top + rect.height > window.innerHeight - 8) {
        top = current.below ? current.below.top - rect.height - 4 : window.innerHeight - rect.height - 8;
      }
      position = { left, top: Math.max(8, top) };
      element.focus();
    });
  });

  function choose(entry: MenuEntry) {
    if ('separator' in entry || 'header' in entry || entry.disabled || entry.submenu) return;
    menu.close();
    entry.action?.();
  }

  function selectable(entries: MenuEntry[]): number[] {
    return entries.flatMap((entry, index) =>
      'separator' in entry || 'header' in entry || entry.disabled ? [] : [index],
    );
  }

  function keydown(event: KeyboardEvent) {
    if (!current) return;
    const indices = selectable(current.entries);
    if (event.key === 'Escape') {
      event.preventDefault();
      menu.close();
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const position = indices.indexOf(active);
      const next = event.key === 'ArrowDown' ? position + 1 : position - 1;
      active = indices[(next + indices.length) % indices.length] ?? -1;
    } else if (event.key === 'Enter' && active >= 0) {
      event.preventDefault();
      choose(current.entries[active]);
    }
  }

  function openSubmenu(index: number, target: HTMLElement) {
    const rect = target.getBoundingClientRect();
    submenu = { index, left: rect.right - 4, top: rect.top - 5 };
  }

  const submenuEntries = $derived.by(() => {
    if (!current || !submenu) return null;
    const entry = current.entries[submenu.index];
    return entry && 'submenu' in entry ? (entry.submenu ?? null) : null;
  });
</script>

<svelte:window
  onkeydown={(event) => current && keydown(event)}
  onblur={() => current && menu.close()}
  onresize={() => current && menu.close()}
/>

{#if current}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="backdrop"
    onpointerdown={() => menu.close()}
    oncontextmenu={(event) => {
      event.preventDefault();
      menu.close();
    }}
  ></div>
  {#snippet list(entries: MenuEntry[], nested: boolean)}
    {#each entries as entry, index}
      {#if 'separator' in entry}
        <div class="separator"></div>
      {:else if 'header' in entry}
        <div class="header caption">{entry.header}</div>
      {:else}
        <button
          class="item"
          class:active={!nested && active === index}
          class:danger={entry.danger}
          disabled={entry.disabled}
          role="menuitem"
          onpointerenter={(event) => {
            if (nested) return;
            active = index;
            if (entry.submenu) openSubmenu(index, event.currentTarget as HTMLElement);
            else submenu = null;
          }}
          onclick={(event) => {
            if (entry.submenu) openSubmenu(index, event.currentTarget as HTMLElement);
            else choose(entry);
          }}
        >
          <span class="lead">
            {#if entry.checked}
              <Icon name="checkmark" />
            {:else if entry.dot}
              <span class="dot" style:background={entry.dot}></span>
            {:else if entry.icon}
              <Icon name={entry.icon} />
            {/if}
          </span>
          <span class="label">{entry.label}</span>
          {#if entry.hint}<span class="hint caption">{entry.hint}</span>{/if}
          {#if entry.submenu}<Icon name="chevronRight" size={12} />{/if}
        </button>
      {/if}
    {/each}
  {/snippet}
  <div
    class="menu"
    role="menu"
    tabindex="-1"
    bind:this={element}
    style:left="{position.left}px"
    style:top="{position.top}px"
  >
    {@render list(current.entries, false)}
  </div>
  {#if submenu && submenuEntries}
    <div class="menu" role="menu" style:left="{submenu.left}px" style:top="{submenu.top}px">
      {@render list(submenuEntries, true)}
    </div>
  {/if}
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 900;
  }
  .menu {
    position: fixed;
    z-index: 901;
    min-width: 200px;
    max-width: 380px;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    padding: 4px;
    background: var(--flyout);
    border: 1px solid var(--stroke-strong);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-flyout);
    outline: none;
    animation: appear 0.12s ease-out;
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 32px;
    padding: 0 10px 0 8px;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
  }
  .item:hover:not(:disabled),
  .item.active:not(:disabled) {
    background: var(--subtle-hover);
  }
  .item:disabled {
    color: var(--text-disabled);
  }
  .item.danger {
    color: var(--red);
  }
  .lead {
    display: inline-flex;
    width: 16px;
    justify-content: center;
    flex: none;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 4px;
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    color: var(--text-3);
  }
  .separator {
    height: 1px;
    margin: 4px -4px;
    background: var(--stroke-strong);
  }
  .header {
    padding: 6px 10px 2px 34px;
    color: var(--text-2);
  }
</style>
