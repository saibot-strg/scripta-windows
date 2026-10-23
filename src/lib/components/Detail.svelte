<script lang="ts">
  import { cancelTranscription, exportEntries, transcribe } from '../actions';
  import { duration, fullDate, languageName, percent, spokenDuration } from '../format';
  import { t } from '../i18n';
  import { menu } from '../menu.svelte';
  import { isGuess } from '../speakers';
  import { app } from '../store.svelte';
  import type { RecordingSource, Speaker } from '../types';
  import Icon from './Icon.svelte';
  import PlayerBar from './PlayerBar.svelte';
  import SourceIcon from './SourceIcon.svelte';
  import SpeakerAssignment from './SpeakerAssignment.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import Transcript from './Transcript.svelte';

  const detail = $derived(app.detail);
  const recording = $derived(detail?.recording ?? null);
  const transcript = $derived(detail?.transcript ?? null);
  const hasSpeakers = $derived(
    !!transcript && new Set(transcript.segments.map((s) => s.speakerId).filter(Boolean)).size > 1,
  );
  const style = $derived(hasSpeakers ? app.transcriptStyle : 'text');
  const progress = $derived(recording ? (app.progress[recording.id] ?? 0) : 0);

  const sourceLabels: Record<RecordingSource, string> = {
    microphone: t('Mikrofon'),
    meeting: t('Meeting'),
    importedFile: t('Import'),
  };

  const unnamed = $derived(transcript?.speakers.filter((s) => s.role === 'diarized' && !s.customName) ?? []);
  const guessed = $derived(detail && transcript ? transcript.speakers.filter((s) => isGuess(s, detail)) : []);

  function openAssignment(_speaker: Speaker | null = null) {
    app.assignmentOpen = true;
  }

  const statusTitle = $derived.by(() => {
    switch (recording?.status) {
      case 'queued':
        return t('In der Warteschlange');
      case 'running':
        return t('Wird transkribiert …');
      case 'failed':
        return t('Transkription fehlgeschlagen');
      default:
        return t('Noch kein Transkript');
    }
  });

  const statusMessage = $derived.by(() => {
    if (!recording) return '';
    const speech = app.service.speech;
    switch (recording.status) {
      case 'queued':
        return t('Die Transkription startet, sobald die vorherigen Aufträge fertig sind.');
      case 'running':
        if (speech.state === 'downloading')
          return t('Das Spracherkennungsmodell wird einmalig geladen (ca. 700 MB) … {percent}', { percent: percent(speech.fraction) });
        if (speech.state === 'loading') return t('Das Spracherkennungsmodell wird vorbereitet …');
        return t('{percent} – du kannst die App währenddessen normal weiter benutzen.', { percent: percent(progress) });
      case 'failed':
        return recording.errorMessage ?? t('Unbekannter Fehler.');
      default:
        return recording.hasAudio
          ? t('Das Transkript wird lokal auf diesem PC erstellt. Audio und Text verlassen deinen PC nicht.')
          : t('Für diese Aufnahme gibt es keine Audiodatei.');
    }
  });
</script>

{#if !recording || !detail}
  <div class="placeholder">
    <Icon name="library" size={40} />
    <div class="subtitle">{t('Keine Aufnahme ausgewählt')}</div>
    <div class="secondary">{t('Wähle eine Aufnahme aus der Liste.')}</div>
  </div>
{:else}
  <section class="detail" aria-label={recording.title}>
    <div class="toolbar">
      {#if recording.status === 'queued' || recording.status === 'running'}
        <button class="subtle" onclick={() => cancelTranscription(recording)}><Icon name="dismissCircle" />{t('Abbrechen')}</button>
      {:else if recording.status === 'done'}
        <button
          class="subtle"
          disabled={!recording.hasAudio}
          title={t('Transkript neu erstellen (Korrekturen gehen verloren)')}
          onclick={() => transcribe(recording)}
        >
          <Icon name="arrowClockwise" />{t('Erneut transkribieren')}
        </button>
      {:else}
        <button class="subtle" disabled={!recording.hasAudio} title={t('Lokal mit Parakeet transkribieren')} onclick={() => transcribe(recording)}>
          <Icon name="chat" />{t('Transkribieren')}
        </button>
      {/if}
      {#if hasSpeakers}
        <div class="segmented" role="radiogroup" aria-label={t('Darstellung des Transkripts')}>
          <button
            class:on={style === 'bubbles'}
            role="radio"
            aria-checked={style === 'bubbles'}
            title={t('Sprechblasen')}
            onclick={() => app.setTranscriptStyle('bubbles')}><Icon name="chatMultiple" /></button
          >
          <button
            class:on={style === 'text'}
            role="radio"
            aria-checked={style === 'text'}
            title={t('Text')}
            onclick={() => app.setTranscriptStyle('text')}><Icon name="textAlignLeft" /></button
          >
        </div>
      {/if}
      <button
        class="subtle"
        disabled={!transcript || transcript.segments.length === 0}
        title={t('Transkript exportieren oder kopieren')}
        onclick={(event) => menu.below(event.currentTarget, exportEntries(recording))}
      >
        <Icon name="arrowExportUp" />{t('Exportieren')}
      </button>
    </div>

    <div class="scroll">
      <div class="content">
        <header>
          <div class="title-row">
            <SourceIcon source={recording.source} size={44} />
            <div class="titles">
              <h1 class="title">{recording.title}</h1>
              <div class="secondary">{fullDate(recording.createdAt)}</div>
            </div>
          </div>
          <div class="chips">
            <span class="chip caption" aria-label={t('Dauer {duration}', { duration: spokenDuration(recording.duration) })}>
              <Icon name="clock" size={14} /><span class="tabular">{duration(recording.duration)}</span>
            </span>
            <span class="chip caption"><Icon name={recording.source === 'meeting' ? 'people' : recording.source === 'microphone' ? 'mic' : 'arrowDownload'} size={14} />{sourceLabels[recording.source]}</span>
            {#if transcript?.language ?? recording.language}
              <span class="chip caption"><Icon name="globe" size={14} />{languageName(transcript?.language ?? recording.language ?? '')}</span>
            {/if}
            <StatusBadge status={recording.status} {progress} />
          </div>
        </header>

        {#if !transcript || recording.status !== 'done'}
          <div class="status card">
            <span class="status-icon" class:running={recording.status === 'running'} class:failed={recording.status === 'failed'}>
              <Icon
                name={recording.status === 'failed' ? 'warning' : recording.status === 'queued' ? 'clock' : recording.status === 'running' ? 'hourglass' : 'chat'}
                size={34}
              />
            </span>
            <div class="subtitle status-title">{statusTitle}</div>
            <div class="secondary status-message">{statusMessage}</div>
            {#if recording.status === 'running'}
              {@const speech = app.service.speech}
              <div class="progress status-progress" class:indeterminate={speech.state === 'loading'}>
                <div style:width="{(speech.state === 'downloading' ? speech.fraction : progress) * 100}%"></div>
              </div>
            {/if}
            {#if recording.status === 'queued' || recording.status === 'running'}
              <button class="button large" onclick={() => cancelTranscription(recording)}>{t('Abbrechen')}</button>
            {:else}
              <button class="button accent large" disabled={!recording.hasAudio} onclick={() => transcribe(recording)}>
                {recording.status === 'failed' ? t('Erneut versuchen') : t('Transkribieren')}
              </button>
            {/if}
          </div>
        {/if}

        {#if transcript && recording.status === 'done' && (unnamed.length || guessed.length)}
          <div class="banner card">
            <span class="accent-icon"><Icon name="personVoice" size={24} /></span>
            <div class="banner-text">
              <div class="body-strong">
                {unnamed.length === 1
                  ? t('1 Stimme ohne Namen')
                  : unnamed.length
                    ? t('{count} Stimmen ohne Namen', { count: unnamed.length })
                    : guessed.length === 1
                      ? t('1 Name vermutet')
                      : t('{count} Namen vermutet', { count: guessed.length })}
              </div>
              <div class="secondary">
                {guessed.length
                  ? t('Vermutete Namen sind mit „?“ markiert. Hör rein und bestätige oder korrigiere sie.')
                  : t('Hör kurz rein und gib jeder Stimme einen Namen.')}
              </div>
            </div>
            <button class="button accent" onclick={() => openAssignment()}>{t('Zuordnen …')}</button>
          </div>
        {/if}

        {#if transcript}
          <Transcript {detail} {style} onAssign={openAssignment} />
        {/if}
      </div>
    </div>

    <div class="player">
      <PlayerBar />
    </div>
  </section>

  {#if app.assignmentOpen && transcript}
    <SpeakerAssignment {detail} onclose={() => (app.assignmentOpen = false)} />
  {/if}
{/if}

<style>
  .placeholder {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text);
  }
  .placeholder > :global(.icon) {
    color: var(--text-3);
    margin-bottom: 8px;
  }
  .detail {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }
  .toolbar {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 4px;
    padding: 10px 16px 0;
  }
  .segmented {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius);
    background: var(--subtle-selected);
  }
  .segmented button {
    width: 34px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--text-2);
  }
  .segmented button.on {
    background: var(--card-solid);
    color: var(--text);
    box-shadow: var(--shadow-card);
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
  }
  .content {
    max-width: 780px;
    margin: 0 auto;
    padding: 12px 32px 96px;
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-bottom: 28px;
  }
  .title-row {
    display: flex;
    gap: 14px;
    align-items: center;
  }
  .titles {
    min-width: 0;
  }
  h1 {
    margin: 0;
    user-select: text;
    overflow-wrap: anywhere;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 9px;
    border-radius: 10px;
    background: var(--subtle-selected);
    color: var(--text-2);
    font-weight: 600;
  }
  .status {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 32px;
    text-align: center;
    margin-bottom: 28px;
  }
  .status-icon {
    color: var(--accent);
    display: inline-flex;
  }
  .status-icon.running {
    animation: breathe 1.4s ease-in-out infinite;
  }
  .status-icon.failed {
    color: var(--red);
  }
  @keyframes breathe {
    50% {
      opacity: 0.4;
    }
  }
  .status-message {
    max-width: 420px;
    white-space: pre-line;
  }
  .status-progress {
    width: 280px;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px;
    margin-bottom: 24px;
  }
  .banner-text {
    flex: 1;
  }
  .accent-icon {
    display: inline-flex;
    color: var(--accent);
  }
  .player {
    position: absolute;
    left: 24px;
    right: 24px;
    bottom: 18px;
  }
</style>
