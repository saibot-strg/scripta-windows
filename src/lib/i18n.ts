// German is the source language; English translations by German text.
// `t('Hallo {name}', { name })` fills in placeholders.

import { english } from './i18n.en';

const language = (navigator.language || 'de').toLowerCase().startsWith('de') ? 'de' : 'en';

export function locale(): string {
  return language === 'de' ? 'de-DE' : navigator.language || 'en-US';
}

export function isGerman(): boolean {
  return language === 'de';
}

export function t(text: string, params?: Record<string, string | number>): string {
  let result = language === 'de' ? text : (english[text] ?? text);
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      result = result.replaceAll(`{${key}}`, String(value));
    }
  }
  return result;
}
