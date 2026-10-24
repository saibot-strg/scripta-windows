<script lang="ts">
  import { open as openDialog } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { dialogs } from '../dialogs.svelte';
  import { bytes, percent } from '../format';
  import { t } from '../i18n';
  import type { IconName } from '../icons';
  import { menu } from '../menu.svelte';
  import { app } from '../store.svelte';
  import type { GraphicsCard, MeetingApp, Microphones, ModelState, Person, Settings } from '../types';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';

  let { onclose }: { onclose: () => void } = $props();

  const panes: { id: string; label: string; icon: IconName }[] = [
    { id: 'general', label: t('Allgemein'), icon: 'settings' },
    { id: 'recording', label: t('Aufnahme'), icon: 'mic' },
    { id: 'transcription', label: t('Transkription'), icon: 'chat' },
    { id: 'people', label: t('Personen'), icon: 'people' },
    { id: 'privacy', label: t('Datenschutz'), icon: 'lockClosed' },
    { id: 'about', label: t('Info'), icon: 'info' },
  ];

  const settings = $derived(app.settings as Settings);
  let microphones = $state<Microphones>({ devices: [], defaultId: null });
  let apps = $state<MeetingApp[]>([]);
  let storageRoot = $state('');
  let sizes = $state<{ speech: number | null; speakers: number | null; speechDownload: number }>({ speech: null, speakers: null, speechDownload: 0 });
  let hasVoiceData = $state(false);
  let cards = $state<GraphicsCard[]>([]);
  let checking = $state(false);
  let checkResult = $state<string | null>(null);

  async function refresh() {
    [microphones, apps, storageRoot, sizes, hasVoiceData, cards] = await Promise.all([
      api.listMicrophones().catch(() => ({ devices: [], defaultId: null })),
      api.listMeetingApps().catch(() => []),
      api.currentStorageRoot(),
      api.modelSizes(),
      api.hasVoiceData(),
      api.graphicsCards().catch(() => []),
    ]);
  }
  void refresh();

  // Sizes change when a model finishes loading or is deleted.
  $effect(() => {
    void app.service.speech.state;
    void app.service.speakers.state;
    void app.people.length;
    void refresh();
  });

  const update = (patch: Partial<Settings>) => app.updateSettings(patch);

  // Without a choice, the card with the most memory (the discrete one).
  const largestCard = $derived(cards.reduce<GraphicsCard | null>((best, c) => (!best || c.memory > best.memory ? c : best), null));
  const card = $derived(cards.find((c) => c.name === settings.gpuName) ?? largestCard);
  // Graphics memory is sold in binary units: 24 GiB reads "24 GB".
  const cardLabel = (c: GraphicsCard) => `${c.name} · ${Math.round(c.memory / 2 ** 30)} GB`;
  const onGpu = $derived(settings.usesGpu && !!card);

  function modelStatus(state: ModelState): { text: string; tone: string } {
    switch (state.state) {
      case 'notInstalled':
        return { text: t('Nicht geladen'), tone: 'secondary' };
      case 'installed':
      case 'ready':
        return { text: t('Installiert'), tone: 'good' };
      case 'downloading':
        return { text: t('Wird geladen … {percent}', { percent: percent(state.fraction) }), tone: 'secondary' };
      case 'loading':
        return { text: t('Wird vorbereitet …'), tone: 'secondary' };
      case 'failed':
        return { text: state.message, tone: 'bad' };
    }
  }

  const busy = $derived(
    ['downloading', 'loading'].includes(app.service.speech.state) || ['downloading', 'loading'].includes(app.service.speakers.state),
  );

  async function chooseStorage() {
    const folder = await openDialog({ directory: true, title: t('Ordner für neue Aufnahmen wählen') });
    if (typeof folder === 'string') {
      await app.perform(async () => {
        storageRoot = await api.setStorageRoot(folder);
        app.settings = await api.getSettings();
      });
    }
  }

  function personMenu(target: Element, person: Person) {
    menu.below(target, [
      {
        label: t('Umbenennen …'),
        icon: 'edit',
        action: async () => {
          const name = await dialogs.prompt({
            title: t('Person umbenennen'),
            message: t('Der Name ändert sich auch in allen Transkripten, in denen die Person erkannt oder benannt wurde.'),
            value: person.name,
            confirm: t('Umbenennen'),
          });
          if (name) await app.perform(() => api.renamePerson(person.id, name));
        },
      },
      {
        label: t('Löschen …'),
        icon: 'delete',
        danger: true,
        action: async () => {
          const confirmed = await dialogs.confirm({
            title: t('„{name}“ löschen?', { name: person.name }),
            message: t('Die Stimmproben werden gelöscht. Namen in Transkripten bleiben stehen.'),
            confirm: t('Löschen'),
            danger: true,
          });
          if (confirmed) await app.perform(() => api.deletePerson(person.id));
        },
      },
    ]);
  }

  async function deleteAllVoices() {
    const confirmed = await dialogs.confirm({
      title: t('Alle Stimmprofile löschen?'),
      message: t('Alle Personen, Stimmproben und Stimm-Fingerabdrücke werden gelöscht. Namen in Transkripten bleiben stehen.'),
      confirm: t('Alle löschen'),
      danger: true,
    });
    if (confirmed) await app.perform(() => api.deleteAllVoices());
  }

  async function checkNow() {
    checking = true;
    checkResult = null;
    try {
      const found = await api.checkUpdates();
      app.update = found;
      checkResult = found ? t('Version {version} ist verfügbar.', { version: found.version }) : t('Scripta ist auf dem neuesten Stand.');
    } catch (error) {
      checkResult = t('Keine Verbindung zu GitHub: {error}', { error: String(error) });
    } finally {
      checking = false;
    }
  }

  const credits = [
    ['Parakeet TDT 0.6B v3', 'NVIDIA', 'CC BY 4.0'],
    ['TitaNet small', 'NVIDIA', 'CC BY 4.0'],
    ['pyannote segmentation 3.0', 'pyannote', 'MIT'],
    ['Silero VAD', 'Silero Team', 'MIT'],
    ['sherpa-onnx', 'k2-fsa', 'Apache 2.0'],
    ['ONNX Runtime', 'Microsoft', 'MIT'],
    ['Tauri', 'Tauri Programme', 'MIT / Apache 2.0'],
    ['Svelte', 'Svelte contributors', 'MIT'],
    ['Fluent UI System Icons', 'Microsoft', 'MIT'],
  ];
</script>

{#snippet toggle(title: string, description: string | null, checked: boolean, onchange: (value: boolean) => void, disabled = false)}
  <label class="setting" class:disabled>
    <div class="labels">
      <div>{title}</div>
      {#if description}<div class="caption secondary">{description}</div>{/if}
    </div>
    <input type="checkbox" class="switch" {checked} {disabled} onchange={(event) => onchange(event.currentTarget.checked)} />
  </label>
{/snippet}

<Dialog width={820} {onclose}>
  <div class="layout">
    <nav class="tabs" aria-label={t('Einstellungen')}>
      <div class="subtitle heading">{t('Einstellungen')}</div>
      {#each panes as pane}
        <button class="tab" class:selected={app.settingsPane === pane.id} onclick={() => (app.settingsPane = pane.id)}>
          <Icon name={pane.icon} />{pane.label}
        </button>
      {/each}
    </nav>
    <div class="pane">
      {#if app.settingsPane === 'general'}
        <h3>{t('Allgemein')}</h3>
        <div class="group">
          <div class="setting">
            <div class="labels"><div>{t('Erscheinungsbild')}</div></div>
            <select class="select" value={settings.appearance} onchange={(event) => update({ appearance: event.currentTarget.value as Settings['appearance'] })}>
              <option value="system">{t('Wie System')}</option>
              <option value="light">{t('Hell')}</option>
              <option value="dark">{t('Dunkel')}</option>
            </select>
          </div>
        </div>
        <div class="group">
          {@render toggle(t('Automatisch transkribieren'), t('Neue Aufnahmen und Importe werden direkt in die Warteschlange gestellt.'), settings.autoTranscribe, (v) => update({ autoTranscribe: v }))}
          {@render toggle(t('Symbol im Infobereich'), t('Aufnahme starten und beenden über das Symbol unten rechts in der Taskleiste.'), settings.showsTrayIcon, (v) => update({ showsTrayIcon: v }))}
        </div>
        <div class="group">
          {@render toggle(t('Automatisch nach Updates suchen'), t('Scripta fragt einmal am Tag bei GitHub nach, ob es eine neue Version gibt. Daten über deine Aufnahmen oder deine Nutzung werden dabei nicht übertragen.'), settings.checksForUpdates, (v) => update({ checksForUpdates: v }))}
          <div class="setting">
            <div class="labels">
              <div>{t('Version {version}', { version: app.version })}</div>
              {#if checkResult}<div class="caption secondary">{checkResult}</div>{/if}
            </div>
            <button class="button" disabled={checking} onclick={checkNow}>{t('Jetzt suchen')}</button>
          </div>
        </div>
      {:else if app.settingsPane === 'recording'}
        <h3>{t('Aufnahme')}</h3>
        <div class="group">
          <div class="setting">
            <div class="labels"><div>{t('Mikrofon')}</div></div>
            <select
              class="select wide"
              value={settings.microphone}
              onchange={(event) => app.perform(async () => {
                await api.selectMicrophone(event.currentTarget.value);
                app.settings = await api.getSettings();
              })}
            >
              <option value="">
                {microphones.devices.find((d) => d.id === microphones.defaultId)?.name
                  ? t('Systemstandard ({name})', { name: microphones.devices.find((d) => d.id === microphones.defaultId)?.name ?? '' })
                  : t('Systemstandard')}
              </option>
              {#if settings.microphone && !microphones.devices.some((d) => d.id === settings.microphone)}
                <option value={settings.microphone}>{t('Nicht verbunden')}</option>
              {/if}
              {#each microphones.devices as device}<option value={device.id}>{device.name}</option>{/each}
            </select>
          </div>
          {@render toggle(t('Leises Mikrofon anheben'), t('Bringt deine Stimme nach der Aufnahme auf normale Lautstärke, etwa bei leisen Headsets.'), settings.levelsMicrophone, (v) => update({ levelsMicrophone: v }))}
          <p class="footer caption secondary">{t('Gilt für Aufnahmen und Meetings („Ich“).')}</p>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels"><div>{t('Format')}</div></div>
            <select class="select" value={settings.recordingFormat} onchange={(event) => update({ recordingFormat: event.currentTarget.value as Settings['recordingFormat'] })}>
              <option value="aac">{t('AAC (kompakt)')}</option>
              <option value="lossless">{t('Verlustfrei (WAV)')}</option>
            </select>
          </div>
          <div class="setting" class:disabled={settings.recordingFormat !== 'aac'}>
            <div class="labels"><div>{t('Qualität')}</div></div>
            <select class="select" disabled={settings.recordingFormat !== 'aac'} value={settings.recordingQuality} onchange={(event) => update({ recordingQuality: event.currentTarget.value as Settings['recordingQuality'] })}>
              <option value="standard">{t('Standard (96 kbit/s)')}</option>
              <option value="high">{t('Hoch (160 kbit/s)')}</option>
            </select>
          </div>
          <p class="footer caption secondary">
            {t('Während der Aufnahme wird verlustfrei und absturzsicher gespeichert. Danach wird die Datei im gewählten Format abgelegt. Für die Transkription spielt das Format keine Rolle.')}
          </p>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels"><div>{t('Meetings: Standard-Quelle')}</div></div>
            <select class="select wide" value={settings.meetingSource} onchange={(event) => update({ meetingSource: event.currentTarget.value })}>
              <option value="">{t('Gesamtes Systemaudio')}</option>
              {#if settings.meetingSource && !apps.some((a) => a.exe === settings.meetingSource)}
                <option value={settings.meetingSource}>{settings.meetingSource}</option>
              {/if}
              {#each apps as meetingApp}<option value={meetingApp.exe}>{meetingApp.name}</option>{/each}
            </select>
          </div>
          <p class="footer caption secondary">
            {t('Mikrofon („Ich“) und Systemaudio („Andere“) werden als getrennte Spuren aufgenommen. Mit Kopfhörern bleiben die Spuren am saubersten getrennt. Nur eine App (z. B. Teams) aufzunehmen, braucht Windows 10 (2004) oder neuer.')}
          </p>
        </div>
        <div class="group">
          <div class="setting column">
            <div class="labels">
              <div>{t('Speicherort für neue Aufnahmen')}</div>
              <div class="caption secondary path">{storageRoot}</div>
            </div>
            <div class="buttons">
              <button class="button" disabled={app.recorderActive} onclick={chooseStorage}>{t('Ändern …')}</button>
              {#if settings.storageRoot}
                <button
                  class="button"
                  disabled={app.recorderActive}
                  onclick={() => app.perform(async () => {
                    storageRoot = await api.setStorageRoot(null);
                    app.settings = await api.getSettings();
                  })}>{t('Standard verwenden')}</button
                >
              {/if}
              <button class="button" onclick={() => api.openStorageFolder()}>{t('Im Explorer zeigen')}</button>
            </div>
          </div>
          <p class="footer caption secondary">
            {t('Bestehende Aufnahmen bleiben, wo sie sind. Netzlaufwerke und externe Platten funktionieren, solange sie verfügbar sind.')}
          </p>
        </div>
      {:else if app.settingsPane === 'transcription'}
        {@const speech = modelStatus(app.service.speech)}
        {@const speakers = modelStatus(app.service.speakers)}
        <h3>{t('Transkription')}</h3>
        <div class="group">
          <div class="setting">
            <div class="labels"><div>{t('Sprache')}</div></div>
            <span class="secondary">{t('Automatisch')}</span>
          </div>
          <p class="footer caption secondary">
            {t('Parakeet erkennt Deutsch, Englisch und 23 weitere europäische Sprachen selbst. Eine feste Sprachwahl ist bei diesem Modell nicht möglich.')}
          </p>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels">
              <div>{t('Spracherkennung')}</div>
              <div class="caption secondary">Parakeet TDT 0.6B v3 ({onGpu ? t('volle Genauigkeit') : 'int8'})</div>
            </div>
            <span class="status {speech.tone}">{speech.text}</span>
          </div>
          <div class="setting">
            <div class="labels">
              <div>{t('Speicherbedarf')}</div>
              <div class="caption secondary">{sizes.speech ? bytes(sizes.speech) : t('Download ca. {size}', { size: bytes(sizes.speechDownload || app.speechDownloadSize) })}</div>
            </div>
            <div class="buttons">
              {#if sizes.speech}
                <button
                  class="button"
                  disabled={busy}
                  onclick={async () => {
                    const confirmed = await dialogs.confirm({
                      title: t('Modell löschen?'),
                      message: t('Für die nächste Transkription wird es erneut geladen (ca. {size}).', { size: bytes(sizes.speechDownload || app.speechDownloadSize) }),
                      confirm: t('Löschen'),
                      danger: true,
                    });
                    if (confirmed) await api.deleteModel('speech');
                  }}>{t('Löschen …')}</button
                >
              {:else}
                <button class="button" disabled={busy} onclick={() => api.prepareModel('speech')}>{t('Jetzt laden')}</button>
              {/if}
            </div>
          </div>
          <p class="footer caption secondary">
            {onGpu ? t('Läuft auf der Grafikkarte dieses PCs.') : t('Läuft auf dem Prozessor dieses PCs.')}
            {t('Das Modell wird einmalig von Hugging Face geladen; danach arbeitet Scripta komplett offline. Modell: NVIDIA Parakeet TDT 0.6B v3 (CC BY 4.0) in der ONNX-Fassung von sherpa-onnx.')}
          </p>
        </div>
        <div class="group">
          {@render toggle(
            t('Grafikkarte verwenden'),
            card ? cardLabel(card) : t('Keine geeignete Grafikkarte gefunden'),
            settings.usesGpu && !!card,
            (v) => update({ usesGpu: v, gpuFailed: false }),
            !card,
          )}
          {#if cards.length > 1 && largestCard}
            <div class="setting" class:disabled={!settings.usesGpu}>
              <div class="labels"><div>{t('Grafikkarte')}</div></div>
              <select
                class="select wide"
                disabled={!settings.usesGpu}
                value={cards.some((c) => c.name === settings.gpuName) ? settings.gpuName : ''}
                onchange={(event) => update({ gpuName: event.currentTarget.value })}
              >
                <option value="">{t('Automatisch')}</option>
                {#each cards as c}
                  <option value={c.name}>{cardLabel(c)}</option>
                {/each}
              </select>
            </div>
          {/if}
          <p class="footer caption" class:secondary={!settings.gpuFailed} class:warning={settings.gpuFailed}>
            {settings.gpuFailed
              ? t('Scripta wurde bei der Arbeit auf der Grafikkarte beendet und rechnet seitdem wieder auf dem Prozessor. Du kannst es erneut versuchen; oft hilft ein aktueller Grafiktreiber.')
              : t('Die Spracherkennung läuft dann auf der Grafikkarte, mit einem genaueren Modell (einmalig ca. 2,5 GB) und meist um ein Vielfaches schneller. Funktioniert mit Grafikkarten von AMD, NVIDIA und Intel. Die Sprechertrennung bleibt auf dem Prozessor.')}
          </p>
        </div>
        <div class="group">
          {@render toggle(t('Sprecher unterscheiden'), null, settings.separatesSpeakers, (v) => update({ separatesSpeakers: v }))}
          {@render toggle(t('Mehrere Personen am Mikrofon'), t('Nur einschalten, wenn bei Meetings weitere Personen mit dir am PC sitzen: Dann werden auch die Stimmen an deinem Mikrofon getrennt, und das Transkribieren dauert länger. Ist im Meeting niemand online zu hören, geschieht das automatisch.'), settings.separatesRoomSpeakers, (v) => update({ separatesRoomSpeakers: v }), !settings.separatesSpeakers)}
          <div class="setting" class:disabled={!settings.separatesSpeakers}>
            <div class="labels">
              <div>{t('Sprechertrennung')}</div>
              <div class="caption secondary">{sizes.speakers ? bytes(sizes.speakers) : t('Download ca. {size}', { size: bytes(app.speakersDownloadSize) })}</div>
            </div>
            <span class="status {speakers.tone}">{speakers.text}</span>
            {#if sizes.speakers}
              <button class="button" disabled={busy} onclick={() => api.deleteModel('speakers')}>{t('Löschen')}</button>
            {:else}
              <button class="button" disabled={busy || !settings.separatesSpeakers} onclick={() => api.prepareModel('speakers')}>{t('Jetzt laden')}</button>
            {/if}
          </div>
          <p class="footer caption secondary">
            {t('Trennt die Stimmen in Meetings und Aufnahmen („Sprecher 1“, „Sprecher 2“ …); deine eigene Stimme im Meeting bleibt „Ich“. Modelle: pyannote segmentation 3.0 (MIT) und NVIDIA TitaNet small (CC BY 4.0). Läuft ebenfalls lokal.')}
          </p>
        </div>
      {:else if app.settingsPane === 'people'}
        <h3>{t('Personen')}</h3>
        <div class="group">
          {@render toggle(t('Stimmen wiedererkennen'), t('Scripta merkt sich die Stimmen der Personen, die du in Transkripten benennst, und erkennt sie in späteren Aufnahmen wieder.'), settings.recognizesVoices, (v) => update({ recognizesVoices: v }))}
          <p class="footer caption secondary">
            {t('Stimmprofile sind biometrische Daten. Sie bleiben auf diesem PC und lassen sich hier jederzeit löschen. Frag die Personen, ob das für sie in Ordnung ist.')}
          </p>
        </div>
        <div class="group">
          {#if app.people.length === 0}
            <div class="setting"><span class="secondary">{t('Noch niemand. Benenne Sprecher in einem Transkript, dann merkt sich Scripta ihre Stimmen.')}</span></div>
          {/if}
          {#each app.people as person (person.id)}
            <div class="setting">
              <div class="labels">
                <div>{person.name}</div>
                <div class="caption secondary">
                  {t('Bestätigte Stimmproben: {count}', { count: person.confirmedSamples })} ·
                  {person.confirmedSamples >= 3 ? t('wird automatisch erkannt') : t('wird vorgeschlagen')}
                </div>
              </div>
              <button class="subtle icon" aria-label={t('Mehr')} onclick={(event) => personMenu(event.currentTarget, person)}>
                <Icon name="moreHorizontal" />
              </button>
            </div>
          {/each}
          <p class="footer caption secondary">
            {t('Ab 3 bestätigten Stimmproben setzt Scripta den Namen selbst, vorher schlägt es ihn vor.')}
          </p>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels">
              <div>{t('So wird die Erkennung besser')}</div>
              <div class="caption secondary">
                {t('Jede Person benennst du einmal von Hand. Danach schlägt Scripta den Namen vor („?“), nach etwa drei bestätigten Meetings erkennt es die Person sicher. Die Stimmtrennung selbst lernt nicht dazu: Sie hängt vom Modell und vom Ton ab (ähnliche Stimmen, ein gemeinsames Mikrofon, komprimierte Anrufe). Fehler korrigierst du von Hand: Anzahl der Stimmen korrigieren, zusammenführen, einzelne Sprechblasen verschieben.')}
              </div>
            </div>
          </div>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels"><div>{t('Alle Stimmprofile löschen')}</div></div>
            <button class="button danger" disabled={!app.people.length && !hasVoiceData} onclick={deleteAllVoices}>{t('Alle löschen …')}</button>
          </div>
        </div>
      {:else if app.settingsPane === 'privacy'}
        <h3>{t('Datenschutz')}</h3>
        <div class="group">
          <div class="setting">
            <div class="labels">
              <div>{t('Mikrofon')}</div>
              <div class="caption secondary">{t('Für Aufnahmen und Meetings („Ich“). Windows erlaubt Desktop-Apps den Zugriff unter Datenschutz › Mikrofon.')}</div>
            </div>
            <button class="button" onclick={() => api.openPrivacySettings()}>{t('Windows-Einstellungen öffnen')}</button>
          </div>
          <div class="setting">
            <div class="labels">
              <div>{t('Systemaudio')}</div>
              <div class="caption secondary">{t('Für Meetings („Andere“). Unter Windows ist dafür keine Freigabe nötig.')}</div>
            </div>
            <button class="button" onclick={() => api.openSoundSettings()}>{t('Soundeinstellungen')}</button>
          </div>
          <p class="footer caption secondary">{t('Scripta arbeitet komplett lokal. Audio, Transkripte und Stimmprofile verlassen deinen PC nicht.')}</p>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels">
              <div>{t('Vor dem Aufnehmen')}</div>
              <div class="caption secondary">
                {t('Frag alle Beteiligten, bevor du ein Gespräch aufnimmst: Aufnahmen ohne Einverständnis sind in Deutschland verboten. Im Unternehmen klären Datenschutzbeauftragte und Betriebsrat, ob und wie Scripta eingesetzt wird, besonders die Stimmerkennung.')}
              </div>
            </div>
          </div>
        </div>
        <div class="group">
          <div class="setting">
            <div class="labels">
              <div>{t('Protokolle')}</div>
              <div class="caption secondary">{t('Für die Fehlersuche. Sie enthalten keine Transkripte, Namen oder Stimmdaten.')}</div>
            </div>
            <button class="button" onclick={() => api.openLogsFolder()}>{t('Ordner öffnen')}</button>
          </div>
        </div>
      {:else}
        <div class="about">
          <img src="/app-icon.png" alt="" width="72" height="72" />
          <div>
            <div class="subtitle">Scripta</div>
            <div class="secondary">{t('Version {version}', { version: app.version })}</div>
            <div class="secondary">{t('Lokale Transkription für Meetings und Sprachnotizen.')}</div>
          </div>
        </div>
        <h3>{t('Danke an')}</h3>
        <div class="group">
          {#each credits as [name, author, license]}
            <div class="setting">
              <div class="labels"><div>{name}</div><div class="caption secondary">{author}</div></div>
              <span class="caption secondary">{license}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
  {#snippet footer()}
    <button class="button accent" onclick={onclose}>{t('Fertig')}</button>
  {/snippet}
</Dialog>

<style>
  .layout {
    display: flex;
    gap: 20px;
    margin: -24px;
    height: 560px;
  }
  .tabs {
    width: 200px;
    flex: none;
    padding: 20px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-right: 1px solid var(--stroke);
  }
  .heading {
    padding: 0 12px 12px;
  }
  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    padding: 0 12px;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
  }
  .tab:hover {
    background: var(--subtle-hover);
  }
  .tab.selected {
    background: var(--subtle-selected);
  }
  .tab.selected::before {
    content: '';
    position: absolute;
    left: 0;
    top: 10px;
    width: 3px;
    height: 16px;
    border-radius: 2px;
    background: var(--accent);
  }
  .pane {
    flex: 1;
    overflow-y: auto;
    padding: 20px 24px 20px 0;
  }
  h3 {
    margin: 0 0 10px;
    font-size: 14px;
    font-weight: 600;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 16px;
  }
  .setting {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 56px;
    padding: 10px 16px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: var(--radius);
  }
  .setting.column {
    flex-direction: column;
    align-items: stretch;
  }
  .setting.disabled {
    opacity: 0.55;
  }
  .labels {
    flex: 1;
    min-width: 0;
  }
  .path {
    overflow-wrap: anywhere;
    user-select: text;
  }
  .buttons {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .wide {
    max-width: 280px;
  }
  .footer {
    margin: 4px 4px 0;
  }
  .status.good {
    color: var(--green);
  }
  .status.bad {
    color: var(--red);
    max-width: 240px;
  }
  .footer.warning {
    color: var(--red);
  }
  .about {
    display: flex;
    gap: 16px;
    align-items: center;
    margin-bottom: 20px;
  }
</style>
