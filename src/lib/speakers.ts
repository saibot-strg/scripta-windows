// Speaker colors and helpers shared by the transcript views.

import type { RecordingDetail, Speaker } from './types';

/** Accent for "Ich", neutral for "Andere", distinct hues for numbered speakers (repeating after six). */
export function tint(speaker: Speaker | null | undefined): string {
  if (!speaker) return 'var(--text-2)';
  if (speaker.role === 'me') return 'var(--accent)';
  if (speaker.role === 'others') return 'var(--text-2)';
  return `var(--speaker-${((Math.max(speaker.number, 1) - 1) % 6) + 1})`;
}

/** The name was set from a guess and not confirmed yet. */
export function isGuess(speaker: Speaker, detail: RecordingDetail): boolean {
  const status = detail.voices[speaker.id];
  return !!status && status.state === 'suggested' && !!status.personName && speaker.customName === status.personName;
}

/** "Ich" first, then "Andere" and the numbered speakers. */
export function sortedSpeakers(speakers: Speaker[]): Speaker[] {
  return [...speakers].sort((a, b) => (a.role === 'me' ? 0 : 1) - (b.role === 'me' ? 0 : 1) || a.number - b.number);
}
