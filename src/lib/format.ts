// Display formatting for durations, dates and languages.

import { locale, t } from './i18n';

/** "2:34" or "1:02:03" (rounded down). */
export function duration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds || 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const secs = total % 60;
  const two = (n: number) => String(n).padStart(2, '0');
  return hours > 0 ? `${hours}:${two(minutes)}:${two(secs)}` : `${minutes}:${two(secs)}`;
}

/** Spoken duration for screen readers, e.g. "2 Minuten, 34 Sekunden". */
export function spokenDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds || 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const secs = total % 60;
  const parts: string[] = [];
  if (hours) parts.push(`${hours} ${t(hours === 1 ? 'Stunde' : 'Stunden')}`);
  if (minutes) parts.push(`${minutes} ${t(minutes === 1 ? 'Minute' : 'Minuten')}`);
  parts.push(`${secs} ${t(secs === 1 ? 'Sekunde' : 'Sekunden')}`);
  return parts.join(', ');
}

/** Wall-clock time, e.g. "15:26". */
export function clockTime(date: Date): string {
  return new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit' }).format(date);
}

/** Mail-style list date: time for today, "Gestern", otherwise the date. */
export function listDate(iso: string, now = new Date()): string {
  const date = new Date(iso);
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  if (sameDay(date, now)) return clockTime(date);
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(date, yesterday)) return t('Gestern');
  return new Intl.DateTimeFormat(locale(), { day: '2-digit', month: '2-digit', year: 'numeric' }).format(date);
}

/** "Freitag, 23. Oktober 2026 um 14:05". */
export function fullDate(iso: string): string {
  return new Intl.DateTimeFormat(locale(), { dateStyle: 'full', timeStyle: 'short' }).format(new Date(iso));
}

/** Language name for a BCP-47 code, e.g. "de" → "Deutsch". */
export function languageName(code: string): string {
  try {
    return new Intl.DisplayNames([locale()], { type: 'language' }).of(code) ?? code;
  } catch {
    return code;
  }
}

export function percent(fraction: number): string {
  return new Intl.NumberFormat(locale(), { style: 'percent', maximumFractionDigits: 0 }).format(fraction || 0);
}

export function bytes(count: number): string {
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = count;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${new Intl.NumberFormat(locale(), { maximumFractionDigits: unit >= 2 ? 0 : 1 }).format(value)} ${units[unit]}`;
}

export function rate(value: number): string {
  return `${new Intl.NumberFormat(locale(), { maximumFractionDigits: 2 }).format(value)}×`;
}
