<script lang="ts">
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import { recordingMenu, remove, rename } from './lib/actions';
  import { api } from './lib/api';
  import DialogHost from './lib/components/DialogHost.svelte';
  import Detail from './lib/components/Detail.svelte';
  import Dialog from './lib/components/Dialog.svelte';
  import Icon from './lib/components/Icon.svelte';
  import MenuHost from './lib/components/MenuHost.svelte';
  import RecordingList from './lib/components/RecordingList.svelte';
  import Settings from './lib/components/Settings.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import Welcome from './lib/components/Welcome.svelte';
  import { dialogs } from './lib/dialogs.svelte';
  import { folderAt, recordingDrag } from './lib/drag.svelte';
  import { t } from './lib/i18n';
  import { menu } from './lib/menu.svelte';
  import { app, applyAppearance } from './lib/store.svelte';

  let dropping = $state(false);
  let failed = $state<string | null>(null);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        await app.start();
        if (app.mica) document.documentElement.classList.add('mica');
        await applyAppearance(app.settings?.appearance ?? 'system');
      } catch (error) {
        failed = String(error);
      }
      // Shown only now, so there is no white flash.
      await getCurrentWindow().show().catch(() => {});
      unlisten = await getCurrentWebview()
        .onDragDropEvent((event) => {
          const payload = event.payload;
          if (payload.type === 'enter' || payload.type === 'over') {
            dropping = true;
            const ratio = window.devicePixelRatio || 1;
            recordingDrag.fileOverFolder = folderAt(payload.position.x / ratio, payload.position.y / ratio);
          } else if (payload.type === 'drop') {
            const folder = recordingDrag.fileOverFolder;
            dropping = false;
            recordingDrag.fileOverFolder = null;
            if (payload.paths.length) void app.perform(() => api.importFiles(payload.paths, folder));
          } else {
            dropping = false;
            recordingDrag.fileOverFolder = null;
          }
        })
        .catch(() => undefined);
    })();
    return () => unlisten?.();
  });

  const typing = (target: EventTarget | null) =>
    target instanceof HTMLElement && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName));

  function keydown(event: KeyboardEvent) {
    if (!app.ready || menu.current || dialogs.current) return;
    const modal = app.settingsOpen || app.assignmentOpen || app.welcomeOpen || app.confirmQuitOpen || app.notice;
    const ctrl = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    if (ctrl && key === 'f') {
      event.preventDefault();
      app.searchFocusRequest += 1;
      return;
    }
    if (modal) return;
    if (ctrl && event.shiftKey && key === 'n') {
      event.preventDefault();
      void app.startMeeting();
    } else if (ctrl && key === 'n') {
      event.preventDefault();
      void app.toggleRecording();
    } else if (ctrl && key === 'enter') {
      event.preventDefault();
      if (app.recorderActive) void app.stopRecording();
    } else if (ctrl && key === 'i') {
      event.preventDefault();
      void app.chooseFilesToImport();
    } else if (ctrl && key === ',') {
      event.preventDefault();
      app.openSettings();
    } else if (ctrl && event.shiftKey && key === 'c') {
      if (app.selected?.hasTranscript) {
        event.preventDefault();
        void app.copyTranscript(app.selected.id);
      }
    } else if (typing(event.target)) {
      return;
    } else if (key === ' ') {
      event.preventDefault();
      app.player.toggle();
    } else if (key === 'arrowdown' || key === 'arrowup') {
      event.preventDefault();
      const list = app.visible;
      const index = list.findIndex((r) => r.id === app.selectedId);
      const next = list[Math.max(0, Math.min(list.length - 1, index + (key === 'arrowdown' ? 1 : -1)))];
      if (next) {
        app.select(next.id);
        document.querySelector(`[data-recording-id="${next.id}"]`)?.scrollIntoView({ block: 'nearest' });
      }
    } else if (key === 'delete' && app.selected) {
      event.preventDefault();
      void remove(app.selected);
    } else if (key === 'f2' && app.selected) {
      event.preventDefault();
      void rename(app.selected);
    } else if (key === 'enter' && app.selected) {
      app.player.play();
    } else if ((key === 'contextmenu' || (event.shiftKey && key === 'f10')) && app.selected) {
      event.preventDefault();
      const row = document.querySelector(`[data-recording-id="${app.selected.id}"]`);
      if (row) menu.below(row, recordingMenu(app.selected));
    }
  }
</script>

<svelte:window onkeydown={keydown} oncontextmenu={(event) => !typing(event.target) && event.preventDefault()} />

{#if failed}
  <div class="failed">
    <div class="subtitle">{t('Scripta konnte nicht starten')}</div>
    <div class="secondary">{failed}</div>
  </div>
{:else if app.ready}
  <div class="window">
    <aside><Sidebar /></aside>
    <main class="content">
      <div class="list-column"><RecordingList /></div>
      <div class="detail-column"><Detail /></div>
    </main>
  </div>

  {#if dropping}
    <div class="drop">
      <div class="drop-card">
        <Icon name="arrowDownload" size={40} />
        <div class="subtitle">{t('Zum Importieren loslassen')}</div>
        <div class="secondary">{t('Audio und Video – bei Videos wird nur die Tonspur übernommen.')}</div>
      </div>
    </div>
  {/if}

  {#if recordingDrag.active}
    <div class="ghost caption body-strong" style:left="{recordingDrag.x + 12}px" style:top="{recordingDrag.y + 12}px">
      <Icon name="folderAdd" size={14} />{recordingDrag.label}
    </div>
  {/if}

  {#if app.toast}
    <div class="toast" role="status"><Icon name="checkmarkCircleFilled" />{app.toast}</div>
  {/if}

  {#if app.settingsOpen}<Settings onclose={() => (app.settingsOpen = false)} />{/if}
  {#if app.welcomeOpen}<Welcome />{/if}

  {#if app.confirmQuitOpen}
    <Dialog title={app.recorderActive ? t('Aufnahme läuft') : t('Aufnahme wird gespeichert')} width={440} onclose={() => (app.confirmQuitOpen = false)}>
      <p class="secondary">
        {app.recorderActive
          ? t('Soll Scripta die Aufnahme beenden, speichern und sich dann schließen?')
          : t('Scripta schließt sich, sobald die Aufnahme gespeichert ist.')}
      </p>
      {#snippet footer()}
        <button class="button accent" onclick={() => { app.confirmQuitOpen = false; void api.quitAfterSaving(); }}>
          {app.recorderActive ? t('Beenden und speichern') : t('Schließen')}
        </button>
        <button class="button" onclick={() => (app.confirmQuitOpen = false)}>{t('Abbrechen')}</button>
      {/snippet}
    </Dialog>
  {/if}

  {#if app.notice}
    {@const notice = app.notice}
    <Dialog title={notice.title} width={460} onclose={() => (app.notice = null)}>
      {#if notice.message}<p class="secondary notice-message">{notice.message}</p>{/if}
      {#snippet footer()}
        {#if notice.action === 'microphone'}
          <button class="button accent" onclick={() => { app.notice = null; void api.openPrivacySettings(); }}>{t('Windows-Einstellungen öffnen')}</button>
          <button class="button" onclick={() => (app.notice = null)}>{t('Abbrechen')}</button>
        {:else}
          <button class="button accent" onclick={() => (app.notice = null)}>{t('OK')}</button>
        {/if}
      {/snippet}
    </Dialog>
  {/if}

  <DialogHost />
  <MenuHost />
{/if}

<style>
  .window {
    display: flex;
    height: 100%;
  }
  aside {
    width: 248px;
    flex: none;
  }
  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    background: var(--layer-solid);
    border-top-left-radius: var(--radius-card);
    border-left: 1px solid var(--stroke);
    border-top: 1px solid var(--stroke);
    margin-top: 2px;
    overflow: hidden;
  }
  :global(html.mica) .content {
    background: var(--layer);
  }
  .list-column {
    width: 360px;
    flex: none;
    border-right: 1px solid var(--stroke);
  }
  .detail-column {
    flex: 1;
    min-width: 0;
    background: var(--card-secondary);
  }
  .drop {
    position: fixed;
    inset: 12px;
    z-index: 700;
    border-radius: 16px;
    border: 3px dashed var(--accent);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
  }
  .drop-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 28px;
    border-radius: var(--radius-card);
    background: var(--flyout);
    box-shadow: var(--shadow-flyout);
    color: var(--text);
  }
  .drop-card > :global(.icon) {
    color: var(--accent);
  }
  .ghost {
    position: fixed;
    z-index: 950;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 260px;
    padding: 6px 10px;
    border-radius: var(--radius);
    background: var(--flyout);
    box-shadow: var(--shadow-flyout);
    pointer-events: none;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 96px;
    transform: translateX(-50%);
    z-index: 750;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-radius: 20px;
    background: var(--flyout);
    border: 1px solid var(--stroke-strong);
    box-shadow: var(--shadow-flyout);
    font-weight: 600;
    animation: toast 0.25s ease-out;
  }
  .toast > :global(.icon) {
    color: var(--green);
  }
  @keyframes toast {
    from {
      opacity: 0;
      transform: translate(-50%, 12px);
    }
  }
  .failed {
    padding: 48px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .notice-message {
    margin: 0;
    white-space: pre-line;
    user-select: text;
  }
</style>
