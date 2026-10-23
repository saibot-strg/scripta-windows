<script lang="ts">
  import { meetingEntries, microphoneEntries } from '../actions';
  import { t } from '../i18n';
  import { menu } from '../menu.svelte';
  import { app, type SortField } from '../store.svelte';
  import type { LibraryFilter } from '../types';
  import Icon from './Icon.svelte';
  import RecordingCapsule from './RecordingCapsule.svelte';
  import RecordingRow from './RecordingRow.svelte';

  let search = $state<HTMLInputElement>();

  $effect(() => {
    if (app.searchFocusRequest > 0) {
      search?.focus();
      search?.select();
    }
  });

  const filterTitles: Record<LibraryFilter, string> = {
    all: t('Alle Aufnahmen'),
    microphone: t('Mikrofon'),
    meetings: t('Meetings'),
    imports: t('Importiert'),
    notTranscribed: t('Nicht transkribiert'),
    inProgress: t('In Bearbeitung'),
    done: t('Fertig'),
    failed: t('Fehler'),
  };

  const title = $derived(
    app.sidebar.kind === 'filter'
      ? filterTitles[app.sidebar.filter]
      : (app.folders.find((f) => app.sidebar.kind === 'folder' && f.id === app.sidebar.id)?.name ?? ''),
  );
  const visible = $derived(app.visible);
  const searching = $derived(app.searchText.trim().length > 0);

  function sortMenu(target: Element) {
    const field = (value: SortField, label: string) => ({
      label,
      checked: app.sortField === value,
      action: () => app.setSort(value, app.sortAscending),
    });
    menu.below(target, [
      { header: t('Sortieren nach') },
      field('date', t('Datum')),
      field('title', t('Titel')),
      field('duration', t('Dauer')),
      { separator: true },
      { label: t('Absteigend'), checked: !app.sortAscending, action: () => app.setSort(app.sortField, false) },
      { label: t('Aufsteigend'), checked: app.sortAscending, action: () => app.setSort(app.sortField, true) },
    ]);
  }
</script>

<section class="list" aria-label={title}>
  <header>
    <div class="heading">
      <h1 class="subtitle">{title}</h1>
      <span class="caption secondary">
        {visible.length === 1 ? t('1 Aufnahme') : t('{count} Aufnahmen', { count: visible.length })}
      </span>
    </div>
    <div class="commands">
      {#if app.recorderActive}
        <button class="button stop" onclick={() => app.stopRecording()} title={t('Aufnahme beenden und speichern (Strg+Eingabe)')}>
          <Icon name="stopFilled" />{t('Aufnahme beenden')}
        </button>
      {:else}
        <div class="split">
          <button class="subtle" onclick={() => app.toggleRecording()} title={t('Neue Aufnahme (Strg+N)')}>
            <span class="record"><Icon name="record" /></span>{t('Aufnahme')}
          </button>
          <button
            class="subtle chevron"
            aria-label={t('Mikrofon wählen')}
            onclick={async (event) => {
              const target = event.currentTarget;
              menu.below(target, await microphoneEntries());
            }}><Icon name="chevronDown" size={12} /></button
          >
        </div>
        <div class="split">
          <button
            class="subtle"
            onclick={() => app.startMeeting()}
            title={t('Meeting aufnehmen: Mikrofon und Systemaudio getrennt (Strg+Umschalt+N)')}
          >
            <Icon name="people" />{t('Meeting')}
          </button>
          <button
            class="subtle chevron"
            aria-label={t('Quelle wählen')}
            onclick={async (event) => {
              const target = event.currentTarget;
              const entries = [...(await meetingEntries()), { separator: true as const }, ...(await microphoneEntries(t('Mikrofon („Ich“)')))];
              menu.below(target, entries);
            }}><Icon name="chevronDown" size={12} /></button
          >
        </div>
      {/if}
      <button
        class="subtle"
        onclick={() => app.chooseFilesToImport(app.sidebar.kind === 'folder' ? app.sidebar.id : null)}
        title={t('Audio- oder Videodateien importieren (Strg+I) – oder einfach ins Fenster ziehen')}
      >
        <Icon name="arrowDownload" />{t('Importieren')}
      </button>
      <span class="spacer"></span>
      <button class="subtle icon" title={t('Sortierung ändern')} aria-label={t('Sortieren')} onclick={(event) => sortMenu(event.currentTarget)}>
        <Icon name="arrowSort" />
      </button>
    </div>
    <div class="search">
      <Icon name="search" />
      <input
        bind:this={search}
        class="input"
        type="search"
        placeholder={t('Titel und Transkripte durchsuchen')}
        value={app.searchText}
        oninput={(event) => app.setSearch(event.currentTarget.value)}
        onkeydown={(event) => {
          if (event.key === 'Escape') {
            app.setSearch('');
            search?.blur();
          }
        }}
      />
    </div>
  </header>

  <div class="rows" role="listbox" aria-label={title}>
    {#each visible as recording (recording.id)}
      <RecordingRow {recording} selected={recording.id === app.selectedId} />
    {/each}
    {#if visible.length === 0}
      <div class="empty">
        {#if searching}
          <Icon name="search" size={32} />
          <div class="body-strong">{t('Keine Treffer für „{text}“', { text: app.searchText.trim() })}</div>
          <div class="secondary">{t('Prüfe die Schreibweise oder suche nach einem anderen Wort.')}</div>
        {:else if app.sidebar.kind === 'folder'}
          <Icon name="folder" size={32} />
          <div class="body-strong">{t('Ordner ist leer')}</div>
          <div class="secondary">{t('Zieh Aufnahmen aus der Liste oder Audiodateien aus dem Explorer auf diesen Ordner links.')}</div>
        {:else if app.sidebar.filter === 'all'}
          <Icon name="mic" size={32} />
          <div class="body-strong">{t('Noch keine Aufnahmen')}</div>
          <div class="secondary">{t('Starte eine Aufnahme mit Strg+N oder ziehe Audio- und Videodateien in dieses Fenster.')}</div>
          <button class="button accent" onclick={() => app.toggleRecording()}>{t('Neue Aufnahme')}</button>
        {:else}
          <Icon name="library" size={32} />
          <div class="body-strong">{title}</div>
          <div class="secondary">{t('Hier gibt es noch keine Aufnahmen.')}</div>
        {/if}
      </div>
    {/if}
  </div>

  {#if app.importStatus || app.recorderActive || app.isSaving}
    <footer>
      {#if app.importStatus}
        {@const status = app.importStatus}
        <div class="import">
          {#if status.isFinished}
            <Icon name={status.importedIds.length ? 'checkmarkCircle' : 'warning'} />
            <span class="grow">
              {status.importedIds.length === 1 ? t('1 Datei importiert') : t('{count} Dateien importiert', { count: status.importedIds.length })}
            </span>
            {#if status.isQueued}
              <span class="caption secondary">{t('In der Warteschlange')}</span>
            {:else if status.importedIds.length}
              <button class="button accent small" onclick={() => app.transcribeImported()}>{t('Transkribieren')}</button>
            {/if}
            <button class="subtle icon" aria-label={t('Schließen')} onclick={() => (app.importStatus = null)}>
              <Icon name="dismiss" size={14} />
            </button>
          {:else}
            <span class="spinner"></span>
            <span class="grow tabular">{t('Importiere {current} von {total} …', { current: status.completed + 1, total: status.total })}</span>
          {/if}
        </div>
      {/if}
      {#if app.recorderActive || app.isSaving}
        <RecordingCapsule />
      {/if}
    </footer>
  {/if}
</section>

<style>
  .list {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }
  header {
    padding: 16px 12px 8px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .heading {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding-left: 4px;
  }
  h1 {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .commands {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .split {
    display: inline-flex;
    border-radius: var(--radius);
  }
  .split > .subtle:first-child {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
    padding-right: 6px;
  }
  .chevron {
    min-width: 22px !important;
    padding: 0 4px !important;
    border-top-left-radius: 0 !important;
    border-bottom-left-radius: 0 !important;
  }
  .record {
    display: inline-flex;
    color: var(--red);
  }
  .stop {
    background: #c42b1c;
    color: white;
    border-color: transparent;
  }
  .stop:hover:not(:disabled) {
    background: #b0281a !important;
  }
  .search {
    position: relative;
    display: flex;
    align-items: center;
  }
  .search > :global(.icon) {
    position: absolute;
    left: 10px;
    color: var(--text-2);
    pointer-events: none;
  }
  .search input {
    width: 100%;
    padding-left: 34px;
  }
  .search input::-webkit-search-cancel-button {
    filter: grayscale(1);
  }
  .rows {
    flex: 1;
    overflow-y: auto;
    padding: 0 10px 12px 12px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
    color: var(--text);
  }
  .empty > :global(.icon) {
    color: var(--text-3);
    margin-bottom: 4px;
  }
  .empty .secondary {
    max-width: 280px;
  }
  .empty .button {
    margin-top: 8px;
  }
  footer {
    padding: 8px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .import {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 44px;
    padding: 4px 6px 4px 14px;
    border-radius: 22px;
    background: var(--flyout);
    border: 1px solid var(--stroke-strong);
    box-shadow: var(--shadow-flyout);
  }
  .grow {
    flex: 1;
  }
  .small {
    min-height: 28px;
    padding: 3px 10px;
  }
</style>
