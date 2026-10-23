<script lang="ts">
  import { duration, rate, spokenDuration } from '../format';
  import { t } from '../i18n';
  import { menu } from '../menu.svelte';
  import { app, RATES, SKIP_SECONDS } from '../store.svelte';
  import Icon from './Icon.svelte';

  const player = app.player;
  let scrubbing = $state<number | null>(null);
  const position = $derived(scrubbing ?? player.currentTime);
  const available = $derived(player.isAvailable);
</script>

<div class="bar" class:disabled={!available} title={available ? '' : t('Für diese Aufnahme gibt es keine Audiodatei.')}>
  <div class="transport">
    <button class="subtle icon" disabled={!available} title={t('15 Sekunden zurück')} aria-label={t('15 Sekunden zurück')} onclick={() => player.skip(-SKIP_SECONDS)}>
      <Icon name="skipBack" size={18} />
    </button>
    <button
      class="play"
      disabled={!available}
      title={t('Wiedergabe starten oder pausieren (Leertaste)')}
      aria-label={player.playing ? t('Pause') : t('Wiedergabe')}
      onclick={() => player.toggle()}
    >
      <Icon name={player.playing ? 'pauseFilled' : 'playFilled'} size={18} />
    </button>
    <button class="subtle icon" disabled={!available} title={t('15 Sekunden vor')} aria-label={t('15 Sekunden vor')} onclick={() => player.skip(SKIP_SECONDS)}>
      <Icon name="skipForward" size={18} />
    </button>
    <span class="time tabular secondary">{duration(position)}</span>
    <input
      class="slider"
      type="range"
      min="0"
      max={Math.max(player.duration, 0.1)}
      step="0.1"
      value={position}
      disabled={!available}
      aria-label={t('Position')}
      aria-valuetext={spokenDuration(position)}
      style:--fill="{(position / Math.max(player.duration, 0.1)) * 100}%"
      oninput={(event) => (scrubbing = Number(event.currentTarget.value))}
      onchange={(event) => {
        player.seek(Number(event.currentTarget.value));
        scrubbing = null;
      }}
    />
    <span class="time tabular secondary">{duration(player.duration)}</span>
  </div>
  <button
    class="rate tabular"
    disabled={!available}
    title={t('Wiedergabetempo')}
    onclick={(event) =>
      menu.below(
        event.currentTarget,
        RATES.map((value) => ({ label: rate(value), checked: player.rate === value, action: () => player.setRate(value) })),
      )}
  >
    {rate(player.rate)}
  </button>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .transport {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px 6px 8px;
    border-radius: 26px;
    background: var(--flyout);
    border: 1px solid var(--stroke-strong);
    box-shadow: var(--shadow-flyout);
  }
  .play {
    width: 36px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 18px;
    border: none;
    background: var(--accent);
    color: var(--on-accent);
  }
  .play:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .play:disabled {
    background: var(--subtle-selected);
    color: var(--text-disabled);
  }
  .time {
    min-width: 46px;
    text-align: center;
    font-size: 12px;
  }
  .slider {
    flex: 1;
    min-width: 60px;
    appearance: none;
    height: 20px;
    background: transparent;
  }
  .slider::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--accent) var(--fill), var(--text-3) var(--fill));
  }
  .slider::-webkit-slider-thumb {
    appearance: none;
    width: 20px;
    height: 20px;
    margin-top: -8px;
    border-radius: 10px;
    background: var(--card-solid);
    border: 1px solid var(--stroke-strong);
    box-shadow: inset 0 0 0 5px var(--card-solid), inset 0 0 0 10px var(--accent);
    transition: box-shadow 0.1s;
  }
  .slider:hover::-webkit-slider-thumb {
    box-shadow: inset 0 0 0 4px var(--card-solid), inset 0 0 0 10px var(--accent);
  }
  .slider:disabled::-webkit-slider-thumb {
    box-shadow: inset 0 0 0 5px var(--card-solid), inset 0 0 0 10px var(--text-3);
  }
  .rate {
    min-width: 52px;
    height: 44px;
    border-radius: 22px;
    border: 1px solid var(--stroke-strong);
    background: var(--flyout);
    box-shadow: var(--shadow-flyout);
  }
  .rate:hover:not(:disabled) {
    background: var(--card-solid);
  }
  .disabled {
    opacity: 0.7;
  }
</style>
