// Values exchanged with the Rust side (camelCase, see src-tauri/src).

export type RecordingSource = 'microphone' | 'meeting' | 'importedFile';
export type TranscriptionStatus = 'notTranscribed' | 'queued' | 'running' | 'done' | 'failed';
export type SpeakerRole = 'me' | 'others' | 'diarized';
export type MatchState = 'unknown' | 'suggested' | 'automatic' | 'confirmed';
export type ExportFormat = 'text' | 'markdown' | 'srt' | 'vtt' | 'json';

export interface RecordingSummary {
  id: string;
  title: string;
  createdAt: string;
  duration: number;
  source: RecordingSource;
  status: TranscriptionStatus;
  errorMessage: string | null;
  language: string | null;
  folderId: string | null;
  hasTranscript: boolean;
  hasAudio: boolean;
}

export interface Folder {
  id: string;
  name: string;
  createdAt: string;
}

export interface Speaker {
  id: string;
  role: SpeakerRole;
  number: number;
  customName: string | null;
  displayName: string;
  defaultName: string;
}

export interface Segment {
  id: string;
  index: number;
  start: number;
  end: number;
  text: string;
  isEdited: boolean;
  speakerId: string | null;
}

export interface Transcript {
  id: string;
  language: string | null;
  engine: string;
  modifiedAt: string;
  speakers: Speaker[];
  segments: Segment[];
}

export interface VoiceStatus {
  personId: string | null;
  personName: string | null;
  state: MatchState;
  score: number | null;
  speechSeconds: number;
}

export interface RecordingDetail {
  recording: RecordingSummary;
  audioPath: string | null;
  transcript: Transcript | null;
  voices: Record<string, VoiceStatus>;
  speech: Record<string, number>;
}

export interface Person {
  id: string;
  name: string;
  samples: number;
  confirmedSamples: number;
  lastHeard: string | null;
}

export type ModelState =
  | { state: 'notInstalled' }
  | { state: 'installed' }
  | { state: 'downloading'; fraction: number }
  | { state: 'loading' }
  | { state: 'ready' }
  | { state: 'failed'; message: string };

export interface ServiceSnapshot {
  speech: ModelState;
  speakers: ModelState;
  queue: string[];
  running: string | null;
  progress: number;
}

export interface RecorderStatus {
  state: 'idle' | 'recording' | 'paused';
  isMeeting: boolean;
  duration: number;
  microphoneLevel: number;
  systemLevel: number;
  saving: RecordingSource[];
  sourceName: string | null;
  microphoneName: string | null;
}

export interface Settings {
  autoTranscribe: boolean;
  appearance: 'system' | 'light' | 'dark';
  recordingFormat: 'aac' | 'lossless';
  recordingQuality: 'standard' | 'high';
  microphone: string;
  levelsMicrophone: boolean;
  meetingSource: string;
  separatesSpeakers: boolean;
  separatesRoomSpeakers: boolean;
  recognizesVoices: boolean;
  usesGpu: boolean;
  gpuName: string;
  gpuFailed: boolean;
  checksForUpdates: boolean;
  lastUpdateCheck: string | null;
  latestReleaseVersion: string | null;
  latestReleasePage: string | null;
  skippedUpdateVersion: string | null;
  storageRoot: string | null;
  showsTrayIcon: boolean;
  welcomeDone: boolean;
}

export interface GraphicsCard {
  name: string;
  /** Dedicated video memory in bytes. */
  memory: number;
}

export interface UpdateInfo {
  version: string;
  page: string;
}

export interface Bootstrap {
  settings: Settings;
  version: string;
  mica: boolean;
  recordingsRoot: string;
  isDefaultStorage: boolean;
  update: UpdateInfo | null;
  speechDownloadSize: number;
  speakersDownloadSize: number;
}

export interface Microphone {
  id: string;
  name: string;
}

export interface Microphones {
  devices: Microphone[];
  defaultId: string | null;
}

export interface MeetingApp {
  exe: string;
  name: string;
}

export interface ImportStatus {
  total: number;
  completed: number;
  importedIds: string[];
  isFinished: boolean;
  isQueued: boolean;
}

export interface Notice {
  title: string;
  message: string | null;
  action: string | null;
}

export interface NameAssignment {
  speakerId: string;
  name: string;
  remember: boolean;
}

/// Sidebar selection: a filter or a folder.
export type LibraryFilter =
  | 'all'
  | 'microphone'
  | 'meetings'
  | 'imports'
  | 'notTranscribed'
  | 'inProgress'
  | 'done'
  | 'failed';

export type SidebarItem = { kind: 'filter'; filter: LibraryFilter } | { kind: 'folder'; id: string };
