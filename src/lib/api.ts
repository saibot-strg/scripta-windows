// Typed calls into the Rust side.

import { invoke } from '@tauri-apps/api/core';
import type {
  Bootstrap,
  ExportFormat,
  Folder,
  MeetingApp,
  Microphones,
  NameAssignment,
  Person,
  RecorderStatus,
  RecordingDetail,
  RecordingSummary,
  ServiceSnapshot,
  Settings,
  UpdateInfo,
} from './types';

export const api = {
  bootstrap: (language: string) => invoke<Bootstrap>('bootstrap', { language }),

  listRecordings: () => invoke<RecordingSummary[]>('list_recordings'),
  listFolders: () => invoke<Folder[]>('list_folders'),
  search: (query: string) => invoke<string[]>('search', { query }),
  getRecording: (id: string) => invoke<RecordingDetail | null>('get_recording', { id }),
  renameRecording: (id: string, title: string) => invoke<void>('rename_recording', { id, title }),
  deleteRecording: (id: string) => invoke<void>('delete_recording', { id }),
  moveToFolder: (ids: string[], folderId: string | null) => invoke<void>('move_to_folder', { ids, folderId }),
  createFolder: (name: string) => invoke<Folder>('create_folder', { name }),
  renameFolder: (id: string, name: string) => invoke<void>('rename_folder', { id, name }),
  deleteFolder: (id: string) => invoke<void>('delete_folder', { id }),
  recordingFile: (id: string) => invoke<string | null>('recording_file', { id }),

  transcribe: (id: string, speakerCount: number | null = null) => invoke<void>('transcribe', { id, speakerCount }),
  cancelTranscription: (id: string) => invoke<void>('cancel_transcription', { id }),
  transcribeAllPending: () => invoke<number>('transcribe_all_pending'),
  serviceSnapshot: () => invoke<ServiceSnapshot>('service_snapshot'),
  prepareModel: (kind: 'speech' | 'speakers') => invoke<void>('prepare_model', { kind }),
  deleteModel: (kind: 'speech' | 'speakers') => invoke<void>('delete_model', { kind }),
  modelSizes: () => invoke<{ speech: number | null; speakers: number | null }>('model_sizes'),

  editSegment: (id: string, text: string) => invoke<void>('edit_segment', { id, textValue: text }),
  restoreSegment: (id: string) => invoke<void>('restore_segment', { id }),
  renameSpeaker: (id: string, name: string, remember = true) => invoke<void>('rename_speaker', { id, name, remember }),
  assignNames: (transcriptId: string, assignments: NameAssignment[]) =>
    invoke<void>('assign_names', { transcriptId, assignments }),
  confirmSuggestion: (id: string) => invoke<void>('confirm_suggestion', { id }),
  mergeSpeaker: (id: string, targetId: string) => invoke<void>('merge_speaker', { id, targetId }),
  moveSegment: (segmentId: string, speakerId: string) => invoke<void>('move_segment', { segmentId, speakerId }),
  moveSegmentToNewSpeaker: (segmentId: string, name: string | null = null) =>
    invoke<void>('move_segment_to_new_speaker', { segmentId, name }),

  listPeople: () => invoke<Person[]>('list_people'),
  hasVoiceData: () => invoke<boolean>('has_voice_data'),
  renamePerson: (id: string, name: string) => invoke<void>('rename_person', { id, name }),
  deletePerson: (id: string) => invoke<void>('delete_person', { id }),
  deleteAllVoices: () => invoke<void>('delete_all_voices'),

  exportTranscript: (id: string, format: ExportFormat, path: string) =>
    invoke<void>('export_transcript', { id, format, path }),
  transcriptText: (id: string, includeHeader: boolean) => invoke<string>('transcript_text', { id, includeHeader }),
  exportFileName: (id: string) => invoke<string>('export_file_name', { id }),

  importFiles: (paths: string[], folderId: string | null = null) => invoke<void>('import_files', { paths, folderId }),
  importExtensions: () => invoke<string[]>('import_extensions'),

  startRecording: () => invoke<void>('start_recording'),
  startMeeting: (appExe: string | null = null) => invoke<void>('start_meeting', { appExe }),
  stopRecording: () => invoke<void>('stop_recording'),
  togglePause: () => invoke<void>('toggle_pause'),
  recorderStatus: () => invoke<RecorderStatus>('recorder_status'),
  listMicrophones: () => invoke<Microphones>('list_microphones'),
  selectMicrophone: (id: string) => invoke<void>('select_microphone', { id }),
  listMeetingApps: () => invoke<MeetingApp[]>('list_meeting_apps'),
  quitAfterSaving: () => invoke<void>('quit_after_saving'),

  getSettings: () => invoke<Settings>('get_settings'),
  updateSettings: (patch: Partial<Settings>) => invoke<Settings>('update_settings', { patch }),
  setStorageRoot: (path: string | null) => invoke<string>('set_storage_root', { path }),
  currentStorageRoot: () => invoke<string>('current_storage_root'),
  openPrivacySettings: () => invoke<void>('open_privacy_settings'),
  openSoundSettings: () => invoke<void>('open_sound_settings'),
  openStorageFolder: () => invoke<void>('open_storage_folder'),
  openLogsFolder: () => invoke<void>('open_logs_folder'),
  checkUpdates: () => invoke<UpdateInfo | null>('check_updates'),
  skipUpdate: (version: string) => invoke<void>('skip_update', { version }),
};
