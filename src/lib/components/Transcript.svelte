<script lang="ts">
  import { tick } from 'svelte';
  import { api } from '../api';
  import { clockTime, duration } from '../format';
  import { t } from '../i18n';
  import { menu, type MenuEntry } from '../menu.svelte';
  import { isGuess, sortedSpeakers, tint } from '../speakers';
  import { app } from '../store.svelte';
  import type { RecordingDetail, Segment, Speaker } from '../types';
  import Icon from './Icon.svelte';

  let {
    detail,
    style,
    onAssign,
  }: { detail: RecordingDetail; style: 'bubbles' | 'text'; onAssign: (speaker: Speaker | null) => void } = $props();

  const transcript = $derived(detail.transcript!);
  const speakers = $derived(new Map(transcript.speakers.map((s) => [s.id, s])));
  const segments = $derived(transcript.segments);
  /** When the recording started; clock times only for own recordings (an import's date is the import). */
  const startedAt = $derived(detail.recording.source === 'importedFile' ? null : new Date(detail.recording.createdAt));

  // MARK: Following playback

  const activeId = $derived.by(() => {
    const player = app.player;
    if (!player.isAvailable || (!player.playing && player.currentTime === 0)) return null;
    const time = player.currentTime + 0.05;
    let low = 0;
    let high = segments.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (segments[mid].start <= time) low = mid + 1;
      else high = mid;
    }
    return low > 0 ? segments[low - 1].id : null;
  });

  let container = $state<HTMLDivElement>();
  $effect(() => {
    const id = activeId;
    // Follow playback, but leave the scroll position alone when paused.
    if (!id || !app.player.playing || !container) return;
    container.querySelector(`[data-segment-id="${id}"]`)?.scrollIntoView({ block: 'center', behavior: 'smooth' });
  });

  function playFrom(segment: Segment) {
    app.player.seek(segment.start);
    if (!app.player.playing) app.player.play();
  }

  // MARK: Editing

  let editing = $state<string | null>(null);
  let draft = $state('');
  let editor = $state<HTMLTextAreaElement>();

  async function edit(segment: Segment) {
    editing = segment.id;
    draft = segment.text;
    await tick();
    if (editor) {
      editor.focus();
      editor.style.height = `${editor.scrollHeight}px`;
    }
  }

  async function commit(segment: Segment) {
    if (editing !== segment.id) return;
    editing = null;
    const text = draft.trim();
    if (text && text !== segment.text) await app.perform(() => api.editSegment(segment.id, text));
  }

  // MARK: Menus

  function speakerChoices(segment: Segment, current: Speaker | null): MenuEntry[] {
    const present = new Set(transcript.speakers.map((s) => s.displayName.toLowerCase()));
    const others = sortedSpeakers(transcript.speakers.filter((s) => s.id !== current?.id));
    const people = app.people.filter((p) => !present.has(p.name.toLowerCase()));
    const entries: MenuEntry[] = [{ header: t('Diese Sprechblase gehört zu') }];
    for (const speaker of others) {
      entries.push({
        label: speaker.displayName,
        dot: tint(speaker),
        action: () => void app.perform(() => api.moveSegment(segment.id, speaker.id)),
      });
    }
    for (const person of people) {
      entries.push({
        label: person.name,
        dot: 'var(--text-3)',
        action: () => void app.perform(() => api.moveSegmentToNewSpeaker(segment.id, person.name)),
      });
    }
    entries.push({
      label: t('Neuer Sprecher'),
      icon: 'personAdd',
      action: () => void app.perform(() => api.moveSegmentToNewSpeaker(segment.id)),
    });
    if (current) {
      entries.push({ separator: true });
      if (isGuess(current, detail)) {
        entries.push({
          label: t('„{name}“ bestätigen', { name: current.displayName }),
          icon: 'checkmarkCircle',
          action: () => void app.perform(() => api.confirmSuggestion(current.id)),
        });
      }
      entries.push({
        label: t('Alle von „{name}“ benennen …', { name: current.displayName }),
        icon: 'people',
        action: () => onAssign(current),
      });
    }
    return entries;
  }

  function segmentMenu(event: MouseEvent, segment: Segment) {
    const speaker = segment.speakerId ? speakers.get(segment.speakerId) : null;
    const entries: MenuEntry[] = [
      { label: t('Ab hier abspielen'), icon: 'playFilled', disabled: !app.player.isAvailable, action: () => playFrom(segment) },
      { label: t('Text bearbeiten'), icon: 'edit', action: () => void edit(segment) },
    ];
    if (segment.isEdited) {
      entries.push({
        label: t('Originaltext wiederherstellen'),
        icon: 'arrowCounterclockwise',
        action: () => void app.perform(() => api.restoreSegment(segment.id)),
      });
    }
    entries.push({ label: t('Abschnitt kopieren'), icon: 'copy', action: () => void app.copyText(segment.text) });
    if (speaker) {
      const others = sortedSpeakers(transcript.speakers.filter((s) => s.id !== speaker.id));
      entries.push(
        { separator: true },
        { label: t('„{name}“ umbenennen …', { name: speaker.displayName }), icon: 'personVoice', action: () => onAssign(speaker) },
      );
      if (others.length) {
        entries.push({
          label: t('„{name}“ zuordnen zu', { name: speaker.displayName }),
          icon: 'people',
          submenu: others.map((other) => ({
            label: other.displayName,
            dot: tint(other),
            action: () => void app.perform(() => api.mergeSpeaker(speaker.id, other.id)),
          })),
        });
      }
      // Two people in one voice: move single passages out.
      entries.push({
        label: t('Nur diesen Abschnitt'),
        icon: 'arrowTurnDownRight',
        submenu: [
          ...others.map((other) => ({
            label: other.displayName,
            dot: tint(other),
            action: () => void app.perform(() => api.moveSegment(segment.id, other.id)),
          })),
          ...(others.length ? [{ separator: true as const }] : []),
          {
            label: t('Neuer Sprecher'),
            icon: 'personAdd' as const,
            action: () => void app.perform(() => api.moveSegmentToNewSpeaker(segment.id)),
          },
        ],
      });
    }
    menu.at(event, entries);
  }

  function spokenAt(segment: Segment): string | null {
    return startedAt ? clockTime(new Date(startedAt.getTime() + segment.start * 1000)) : null;
  }
</script>

{#snippet speakerName(segment: Segment, speaker: Speaker)}
  <button
    class="speaker-name"
    style:color={speaker.role === 'others' ? 'var(--text-2)' : tint(speaker)}
    title={t('Wem gehört diese Sprechblase?')}
    onclick={(event) => menu.below(event.currentTarget, speakerChoices(segment, speaker))}
  >
    {speaker.displayName}
  </button>
  {#if isGuess(speaker, detail)}
    <button
      class="guess"
      title={t('Vermutet – klicken zum Bestätigen, auf den Namen klicken zum Ändern')}
      onclick={() => app.perform(() => api.confirmSuggestion(speaker.id))}>?</button
    >
  {/if}
{/snippet}

{#snippet editable(segment: Segment)}
  {#if editing === segment.id}
    <textarea
      class="editor"
      bind:this={editor}
      bind:value={draft}
      rows="1"
      aria-label={t('Abschnitt bearbeiten')}
      oninput={() => {
        if (editor) {
          editor.style.height = 'auto';
          editor.style.height = `${editor.scrollHeight}px`;
        }
      }}
      onkeydown={(event) => {
        if (event.key === 'Enter' && !event.shiftKey) {
          event.preventDefault();
          void commit(segment);
        } else if (event.key === 'Escape') {
          event.stopPropagation();
          editing = null;
        }
      }}
      onblur={() => void commit(segment)}
    ></textarea>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="text" ondblclick={() => void edit(segment)}>{segment.text}</span>
  {/if}
{/snippet}

<div class="transcript" bind:this={container}>
  {#each segments as segment, index (segment.id)}
    {@const speaker = segment.speakerId ? (speakers.get(segment.speakerId) ?? null) : null}
    {@const newSpeaker = index === 0 || segments[index - 1].speakerId !== segment.speakerId}
    {@const active = segment.id === activeId}
    {#if style === 'bubbles'}
      {@const mine = speaker?.role === 'me'}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="bubble-row"
        class:mine
        class:new={newSpeaker && index > 0}
        data-segment-id={segment.id}
        oncontextmenu={(event) => segmentMenu(event, segment)}
      >
        <div class="bubble-column">
          {#if newSpeaker || segment.isEdited}
            <div class="bubble-header caption">
              {#if speaker && newSpeaker}{@render speakerName(segment, speaker)}{/if}
              <button class="time tabular" title={t('Zu dieser Stelle springen')} onclick={() => playFrom(segment)}>
                {duration(segment.start)}
              </button>
              {#if spokenAt(segment)}<span class="clock tabular">· {spokenAt(segment)}</span>{/if}
              {#if segment.isEdited}<span class="edited" title={t('Bearbeitet')}><Icon name="edit" size={12} /></span>{/if}
            </div>
          {/if}
          <div class="bubble" class:active class:editing={editing === segment.id}>
            {@render editable(segment)}
          </div>
        </div>
        {#if speaker && editing !== segment.id}
          <button
            class="who subtle icon"
            title={t('Wem gehört diese Sprechblase?')}
            aria-label={t('Sprecher dieser Sprechblase')}
            onclick={(event) => menu.below(event.currentTarget, speakerChoices(segment, speaker))}
          >
            <Icon name="personCircle" />
          </button>
        {/if}
      </div>
    {:else}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="text-row"
        class:active
        class:new={newSpeaker && index > 0}
        data-segment-id={segment.id}
        oncontextmenu={(event) => segmentMenu(event, segment)}
      >
        <button class="stamp" title={t('Zu dieser Stelle springen')} onclick={() => playFrom(segment)}>
          <span class="tabular">{duration(segment.start)}</span>
          {#if spokenAt(segment)}<span class="clock caption tabular">{spokenAt(segment)}</span>{/if}
        </button>
        <div class="body">
          {#if newSpeaker && speaker}
            <div class="text-speaker">{@render speakerName(segment, speaker)}</div>
          {/if}
          {@render editable(segment)}
        </div>
        <div class="tools">
          {#if speaker && editing !== segment.id}
            <button
              class="subtle icon"
              title={t('Wem gehört diese Sprechblase?')}
              aria-label={t('Sprecher dieser Sprechblase')}
              onclick={(event) => menu.below(event.currentTarget, speakerChoices(segment, speaker))}
            >
              <Icon name="personCircle" />
            </button>
          {/if}
          {#if editing !== segment.id}
            <button
              class="subtle icon"
              class:keep={segment.isEdited}
              title={segment.isEdited ? t('Bearbeitet – klicken zum erneuten Bearbeiten') : t('Text bearbeiten')}
              aria-label={t('Text bearbeiten')}
              onclick={() => void edit(segment)}
            >
              <Icon name="edit" />
            </button>
          {/if}
        </div>
      </div>
    {/if}
  {/each}
</div>

<style>
  .transcript {
    display: flex;
    flex-direction: column;
  }
  .text {
    user-select: text;
    cursor: text;
    white-space: pre-wrap;
  }
  .editor {
    width: 100%;
    min-width: 240px;
    resize: none;
    overflow: hidden;
    border: none;
    outline: none;
    padding: 0;
    background: transparent;
    line-height: 22px;
    user-select: text;
  }
  .speaker-name {
    border: none;
    background: none;
    padding: 0;
    font-weight: 600;
    color: inherit;
  }
  .speaker-name:hover {
    text-decoration: underline;
  }
  .guess {
    border: none;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--subtle-selected);
    color: var(--text-2);
    font-weight: 600;
    font-size: 11px;
  }
  .time {
    border: none;
    background: none;
    padding: 0;
    color: inherit;
  }
  .time:hover {
    color: var(--accent-text);
  }
  .clock {
    color: var(--text-3);
  }
  .edited {
    display: inline-flex;
    color: var(--text-2);
  }

  /* Bubbles: "Ich" on the right in the accent color, others on the left. */
  .bubble-row {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding-right: 72px;
    margin-top: 3px;
  }
  .bubble-row.new {
    margin-top: 14px;
  }
  .bubble-row.mine {
    flex-direction: row-reverse;
    padding-right: 0;
    padding-left: 72px;
  }
  .bubble-column {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .mine .bubble-column {
    align-items: flex-end;
  }
  .bubble-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    color: var(--text-2);
  }
  .bubble {
    padding: 8px 14px;
    border-radius: 18px;
    background: var(--bubble);
    line-height: 22px;
    outline: 2px solid transparent;
    outline-offset: 3px;
    transition: outline-color 0.25s;
  }
  .mine .bubble {
    background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 88%, white), var(--accent));
    color: var(--on-accent);
  }
  .mine .bubble ::selection {
    background: var(--on-accent);
    color: var(--accent);
  }
  .bubble.active,
  .bubble.editing {
    outline-color: var(--accent);
  }
  .mine .editor {
    color: var(--on-accent);
  }
  .who {
    opacity: 0;
    margin-bottom: 4px;
    color: var(--text-2);
  }
  .bubble-row:hover .who {
    opacity: 1;
  }

  /* Plain text. */
  .text-row {
    display: flex;
    align-items: baseline;
    gap: 14px;
    padding: 6px 10px;
    border-radius: 10px;
    margin-top: 4px;
    transition: background 0.25s;
  }
  .text-row.new {
    margin-top: 14px;
  }
  .text-row.active {
    background: var(--accent-soft);
  }
  .stamp {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    min-width: 52px;
    border: none;
    background: none;
    padding: 0;
    color: var(--text-2);
  }
  .text-row.active .stamp {
    color: var(--accent-text);
  }
  .stamp:hover {
    color: var(--accent-text);
  }
  .body {
    flex: 1;
    min-width: 0;
    line-height: 23px;
  }
  .text-speaker {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-bottom: 2px;
  }
  .tools {
    display: flex;
    align-self: flex-start;
  }
  .tools .subtle {
    opacity: 0;
    color: var(--text-2);
  }
  .tools .subtle.keep {
    opacity: 0.5;
  }
  .text-row:hover .tools .subtle {
    opacity: 1;
  }
</style>
