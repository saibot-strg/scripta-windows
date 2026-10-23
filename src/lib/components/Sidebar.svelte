<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { api } from '../api';
  import { dialogs } from '../dialogs.svelte';
  import { recordingDrag } from '../drag.svelte';
  import { percent } from '../format';
  import { t } from '../i18n';
  import type { IconName } from '../icons';
  import { menu } from '../menu.svelte';
  import { app } from '../store.svelte';
  import type { Folder, LibraryFilter, SidebarItem } from '../types';
  import Icon from './Icon.svelte';

  const filters: { section: string; items: { filter: LibraryFilter; label: string; icon: IconName }[] }[] = [
    { section: t('Bibliothek'), items: [{ filter: 'all', label: t('Alle Aufnahmen'), icon: 'library' }] },
    {
      section: t('Quelle'),
      items: [
        { filter: 'microphone', label: t('Mikrofon'), icon: 'mic' },
        { filter: 'meetings', label: t('Meetings'), icon: 'people' },
        { filter: 'imports', label: t('Importiert'), icon: 'arrowDownload' },
      ],
    },
    {
      section: t('Status'),
      items: [
        { filter: 'notTranscribed', label: t('Nicht transkribiert'), icon: 'documentText' },
        { filter: 'inProgress', label: t('In Bearbeitung'), icon: 'hourglass' },
        { filter: 'done', label: t('Fertig'), icon: 'checkmarkCircle' },
        { filter: 'failed', label: t('Fehler'), icon: 'warning' },
      ],
    },
  ];

  function isSelected(item: SidebarItem): boolean {
    const current = app.sidebar;
    if (item.kind === 'filter') return current.kind === 'filter' && current.filter === item.filter;
    return current.kind === 'folder' && current.id === item.id;
  }

  async function createFolder() {
    const name = await dialogs.prompt({ title: t('Neuer Ordner'), placeholder: t('Name'), confirm: t('Anlegen') });
    if (!name) return;
    await app.perform(async () => {
      const folder = await api.createFolder(name);
      await app.reload(false);
      app.setSidebar({ kind: 'folder', id: folder.id });
    });
  }

  function folderMenu(event: MouseEvent, folder: Folder) {
    menu.at(event, [
      {
        label: t('Umbenennen …'),
        icon: 'edit',
        action: async () => {
          const name = await dialogs.prompt({ title: t('Ordner umbenennen'), value: folder.name, confirm: t('Umbenennen') });
          if (name) await app.perform(() => api.renameFolder(folder.id, name));
        },
      },
      {
        label: t('Audiodateien hierhin importieren …'),
        icon: 'arrowDownload',
        action: () => void app.chooseFilesToImport(folder.id),
      },
      { separator: true },
      {
        label: t('Ordner löschen …'),
        icon: 'delete',
        danger: true,
        action: async () => {
          const confirmed = await dialogs.confirm({
            title: t('Ordner „{name}“ löschen?', { name: folder.name }),
            message: t('Die Aufnahmen darin bleiben erhalten.'),
            confirm: t('Ordner löschen'),
            danger: true,
          });
          if (!confirmed) return;
          if (isSelected({ kind: 'folder', id: folder.id })) app.setSidebar({ kind: 'filter', filter: 'all' });
          await app.perform(() => api.deleteFolder(folder.id));
        },
      },
    ]);
  }

  const speech = $derived(app.service.speech);
  const speakers = $derived(app.service.speakers);
  const downloading = $derived(
    speech.state === 'downloading' ? speech : speakers.state === 'downloading' ? speakers : null,
  );
</script>

<nav class="sidebar" aria-label={t('Bibliothek')}>
  <div class="scroll">
    {#each filters as group}
      <div class="section caption">{group.section}</div>
      {#each group.items as item}
        {@const sidebarItem = { kind: 'filter', filter: item.filter } as SidebarItem}
        <button class="row" class:selected={isSelected(sidebarItem)} onclick={() => app.setSidebar(sidebarItem)}>
          <Icon name={item.icon} />
          <span class="name">{item.label}</span>
          <span class="count caption tabular">{app.count(sidebarItem)}</span>
        </button>
      {/each}
    {/each}

    <div class="section caption folders">
      <span>{t('Ordner')}</span>
      <button class="subtle icon small" title={t('Neuer Ordner')} aria-label={t('Neuer Ordner')} onclick={createFolder}>
        <Icon name="add" size={14} />
      </button>
    </div>
    {#each app.folders as folder (folder.id)}
      {@const sidebarItem = { kind: 'folder', id: folder.id } as SidebarItem}
      {@const target = recordingDrag.overFolder === folder.id || recordingDrag.fileOverFolder === folder.id}
      <button
        class="row"
        class:selected={isSelected(sidebarItem)}
        class:target
        data-folder-id={folder.id}
        onclick={() => app.setSidebar(sidebarItem)}
        oncontextmenu={(event) => folderMenu(event, folder)}
      >
        <Icon name={target ? 'folderOpen' : 'folder'} />
        <span class="name">{folder.name}</span>
        <span class="count caption tabular">{app.count(sidebarItem)}</span>
      </button>
    {/each}
    {#if app.folders.length === 0}
      <p class="hint caption">{t('Ordner anlegen mit +, dann Aufnahmen hineinziehen.')}</p>
    {/if}
  </div>

  <div class="footer">
    {#if app.update}
      <div class="note">
        <div class="caption body-strong">{t('Version {version} ist verfügbar', { version: app.update.version })}</div>
        <div class="actions">
          <button class="link caption" onclick={() => app.update && openUrl(app.update.page)}>{t('Herunterladen')}</button>
          <button
            class="link caption"
            onclick={async () => {
              if (app.update) await api.skipUpdate(app.update.version);
              app.update = null;
            }}>{t('Überspringen')}</button
          >
        </div>
      </div>
    {/if}
    {#if downloading}
      <div class="note">
        <div class="caption body-strong row-inline"><Icon name="arrowDownload" size={14} />{t('Spracherkennung wird geladen')}</div>
        <div class="progress"><div style:width="{downloading.fraction * 100}%"></div></div>
        <div class="caption secondary">
          {percent(downloading.fraction)} · {t('Einmalig, danach komplett offline.')}
        </div>
      </div>
    {:else if speech.state === 'loading' || speakers.state === 'loading'}
      <div class="note row-inline caption body-strong"><span class="spinner"></span>{t('Modell wird vorbereitet …')}</div>
    {:else if speech.state === 'failed'}
      <div class="note">
        <div class="caption body-strong row-inline warn"><Icon name="warning" size={14} />{t('Modell nicht verfügbar')}</div>
        <div class="caption secondary clamp">{speech.message}</div>
        <button class="button" onclick={() => api.prepareModel('speech')}>{t('Erneut versuchen')}</button>
      </div>
    {/if}
    <button class="row" onclick={() => app.openSettings()}>
      <Icon name="settings" />
      <span class="name">{t('Einstellungen')}</span>
    </button>
  </div>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 4px 4px 4px 4px;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 0 8px;
  }
  .section {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 12px 4px;
    color: var(--text-2);
    font-weight: 600;
  }
  .section.folders {
    padding-right: 4px;
  }
  .small {
    min-height: 24px;
    min-width: 24px;
    width: 24px;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    height: 36px;
    padding: 0 12px 0 14px;
    margin: 2px 0;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
  }
  .row:hover {
    background: var(--subtle-hover);
  }
  .row:active {
    background: var(--subtle-pressed);
  }
  .row.selected {
    background: var(--subtle-selected);
  }
  .row.selected::before {
    content: '';
    position: absolute;
    left: 0;
    top: 10px;
    width: 3px;
    height: 16px;
    border-radius: 2px;
    background: var(--accent);
  }
  .row.target {
    background: var(--accent-soft);
    outline: 1px solid var(--accent);
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--text-2);
  }
  .hint {
    padding: 2px 14px;
    color: var(--text-3);
    margin: 0;
  }
  .footer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 4px;
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0 4px;
    padding: 10px 12px;
    border-radius: var(--radius-card);
    background: var(--card);
    border: 1px solid var(--stroke);
  }
  .row-inline {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .warn {
    color: var(--orange);
  }
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .actions {
    display: flex;
    gap: 12px;
  }
  .link {
    color: var(--accent-text);
    background: none;
    border: none;
    padding: 0;
    text-decoration: none;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
