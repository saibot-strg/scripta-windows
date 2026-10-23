// Recording actions shared by the list, the detail view and shortcuts.

import { api } from './api';
import { dialogs } from './dialogs.svelte';
import { t } from './i18n';
import type { MenuEntry } from './menu.svelte';
import { app } from './store.svelte';
import type { ExportFormat, RecordingSummary } from './types';

export const exportFormats: { format: ExportFormat; label: string }[] = [
  { format: 'text', label: t('Text (.txt)') },
  { format: 'markdown', label: t('Markdown (.md)') },
  { format: 'srt', label: t('Untertitel SRT (.srt)') },
  { format: 'vtt', label: t('Untertitel WebVTT (.vtt)') },
  { format: 'json', label: t('JSON mit Zeitstempeln (.json)') },
];

export function exportEntries(recording: RecordingSummary): MenuEntry[] {
  return [
    ...exportFormats.map((item) => ({ label: item.label, action: () => void app.export(recording.id, item.format) })),
    { separator: true },
    { label: t('Transkript kopieren'), icon: 'copy', hint: t('Strg+Umschalt+C'), action: () => void app.copyTranscript(recording.id) },
  ] as MenuEntry[];
}

export async function play(recording: RecordingSummary) {
  await app.reveal(recording.id);
  // The player loads with the detail; wait a moment for it.
  for (let i = 0; i < 20 && app.player.path === null; i++) await new Promise((r) => setTimeout(r, 50));
  app.player.play();
}

export async function rename(recording: RecordingSummary) {
  const title = await dialogs.prompt({ title: t('Aufnahme umbenennen'), value: recording.title, confirm: t('Umbenennen') });
  if (title) await app.perform(() => api.renameRecording(recording.id, title));
}

export async function remove(recording: RecordingSummary) {
  const confirmed = await dialogs.confirm({
    title: t('Aufnahme löschen?'),
    message: t('„{title}“ wird mit Transkript und Audiodateien endgültig gelöscht.', { title: recording.title }),
    confirm: t('Löschen'),
    danger: true,
  });
  if (confirmed) await app.perform(() => api.deleteRecording(recording.id));
}

export function transcribe(recording: RecordingSummary) {
  void app.perform(() => api.transcribe(recording.id));
}

export function cancelTranscription(recording: RecordingSummary) {
  void app.perform(() => api.cancelTranscription(recording.id));
}

export function recordingMenu(recording: RecordingSummary): MenuEntry[] {
  const inProgress = recording.status === 'queued' || recording.status === 'running';
  const entries: MenuEntry[] = [
    { label: t('Abspielen'), icon: 'playFilled', disabled: !recording.hasAudio, action: () => void play(recording) },
    inProgress
      ? { label: t('Transkription abbrechen'), icon: 'dismissCircle', action: () => cancelTranscription(recording) }
      : {
          label: recording.status === 'done' ? t('Erneut transkribieren') : t('Transkribieren'),
          icon: 'chat',
          disabled: !recording.hasAudio,
          action: () => transcribe(recording),
        },
    { separator: true },
    { label: t('Umbenennen …'), icon: 'edit', hint: 'F2', action: () => void rename(recording) },
  ];
  if (recording.hasTranscript) {
    entries.push(
      { label: t('Transkript kopieren'), icon: 'copy', action: () => void app.copyTranscript(recording.id) },
      { label: t('Exportieren'), icon: 'arrowExportUp', submenu: exportEntries(recording).slice(0, exportFormats.length) },
    );
  }
  const folders: MenuEntry[] = app.folders.map((folder) => ({
    label: folder.name,
    disabled: recording.folderId === folder.id,
    action: () => void app.perform(() => api.moveToFolder([recording.id], folder.id)),
  }));
  if (recording.folderId) {
    folders.push(
      { separator: true },
      { label: t('Aus dem Ordner nehmen'), action: () => void app.perform(() => api.moveToFolder([recording.id], null)) },
    );
  }
  if (app.folders.length === 0) folders.push({ label: t('Noch keine Ordner – lege sie links mit + an.'), disabled: true });
  entries.push(
    { label: t('In Ordner'), icon: 'folderAdd', submenu: folders },
    {
      label: t('Im Explorer zeigen'),
      icon: 'folderOpen',
      disabled: !recording.hasAudio,
      action: () => void app.revealInExplorer(recording.id),
    },
    { separator: true },
    { label: t('Löschen …'), icon: 'delete', danger: true, hint: t('Entf'), action: () => void remove(recording) },
  );
  return entries;
}

/** The microphone choice with a checkmark on the current one. */
export async function microphoneEntries(title = t('Mikrofon')): Promise<MenuEntry[]> {
  const microphones = await api.listMicrophones().catch(() => ({ devices: [], defaultId: null }));
  const current = app.settings?.microphone ?? '';
  const defaultName = microphones.devices.find((d) => d.id === microphones.defaultId)?.name;
  const choose = (id: string) => () =>
    void app.perform(async () => {
      await api.selectMicrophone(id);
      app.settings = await api.getSettings();
    });
  const entries: MenuEntry[] = [
    { header: title },
    {
      label: defaultName ? t('Systemstandard ({name})', { name: defaultName }) : t('Systemstandard'),
      checked: current === '',
      action: choose(''),
    },
  ];
  if (current && !microphones.devices.some((d) => d.id === current)) {
    entries.push({ label: t('Nicht verbunden'), checked: true, disabled: true });
  }
  for (const device of microphones.devices) {
    entries.push({ label: device.name, checked: device.id === current, action: choose(device.id) });
  }
  return entries;
}

/** Menu items to start a meeting with a specific audio source. */
export async function meetingEntries(): Promise<MenuEntry[]> {
  const apps = await api.listMeetingApps().catch(() => []);
  const entries: MenuEntry[] = [
    { label: t('Gesamtes Systemaudio'), icon: 'speaker', action: () => void app.startMeeting('') },
    { header: t('Nur eine App') },
  ];
  if (apps.length === 0) entries.push({ label: t('Keine App spielt gerade Audio ab'), disabled: true });
  for (const meetingApp of apps) {
    entries.push({
      label: meetingApp.name,
      checked: app.settings?.meetingSource === meetingApp.exe,
      action: () => void app.startMeeting(meetingApp.exe),
    });
  }
  return entries;
}
