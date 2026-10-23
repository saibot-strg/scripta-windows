<script lang="ts">
  import { api } from '../api';
  import { bytes } from '../format';
  import { t } from '../i18n';
  import type { IconName } from '../icons';
  import { app } from '../store.svelte';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';

  const size = $derived(bytes(app.speechDownloadSize + app.speakersDownloadSize));

  const points: { icon: IconName; title: string; text: string }[] = $derived([
    {
      icon: 'lockClosed',
      title: t('Alles bleibt auf diesem PC'),
      text: t('Aufnahmen, Transkripte und Stimmprofile verlassen deinen PC nicht. Es gibt keine Cloud und keine Telemetrie.'),
    },
    {
      icon: 'arrowDownload',
      title: t('Einmalig {size} laden', { size }),
      text: t('Die Spracherkennung und die Sprechertrennung werden jetzt im Hintergrund geladen. Danach arbeitet Scripta offline.'),
    },
    {
      icon: 'people',
      title: t('Mikrofon oder Meeting'),
      text: t('„Aufnahme“ für Notizen und Gespräche im Raum. „Meeting“ für Teams & Co.: deine Stimme („Ich“) und die anderen („Andere“) werden getrennt aufgenommen.'),
    },
    {
      icon: 'personVoice',
      title: t('Namen werden mit der Zeit besser'),
      text: t('Scripta trennt die Stimmen. Jede Person benennst du einmal; wenn du „Stimmen wiedererkennen“ einschaltest, schlägt Scripta die Namen danach vor und erkennt sie nach etwa drei Meetings sicher.'),
    },
    {
      icon: 'shieldCheckmark',
      title: t('Vorher fragen'),
      text: t('Frag alle Beteiligten, bevor du aufnimmst: Aufnahmen ohne Einverständnis sind in Deutschland verboten.'),
    },
  ]);

  async function start() {
    await app.updateSettings({ welcomeDone: true });
    await api.prepareModel('speech');
    if (app.settings?.separatesSpeakers) await api.prepareModel('speakers');
    app.welcomeOpen = false;
  }
</script>

<Dialog title={t('Willkommen bei Scripta')} width={560}>
  <p class="secondary intro">{t('Scripta schreibt mit, was in deinen Meetings und Sprachnotizen gesagt wird – lokal auf diesem PC.')}</p>
  <ul>
    {#each points as point}
      <li>
        <span class="icon-wrap"><Icon name={point.icon} size={20} /></span>
        <div>
          <div class="body-strong">{point.title}</div>
          <div class="secondary">{point.text}</div>
        </div>
      </li>
    {/each}
  </ul>
  {#snippet footer()}
    <button class="button accent" onclick={start}>{t('Los geht’s')}</button>
  {/snippet}
</Dialog>

<style>
  .intro {
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  li {
    display: flex;
    gap: 14px;
  }
  .icon-wrap {
    display: inline-flex;
    color: var(--accent);
    padding-top: 2px;
  }
</style>
