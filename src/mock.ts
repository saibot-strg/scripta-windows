// Browser preview without the Rust side (`npm run dev`, then open
// http://localhost:1420). Sample data only; nothing here ships.
// URL options: ?select=<index>&style=text&theme=dark&recording=meeting&settings=<pane>&assign=1&welcome=1

import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import type { RecordingDetail, RecordingSummary, Settings } from './lib/types';

const params = new URLSearchParams(location.search);
const german = !navigator.language.toLowerCase().startsWith('en');
const hour = 3_600_000;
const now = Date.now();
const iso = (offset: number) => new Date(now - offset).toISOString();

const recordings: RecordingSummary[] = [
  {
    id: 'r1',
    title: german ? 'Meeting Microsoft Teams 23.10.2026, 10:02' : 'Meeting Microsoft Teams 10/23/2026, 10:02 AM',
    createdAt: iso(2 * hour),
    duration: 2_741,
    source: 'meeting',
    status: 'done',
    errorMessage: null,
    language: 'de',
    folderId: 'f1',
    hasTranscript: true,
    hasAudio: true,
  },
  {
    id: 'r2',
    title: german ? 'Aufnahme 22.10.2026, 16:40' : 'Recording 10/22/2026, 4:40 PM',
    createdAt: iso(20 * hour),
    duration: 312,
    source: 'microphone',
    status: 'notTranscribed',
    errorMessage: null,
    language: null,
    folderId: null,
    hasTranscript: false,
    hasAudio: true,
  },
  {
    id: 'r3',
    title: german ? 'Kundengespräch Planung' : 'Customer call planning',
    createdAt: iso(30 * hour),
    duration: 1_804,
    source: 'importedFile',
    status: 'running',
    errorMessage: null,
    language: null,
    folderId: 'f2',
    hasTranscript: false,
    hasAudio: true,
  },
  {
    id: 'r4',
    title: 'Team-Weekly',
    createdAt: iso(50 * hour),
    duration: 3_390,
    source: 'meeting',
    status: 'queued',
    errorMessage: null,
    language: null,
    folderId: null,
    hasTranscript: false,
    hasAudio: true,
  },
  {
    id: 'r5',
    title: german ? 'Interview Rohfassung' : 'Interview draft',
    createdAt: iso(140 * hour),
    duration: 945,
    source: 'importedFile',
    status: 'failed',
    errorMessage: german ? 'Die Audiodatei konnte nicht gelesen werden.' : 'The audio file could not be read.',
    language: null,
    folderId: null,
    hasTranscript: false,
    hasAudio: true,
  },
];

const lines: [string | null, number, string][] = [
  ['s1', 4, 'Guten Morgen zusammen. Heute geht es um die Planung für das nächste Quartal.'],
  ['s1', 11, 'Vorher eine kurze Frage zum Budget für die neuen Server.'],
  ['me', 16, 'Das hat die Finanzabteilung letzte Woche freigegeben. Wir können bestellen, sobald das Angebot da ist.'],
  ['s2', 25, 'Sehr gut. Dann kann das Team im März mit der Umstellung beginnen.'],
  ['s2', 31, 'Die Tests schließen wir bis Ende April ab, wenn nichts dazwischenkommt.'],
  ['me', 38, 'Passt. Denkt bitte auch an die Schulungen für die Buchhaltung.'],
  ['s3', 45, 'Mache ich. Ich schicke bis Freitag einen kurzen Zeitplan.'],
  ['s1', 52, 'Danke euch, dann sind wir für heute durch.'],
];

const detail: RecordingDetail = {
  recording: recordings[0],
  audioPath: 'C:\\Users\\demo\\AppData\\Local\\Scripta\\Recordings\\r1\\mix.m4a',
  transcript: {
    id: 't1',
    language: 'de',
    engine: 'parakeet-tdt-0.6b-v3',
    modifiedAt: iso(hour),
    speakers: [
      { id: 's1', role: 'diarized', number: 1, customName: 'Jonas', displayName: 'Jonas', defaultName: german ? 'Sprecher 1' : 'Speaker 1' },
      { id: 's2', role: 'diarized', number: 2, customName: null, displayName: german ? 'Sprecher 2' : 'Speaker 2', defaultName: german ? 'Sprecher 2' : 'Speaker 2' },
      { id: 's3', role: 'diarized', number: 3, customName: 'Mia', displayName: 'Mia', defaultName: german ? 'Sprecher 3' : 'Speaker 3' },
      { id: 'me', role: 'me', number: 0, customName: null, displayName: german ? 'Ich' : 'Me', defaultName: german ? 'Ich' : 'Me' },
    ],
    segments: lines.map(([speaker, start, text], index) => ({
      id: `g${index}`,
      index,
      start,
      end: start + 5,
      text,
      isEdited: index === 4,
      speakerId: speaker,
    })),
  },
  voices: {
    s1: { personId: 'p1', personName: 'Jonas', state: 'suggested', score: 0.62, speechSeconds: 21 },
    s3: { personId: 'p2', personName: 'Mia', state: 'automatic', score: 0.84, speechSeconds: 12 },
    s2: { personId: null, personName: null, state: 'unknown', score: null, speechSeconds: 11 },
  },
  speech: { s1: 21, s2: 11, s3: 6, me: 12 },
};

const settings: Settings = {
  autoTranscribe: false,
  appearance: (params.get('theme') as Settings['appearance']) ?? 'system',
  recordingFormat: 'aac',
  recordingQuality: 'standard',
  microphone: '',
  levelsMicrophone: true,
  meetingSource: '',
  separatesSpeakers: true,
  separatesRoomSpeakers: false,
  recognizesVoices: params.get('recognize') === '1',
  usesGpu: params.get('gpu') === '1',
  gpuName: '',
  gpuFailed: params.get('gpu') === 'failed',
  checksForUpdates: true,
  lastUpdateCheck: null,
  latestReleaseVersion: null,
  latestReleasePage: null,
  skippedUpdateVersion: null,
  storageRoot: null,
  showsTrayIcon: true,
  welcomeDone: params.get('welcome') !== '1',
};

const recordingKind = params.get('recording');
let elapsed = 754;

mockWindows('main');
mockConvertFileSrc('windows');
mockIPC(
  (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'bootstrap':
        return {
          settings,
          version: '0.1.0',
          mica: false,
          recordingsRoot: 'C:\\Users\\demo\\AppData\\Local\\Scripta\\Recordings',
          isDefaultStorage: true,
          update: null,
          speechDownloadSize: 672_286_294,
          speakersDownloadSize: 46_250_196,
        };
      case 'list_recordings':
        return recordings;
      case 'list_folders':
        return [
          { id: 'f1', name: 'Projekt Atlas', createdAt: iso(400 * hour) },
          { id: 'f2', name: german ? 'Kunden' : 'Customers', createdAt: iso(300 * hour) },
        ];
      case 'search':
        return recordings.filter((r) => r.title.toLowerCase().includes(String(a.query).toLowerCase())).map((r) => r.id);
      case 'get_recording': {
        const recording = recordings.find((r) => r.id === a.id)!;
        if (recording.id === 'r1') return detail;
        return { recording, audioPath: 'C:\\demo.m4a', transcript: null, voices: {}, speech: {} };
      }
      case 'service_snapshot':
        return { speech: { state: 'ready' }, speakers: { state: 'ready' }, queue: ['r3', 'r4'], running: 'r3', progress: 0.42 };
      case 'recorder_status':
        return recordingKind
          ? {
              state: 'recording',
              isMeeting: recordingKind === 'meeting',
              duration: (elapsed += 0.1),
              microphoneLevel: 0.4 + Math.random() * 0.5,
              systemLevel: Math.random() * 0.8,
              saving: [],
              sourceName: recordingKind === 'meeting' ? 'Microsoft Teams' : null,
              microphoneName: 'Headset (USB)',
            }
          : { state: 'idle', isMeeting: false, duration: 0, microphoneLevel: 0, systemLevel: 0, saving: [], sourceName: null, microphoneName: null };
      case 'list_people':
        return [
          { id: 'p1', name: 'Jonas', samples: 2, confirmedSamples: 2, lastHeard: iso(hour) },
          { id: 'p2', name: 'Mia', samples: 5, confirmedSamples: 4, lastHeard: iso(hour) },
          { id: 'p3', name: 'Lena', samples: 1, confirmedSamples: 1, lastHeard: iso(90 * hour) },
        ];
      case 'list_microphones':
        return {
          devices: [
            { id: 'm1', name: 'Mikrofonarray (Realtek(R) Audio)' },
            { id: 'm2', name: 'Headset (USB)' },
          ],
          defaultId: 'm1',
        };
      case 'list_meeting_apps':
        return [
          { exe: 'ms-teams.exe', name: 'Microsoft Teams' },
          { exe: 'msedge.exe', name: 'Microsoft Edge' },
        ];
      case 'import_extensions':
        return ['mp3', 'm4a', 'wav'];
      case 'model_sizes':
        return { speech: 672_286_294, speakers: 46_250_196, speechDownload: 672_286_294 };
      case 'graphics_cards':
        return params.get('gpu') === 'none'
          ? []
          : [
              { name: 'AMD Radeon(TM) Graphics', memory: 536_870_912 },
              { name: 'AMD Radeon RX 7900 XTX', memory: 25_753_026_560 },
            ];
      case 'has_voice_data':
        return true;
      case 'current_storage_root':
        return 'C:\\Users\\demo\\AppData\\Local\\Scripta\\Recordings';
      case 'get_settings':
        return settings;
      case 'update_settings':
        Object.assign(settings, a.patch);
        return settings;
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

// Live meters in the preview.
if (recordingKind) {
  const { emit } = await import('@tauri-apps/api/event');
  setInterval(async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    void emit('recorder-status', await invoke('recorder_status'));
  }, 100);
}

export const preview = {
  select: params.get('select'),
  style: params.get('style'),
  settings: params.get('settings'),
  assign: params.get('assign') === '1',
};
