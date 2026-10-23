<script lang="ts">
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../api';
  import { dialogs } from '../dialogs.svelte';
  import { percent } from '../format';
  import { t } from '../i18n';
  import { menu, type MenuEntry } from '../menu.svelte';
  import { tint } from '../speakers';
  import { app } from '../store.svelte';
  import type { NameAssignment, RecordingDetail, Speaker } from '../types';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';

  let { detail, onclose }: { detail: RecordingDetail; onclose: () => void } = $props();

  const transcript = $derived(detail.transcript!);

  /** Speakers in order of first appearance, "Ich" last. */
  const speakers = $derived.by(() => {
    const seen = new Set<string>();
    const ordered: Speaker[] = [];
    for (const segment of transcript.segments) {
      const speaker = transcript.speakers.find((s) => s.id === segment.speakerId);
      if (speaker && !seen.has(speaker.id)) {
        seen.add(speaker.id);
        ordered.push(speaker);
      }
    }
    return [...ordered.filter((s) => s.role !== 'me'), ...ordered.filter((s) => s.role === 'me')];
  });

  // The names as they are when the dialog opens.
  let names = $state<Record<string, string>>({});
  let remembers = $state<Record<string, boolean>>({});
  untrack(() => {
    for (const speaker of detail.transcript?.speakers ?? []) {
      names[speaker.id] = speaker.displayName;
      remembers[speaker.id] = true;
    }
  });
  app.player.pause();

  const recognizes = $derived(app.settings?.recognizesVoices ?? false);
  const total = $derived(Object.values(detail.speech).reduce((a, b) => a + b, 0));

  /** The passages of a speaker, longest first. */
  function passages(speaker: Speaker) {
    return transcript.segments
      .filter((s) => s.speakerId === speaker.id)
      .sort((a, b) => b.end - b.start - (a.end - a.start));
  }

  // MARK: Listening

  const excerpt = new Audio();
  let playingId = $state<string | null>(null);
  let stopTimer = 0;
  const nextIndex: Record<string, number> = {};

  function stopExcerpt() {
    clearTimeout(stopTimer);
    excerpt.pause();
    playingId = null;
  }

  /** Each start plays another passage (3–8 s). */
  function toggleExcerpt(speaker: Speaker) {
    const wasPlaying = playingId === speaker.id;
    stopExcerpt();
    if (wasPlaying || !detail.audioPath) return;
    const ranges = passages(speaker)
      .slice(0, 3)
      .map((s) => ({ start: s.start, end: Math.min(s.end, s.start + 8) }));
    if (!ranges.length) return;
    const index = (nextIndex[speaker.id] ?? 0) % ranges.length;
    nextIndex[speaker.id] = index + 1;
    const range = ranges[index];
    const source = convertFileSrc(detail.audioPath);
    if (!excerpt.src.startsWith(source)) excerpt.src = source;
    excerpt.currentTime = range.start;
    void excerpt.play();
    playingId = speaker.id;
    stopTimer = window.setTimeout(stopExcerpt, Math.max(range.end - range.start, 1) * 1000);
  }

  onDestroy(stopExcerpt);

  // MARK: Names

  function choices(speaker: Speaker): MenuEntry[] {
    // Each name once: speakers with the same name are merged on "Fertig" anyway.
    const seen = new Set<string>();
    const others = speakers.filter((other) => {
      if (other.id === speaker.id) return false;
      const key = (names[other.id] ?? other.displayName).toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    const people = app.people.filter((p) => !seen.has(p.name.toLowerCase()));
    const entries: MenuEntry[] = [];
    if (people.length) {
      entries.push({ header: t('Aus früheren Meetings') });
      for (const person of people) entries.push({ label: person.name, action: () => (names[speaker.id] = person.name) });
    }
    if (others.length) {
      entries.push({ header: t('Zusammenführen mit') });
      for (const other of others) {
        const name = names[other.id] ?? other.displayName;
        entries.push({ label: name, dot: tint(other), action: () => (names[speaker.id] = name) });
      }
    }
    if (!entries.length) entries.push({ label: t('Noch keine bekannten Personen'), disabled: true });
    return entries;
  }

  function recount(event: MouseEvent) {
    menu.below(
      event.currentTarget as Element,
      Array.from({ length: 10 }, (_, i) => ({
        label: String(i + 1),
        action: async () => {
          const confirmed = await dialogs.confirm({
            title: t('Mit {count} Stimmen neu erkennen?', { count: i + 1 }),
            message: t(
              'Das Transkript wird neu erstellt; Textkorrekturen und Namen in diesem Transkript gehen verloren. Bekannte Stimmen schlägt Scripta wieder vor.',
            ),
            confirm: t('Neu erkennen'),
          });
          if (!confirmed) return;
          await app.perform(() => api.transcribe(detail.recording.id, i + 1));
          onclose();
        },
      })),
    );
  }

  /** Applies all names together, so swapping two names doesn't merge the speakers on the way. */
  async function apply() {
    const assignments: NameAssignment[] = [];
    for (const speaker of speakers) {
      const name = (names[speaker.id] ?? '').trim();
      const remember = remembers[speaker.id] ?? true;
      const status = detail.voices[speaker.id];
      const changed = name !== speaker.displayName;
      // Shown as recognized or guessed and left as is: the user confirmed it.
      const confirms = (status?.state === 'automatic' || status?.state === 'suggested') && name === status?.personName;
      const forgets = !remember && !!status?.personId;
      if (changed || confirms || forgets) assignments.push({ speakerId: speaker.id, name, remember });
    }
    if (assignments.length) await app.perform(() => api.assignNames(transcript.id, assignments));
    onclose();
  }
</script>

<Dialog title={t('Wer spricht hier?')} width={640} {onclose}>
  <p class="intro secondary">
    {t('Hör kurz rein und gib jeder Stimme einen Namen. Bekommen zwei Sprecher denselben Namen, werden sie zusammengeführt.')}
  </p>
  {#if !recognizes}
    <div class="optin card">
      <span class="accent-icon"><Icon name="shieldCheckmark" size={24} /></span>
      <div>
        <div class="body-strong">{t('Stimmen wiedererkennen')}</div>
        <p class="secondary">
          {t(
            'Scripta kann sich die Stimmen merken, die du benennst, und schlägt die Namen in späteren Aufnahmen vor. Stimmprofile sind biometrische Daten: Sie bleiben auf diesem PC und lassen sich in den Einstellungen jederzeit löschen. Frag die Personen, ob das für sie in Ordnung ist.',
          )}
        </p>
        <button class="button" onclick={() => app.updateSettings({ recognizesVoices: true })}>{t('Einschalten')}</button>
      </div>
    </div>
  {/if}
  <datalist id="known-people">
    {#each app.people as person}<option value={person.name}></option>{/each}
  </datalist>
  <div class="rows">
    {#each speakers as speaker (speaker.id)}
      {@const status = detail.voices[speaker.id]}
      {@const longest = passages(speaker)}
      <div class="row">
        <span class="dot" style:background={tint(speaker)}></span>
        <div class="main">
          <div class="line">
            <input class="input name" list="known-people" placeholder={speaker.defaultName} bind:value={names[speaker.id]} />
            <button class="subtle icon" title={t('Bekannte Person oder anderen Sprecher wählen')} onclick={(event) => menu.below(event.currentTarget, choices(speaker))}>
              <Icon name="personCircle" />
            </button>
            {#if status?.state === 'suggested' && status.personName && names[speaker.id] !== status.personName}
              <button class="button" title={t('Klingt nach {name} – klicken zum Übernehmen', { name: status.personName })} onclick={() => (names[speaker.id] = status.personName ?? '')}>
                {status.personName}?
              </button>
            {/if}
            {#if status?.state === 'suggested' && names[speaker.id] === status.personName}
              <span class="tag caption secondary"><Icon name="questionCircle" size={14} />{t('Vermutet')}</span>
            {/if}
            {#if status?.state === 'automatic'}
              <span class="tag caption secondary"><Icon name="checkmarkCircle" size={14} />{t('Erkannt')}</span>
            {/if}
          </div>
          {#if longest[0]}<div class="quote secondary">„{longest[0].text}“</div>{/if}
          <div class="line caption secondary">
            <span>{t('{share} der Redezeit', { share: percent(total ? (detail.speech[speaker.id] ?? 0) / total : 0) })}</span>
            {#if recognizes && status}
              <label class="remember" title={t('Aus: nur hier benennen, z. B. für Gäste')}>
                <input type="checkbox" class="checkbox" bind:checked={remembers[speaker.id]} />
                {t('Stimme merken')}
              </label>
            {/if}
          </div>
        </div>
        <button
          class="button listen"
          disabled={!longest.length || !detail.audioPath}
          title={t('Reinhören (jeder Klick spielt eine andere Stelle)')}
          aria-label={t('Reinhören')}
          onclick={() => toggleExcerpt(speaker)}
        >
          <Icon name={playingId === speaker.id ? 'stopFilled' : 'playFilled'} />
        </button>
      </div>
    {/each}
  </div>
  {#snippet footer()}
    <button
      class="subtle recount"
      disabled={!detail.audioPath}
      title={detail.recording.source === 'meeting'
        ? t('Wie viele Personen außer dir waren zu hören? Das Transkript wird mit genau so vielen Stimmen neu erstellt.')
        : t('Wie viele Personen waren zu hören? Das Transkript wird mit genau so vielen Stimmen neu erstellt.')}
      onclick={recount}
    >
      {detail.recording.source === 'meeting' ? t('Anzahl der anderen korrigieren') : t('Anzahl der Personen korrigieren')}
      <Icon name="chevronDown" size={12} />
    </button>
    <span class="spacer"></span>
    <button class="button accent" onclick={apply}>{t('Fertig')}</button>
    <button class="button" onclick={onclose}>{t('Abbrechen')}</button>
  {/snippet}
</Dialog>

<style>
  .intro {
    margin: 0;
  }
  .optin {
    display: flex;
    gap: 12px;
    padding: 14px;
  }
  .optin p {
    margin: 4px 0 8px;
  }
  .accent-icon {
    color: var(--accent);
    display: inline-flex;
  }
  .rows {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--stroke);
    margin: 0 -24px;
    max-height: 470px;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--stroke);
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 5px;
    margin-top: 11px;
    flex: none;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .name {
    width: 220px;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .quote {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .remember {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 12px;
  }
  .listen {
    width: 36px;
    padding: 0;
  }
  .spacer {
    flex: 1;
  }
  .recount {
    min-width: 0 !important;
  }
</style>
