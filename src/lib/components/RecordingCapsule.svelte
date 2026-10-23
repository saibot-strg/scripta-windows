<script lang="ts">
  import { microphoneEntries } from '../actions';
  import { api } from '../api';
  import { duration, spokenDuration } from '../format';
  import { t } from '../i18n';
  import { menu } from '../menu.svelte';
  import { app } from '../store.svelte';
  import Icon from './Icon.svelte';
  import LevelMeter from './LevelMeter.svelte';

  const recorder = $derived(app.recorder);
  const paused = $derived(recorder.state === 'paused');
</script>

<div class="capsule" role="status" aria-label={paused ? t('Aufnahme pausiert') : t('Aufnahme läuft')}>
  {#if app.isSaving && recorder.state === 'idle'}
    <div class="status">
      <span class="spinner"></span>
      <span class="body-strong">
        {recorder.saving.includes('meeting') ? t('Meeting wird gespeichert …') : t('Aufnahme wird gespeichert …')}
      </span>
    </div>
  {:else}
    <div class="status" title={spokenDuration(recorder.duration)}>
      <span class="dot" class:paused></span>
      <span class="time tabular">{duration(recorder.duration)}</span>
      {#if recorder.isMeeting}
        <div class="meters">
          <div class="meter">
            <span class="caption label">{t('Ich')}</span>
            <LevelMeter levels={app.microphoneLevels} color={paused ? 'var(--text-3)' : 'var(--red)'} height={12} bar={2} gap={1.5} width={72} />
          </div>
          <div class="meter">
            <span class="caption label">{t('Andere')}</span>
            <LevelMeter levels={app.systemLevels} color={paused ? 'var(--text-3)' : 'var(--green)'} height={12} bar={2} gap={1.5} width={72} />
          </div>
        </div>
      {:else}
        <LevelMeter levels={app.microphoneLevels} color={paused ? 'var(--text-3)' : 'var(--red)'} width={96} />
      {/if}
    </div>
    <div class="buttons">
      <button
        class="round"
        title={t('Mikrofon wechseln (aktiv: {name})', { name: recorder.microphoneName ?? '–' })}
        aria-label={t('Mikrofon wechseln')}
        onclick={async (event) => {
          const target = event.currentTarget;
          menu.below(target, await microphoneEntries());
        }}
      >
        <Icon name="mic" />
      </button>
      <button
        class="round"
        title={paused ? t('Aufnahme fortsetzen') : t('Aufnahme pausieren')}
        aria-label={paused ? t('Fortsetzen') : t('Pausieren')}
        onclick={() => api.togglePause()}
      >
        <Icon name={paused ? 'micFilled' : 'pauseFilled'} />
      </button>
      <button
        class="round stop"
        title={t('Aufnahme beenden und speichern (Strg+Eingabe)')}
        aria-label={t('Aufnahme beenden')}
        onclick={() => app.stopRecording()}
      >
        <Icon name="stopFilled" />
      </button>
    </div>
  {/if}
</div>
{#if recorder.sourceName && recorder.state !== 'idle'}
  <div class="source caption secondary">{t('Andere: nur {app}', { app: recorder.sourceName })}</div>
{/if}

<style>
  .capsule {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 8px 8px 14px;
    border-radius: 24px;
    background: var(--flyout);
    border: 1px solid var(--stroke-strong);
    box-shadow: var(--shadow-flyout);
    animation: rise 0.2s ease-out;
  }
  @keyframes rise {
    from {
      transform: translateY(12px);
      opacity: 0;
    }
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 32px;
    min-width: 0;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 5px;
    background: var(--red);
    animation: pulse 1.4s ease-in-out infinite;
    flex: none;
  }
  .dot.paused {
    background: var(--text-3);
    animation: none;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .time {
    font-family: var(--font-display);
    font-size: 17px;
    font-weight: 600;
    min-width: 52px;
  }
  .meters {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .label {
    width: 34px;
    text-align: right;
    color: var(--text-2);
    font-weight: 600;
  }
  .buttons {
    display: flex;
    gap: 6px;
    flex: none;
  }
  .round {
    width: 36px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 18px;
    border: 1px solid var(--stroke-control);
    background: var(--control);
  }
  .round:hover {
    background: var(--control-hover);
  }
  .round.stop {
    background: #c42b1c;
    color: white;
    border-color: transparent;
  }
  .round.stop:hover {
    background: #b0281a;
  }
  .source {
    text-align: center;
    margin-top: 4px;
  }
</style>
