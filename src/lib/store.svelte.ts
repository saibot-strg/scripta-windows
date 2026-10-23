// App-wide UI state, kept in step with the Rust side through events.

import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';

import { api } from './api';
import { t } from './i18n';
import type {
  ExportFormat,
  Folder,
  ImportStatus,
  LibraryFilter,
  Notice,
  Person,
  RecorderStatus,
  RecordingDetail,
  RecordingSummary,
  ServiceSnapshot,
  Settings,
  SidebarItem,
  UpdateInfo,
} from './types';

export const RATES = [0.5, 0.75, 1, 1.25, 1.5, 2];
export const SKIP_SECONDS = 15;

/** Plays the selected recording; one audio element for the whole app. */
class Player {
  readonly audio = new Audio();
  path = $state<string | null>(null);
  currentTime = $state(0);
  duration = $state(0);
  playing = $state(false);
  rate = $state(1);
  private frame = 0;

  constructor() {
    this.audio.preload = 'metadata';
    const stored = Number(localStorage.getItem('playback.rate'));
    if (RATES.includes(stored)) this.rate = stored;
    this.audio.addEventListener('loadedmetadata', () => {
      this.duration = Number.isFinite(this.audio.duration) ? this.audio.duration : 0;
      this.audio.playbackRate = this.rate;
    });
    this.audio.addEventListener('durationchange', () => {
      if (Number.isFinite(this.audio.duration)) this.duration = this.audio.duration;
    });
    this.audio.addEventListener('timeupdate', () => (this.currentTime = this.audio.currentTime));
    this.audio.addEventListener('play', () => {
      this.playing = true;
      this.follow();
    });
    this.audio.addEventListener('pause', () => (this.playing = false));
    this.audio.addEventListener('ended', () => (this.playing = false));
  }

  get isAvailable(): boolean {
    return this.path !== null;
  }

  /** Smooth position updates while playing (timeupdate fires only ~4×/s). */
  private follow() {
    cancelAnimationFrame(this.frame);
    const tick = () => {
      this.currentTime = this.audio.currentTime;
      if (!this.audio.paused) this.frame = requestAnimationFrame(tick);
    };
    this.frame = requestAnimationFrame(tick);
  }

  load(path: string | null, keepPosition = false) {
    if (path === this.path) return;
    const position = this.audio.currentTime;
    const wasPlaying = !this.audio.paused;
    this.audio.pause();
    this.path = path;
    this.duration = 0;
    if (!keepPosition) this.currentTime = 0;
    if (path) {
      // A cache breaker: the file may be replaced in place (WAV → AAC).
      this.audio.src = `${convertFileSrc(path)}?v=${Date.now()}`;
      this.audio.load();
      if (keepPosition) {
        this.audio.currentTime = position;
        if (wasPlaying) void this.audio.play();
      }
    } else {
      this.audio.removeAttribute('src');
      this.audio.load();
    }
  }

  play() {
    if (this.path) void this.audio.play().catch(() => {});
  }

  pause() {
    this.audio.pause();
  }

  toggle() {
    if (this.audio.paused) this.play();
    else this.pause();
  }

  seek(time: number) {
    this.audio.currentTime = Math.max(0, Math.min(time, this.duration || time));
    this.currentTime = this.audio.currentTime;
  }

  skip(seconds: number) {
    this.seek(this.audio.currentTime + seconds);
  }

  setRate(rate: number) {
    this.rate = rate;
    this.audio.playbackRate = rate;
    localStorage.setItem('playback.rate', String(rate));
  }
}

const idleRecorder: RecorderStatus = {
  state: 'idle',
  isMeeting: false,
  duration: 0,
  microphoneLevel: 0,
  systemLevel: 0,
  saving: [],
  sourceName: null,
  microphoneName: null,
};

const emptyService: ServiceSnapshot = {
  speech: { state: 'notInstalled' },
  speakers: { state: 'notInstalled' },
  queue: [],
  running: null,
  progress: 0,
};

export type SortField = 'date' | 'title' | 'duration';

function stored<T extends string>(key: string, fallback: T, allowed: readonly T[]): T {
  try {
    const value = localStorage.getItem(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

class AppStore {
  ready = $state(false);
  mica = $state(false);
  version = $state('');
  speechDownloadSize = $state(0);
  speakersDownloadSize = $state(0);
  settings = $state<Settings | null>(null);

  recordings = $state<RecordingSummary[]>([]);
  folders = $state<Folder[]>([]);
  sidebar = $state<SidebarItem>({ kind: 'filter', filter: 'all' });
  selectedId = $state<string | null>(null);
  detail = $state<RecordingDetail | null>(null);
  searchText = $state('');
  searchMatches = $state<Set<string> | null>(null);
  sortField = $state<SortField>(stored('library.sortField', 'date', ['date', 'title', 'duration'] as const));
  sortAscending = $state(localStorage.getItem('library.sortAscending') === 'true');
  transcriptStyle = $state<'bubbles' | 'text'>(stored('detail.transcriptStyle', 'bubbles', ['bubbles', 'text'] as const));

  service = $state<ServiceSnapshot>(emptyService);
  progress = $state<Record<string, number>>({});
  recorder = $state<RecorderStatus>(idleRecorder);
  microphoneLevels = $state<number[]>([]);
  systemLevels = $state<number[]>([]);
  people = $state<Person[]>([]);
  importStatus = $state<ImportStatus | null>(null);
  update = $state<UpdateInfo | null>(null);

  toast = $state<string | null>(null);
  notice = $state<Notice | null>(null);
  settingsOpen = $state(false);
  settingsPane = $state('general');
  assignmentOpen = $state(false);
  welcomeOpen = $state(false);
  confirmQuitOpen = $state(false);
  searchFocusRequest = $state(0);

  readonly player = new Player();
  private toastTimer = 0;
  private importTimer = 0;
  private reloadTimer = 0;

  // MARK: Derived

  get recorderActive(): boolean {
    return this.recorder.state !== 'idle';
  }

  get isSaving(): boolean {
    return this.recorder.saving.length > 0;
  }

  get selected(): RecordingSummary | null {
    return this.recordings.find((r) => r.id === this.selectedId) ?? null;
  }

  matches(item: SidebarItem, recording: RecordingSummary): boolean {
    if (item.kind === 'folder') return recording.folderId === item.id;
    return filterMatches(item.filter, recording);
  }

  get visible(): RecordingSummary[] {
    const matches = this.searchMatches;
    const list = this.recordings.filter((r) => this.matches(this.sidebar, r) && (!matches || matches.has(r.id)));
    const direction = this.sortAscending ? 1 : -1;
    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
    return list.sort((a, b) => {
      let order = 0;
      if (this.sortField === 'title') order = collator.compare(a.title, b.title);
      else if (this.sortField === 'duration') order = a.duration - b.duration;
      else order = a.createdAt.localeCompare(b.createdAt);
      if (order === 0) return b.createdAt.localeCompare(a.createdAt);
      return order * direction;
    });
  }

  count(item: SidebarItem): number {
    return this.recordings.filter((r) => this.matches(item, r)).length;
  }

  // MARK: Loading

  async start() {
    const bootstrap = await api.bootstrap(navigator.language);
    this.settings = bootstrap.settings;
    this.mica = bootstrap.mica;
    this.version = bootstrap.version;
    this.update = bootstrap.update;
    this.speechDownloadSize = bootstrap.speechDownloadSize;
    this.speakersDownloadSize = bootstrap.speakersDownloadSize;
    await this.listen();
    await Promise.all([this.reload(), this.loadPeople()]);
    this.applyService(await api.serviceSnapshot());
    this.recorder = await api.recorderStatus();
    this.welcomeOpen = !bootstrap.settings.welcomeDone;
    this.ready = true;
  }

  private async listen() {
    await listen<{ id: string | null }>('library-changed', (event) => this.scheduleReload(event.payload.id));
    await listen<ServiceSnapshot>('service-changed', (event) => this.applyService(event.payload));
    await listen<{ id: string; fraction: number }>('transcription-progress', (event) => {
      this.progress = { [event.payload.id]: event.payload.fraction };
    });
    await listen<RecorderStatus>('recorder-status', (event) => this.applyRecorder(event.payload));
    await listen<Notice>('notice', (event) => (this.notice = event.payload));
    await listen<string>('toast', (event) => this.showToast(event.payload));
    await listen<ImportStatus>('import-status', (event) => this.applyImport(event.payload));
    await listen<string>('recording-added', (event) => void this.reveal(event.payload));
    await listen('voices-changed', () => void this.loadPeople());
    await listen<Settings>('settings-changed', (event) => (this.settings = event.payload));
    await listen<UpdateInfo>('update-available', (event) => (this.update = event.payload));
    await listen('confirm-quit', () => (this.confirmQuitOpen = true));
  }

  private applyService(snapshot: ServiceSnapshot) {
    this.service = snapshot;
    const progress: Record<string, number> = {};
    if (snapshot.running) progress[snapshot.running] = snapshot.progress;
    this.progress = progress;
  }

  private applyRecorder(status: RecorderStatus) {
    const wasActive = this.recorder.state !== 'idle';
    this.recorder = status;
    if (status.state === 'recording') {
      this.microphoneLevels = [...this.microphoneLevels.slice(-119), status.microphoneLevel];
      if (status.isMeeting) this.systemLevels = [...this.systemLevels.slice(-119), status.systemLevel];
    } else if (status.state === 'idle' && wasActive) {
      this.microphoneLevels = [];
      this.systemLevels = [];
    }
  }

  private applyImport(status: ImportStatus) {
    this.importStatus = status;
    clearTimeout(this.importTimer);
    if (status.isFinished) {
      // Leave the summary visible for a moment.
      this.importTimer = window.setTimeout(() => (this.importStatus = null), 10_000);
    }
  }

  private scheduleReload(id: string | null) {
    clearTimeout(this.reloadTimer);
    const reloadDetail = id === null || id === this.selectedId;
    this.reloadTimer = window.setTimeout(() => void this.reload(reloadDetail), 60);
  }

  async reload(includeDetail = true) {
    const [recordings, folders] = await Promise.all([api.listRecordings(), api.listFolders()]);
    this.recordings = recordings;
    this.folders = folders;
    if (this.selectedId && !recordings.some((r) => r.id === this.selectedId)) {
      this.select(null);
    } else if (includeDetail) {
      await this.loadDetail();
    }
    if (this.searchText.trim()) await this.runSearch();
  }

  async loadDetail() {
    const id = this.selectedId;
    if (!id) {
      this.detail = null;
      return;
    }
    const detail = await api.getRecording(id);
    if (this.selectedId !== id) return;
    this.detail = detail;
    this.player.load(detail?.audioPath ?? null, true);
  }

  async loadPeople() {
    this.people = await api.listPeople();
  }

  // MARK: Selection and search

  select(id: string | null) {
    if (id === this.selectedId) return;
    this.selectedId = id;
    this.detail = null;
    this.player.load(null);
    void this.loadDetail();
  }

  /** Selects a recording and widens filter and search so it is visible. */
  async reveal(id: string) {
    await this.reload(false);
    const recording = this.recordings.find((r) => r.id === id);
    if (!recording) return;
    if (!this.matches(this.sidebar, recording)) this.sidebar = { kind: 'filter', filter: 'all' };
    if (this.searchMatches && !this.searchMatches.has(id)) this.setSearch('');
    this.select(id);
  }

  setSidebar(item: SidebarItem) {
    this.sidebar = item;
  }

  private searchTimer = 0;
  setSearch(text: string) {
    this.searchText = text;
    clearTimeout(this.searchTimer);
    if (!text.trim()) {
      this.searchMatches = null;
      return;
    }
    this.searchTimer = window.setTimeout(() => void this.runSearch(), 150);
  }

  private async runSearch() {
    const query = this.searchText;
    const ids = await api.search(query);
    if (query === this.searchText) this.searchMatches = new Set(ids);
  }

  setSort(field: SortField, ascending: boolean) {
    this.sortField = field;
    this.sortAscending = ascending;
    localStorage.setItem('library.sortField', field);
    localStorage.setItem('library.sortAscending', String(ascending));
  }

  setTranscriptStyle(style: 'bubbles' | 'text') {
    this.transcriptStyle = style;
    localStorage.setItem('detail.transcriptStyle', style);
  }

  // MARK: Messages

  showToast(message: string) {
    this.toast = message;
    clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => (this.toast = null), 1800);
  }

  showError(error: unknown, title = t('Das hat nicht geklappt')) {
    const message = typeof error === 'string' ? error : error instanceof Error ? error.message : JSON.stringify(error);
    this.notice = { title, message, action: null };
  }

  /** Runs an action and shows its error in an alert. */
  async perform(action: () => Promise<unknown>) {
    try {
      await action();
    } catch (error) {
      if (error && typeof error === 'object' && 'title' in error) this.notice = error as Notice;
      else this.showError(error);
    }
  }

  // MARK: Recording

  async toggleRecording() {
    if (this.recorderActive) await this.stopRecording();
    else await this.perform(() => api.startRecording());
  }

  async startMeeting(appExe: string | null = null) {
    if (this.recorderActive) return;
    await this.perform(() => api.startMeeting(appExe));
  }

  async stopRecording() {
    await this.perform(() => api.stopRecording());
  }

  // MARK: Import

  async chooseFilesToImport(folderId: string | null = null) {
    const extensions = await api.importExtensions();
    const picked = await openDialog({
      multiple: true,
      title: t('Audio- oder Videodateien importieren'),
      filters: [{ name: t('Audio und Video'), extensions }],
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    await api.importFiles(paths, folderId);
  }

  async transcribeImported() {
    const status = this.importStatus;
    if (!status) return;
    for (const id of status.importedIds) {
      const recording = this.recordings.find((r) => r.id === id);
      if (recording && recording.status !== 'done') await api.transcribe(id);
    }
    this.importStatus = { ...status, isQueued: true };
  }

  // MARK: Export

  async export(id: string, format: ExportFormat) {
    const extension = { text: 'txt', markdown: 'md', srt: 'srt', vtt: 'vtt', json: 'json' }[format];
    const name = await api.exportFileName(id);
    const path = await saveDialog({
      defaultPath: `${name}.${extension}`,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!path) return;
    await this.perform(async () => {
      await api.exportTranscript(id, format, path);
      this.showToast(t('Exportiert'));
    });
  }

  async copyTranscript(id: string) {
    await this.perform(async () => {
      await writeText(await api.transcriptText(id, false));
      this.showToast(t('Transkript kopiert'));
    });
  }

  async copyText(text: string) {
    await writeText(text);
    this.showToast(t('Kopiert'));
  }

  async revealInExplorer(id: string) {
    const path = await api.recordingFile(id);
    if (path) await revealItemInDir(path);
  }

  // MARK: Settings

  async updateSettings(patch: Partial<Settings>) {
    await this.perform(async () => {
      this.settings = await api.updateSettings(patch);
      if ('appearance' in patch) await applyAppearance(this.settings.appearance);
    });
  }

  openSettings(pane = 'general') {
    this.settingsPane = pane;
    this.settingsOpen = true;
  }
}

export function filterMatches(filter: LibraryFilter, recording: RecordingSummary): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'microphone':
      return recording.source === 'microphone';
    case 'meetings':
      return recording.source === 'meeting';
    case 'imports':
      return recording.source === 'importedFile';
    case 'notTranscribed':
      return recording.status === 'notTranscribed';
    case 'inProgress':
      return recording.status === 'queued' || recording.status === 'running';
    case 'done':
      return recording.status === 'done';
    case 'failed':
      return recording.status === 'failed';
  }
}

/** Light, dark or the system setting, for the page and the window frame. */
export async function applyAppearance(appearance: 'system' | 'light' | 'dark') {
  const root = document.documentElement;
  if (appearance === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', appearance);
  try {
    await getCurrentWindow().setTheme(appearance === 'system' ? null : appearance);
  } catch {
    // Not in Tauri (browser preview).
  }
}

export const app = new AppStore();
