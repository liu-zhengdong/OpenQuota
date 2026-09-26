import type { LanguagePreference, UiLocale } from './types';

export function detectSystemLanguageTag(
  language = globalThis.navigator?.language ?? '',
  languages = globalThis.navigator?.languages ?? [],
): string {
  const tag = language.trim() || languages.find((item) => item.trim())?.trim() || '';
  return tag;
}

export function isChineseLanguageTag(tag: string): boolean {
  const normalized = tag.trim().toLowerCase().replace(/_/g, '-');
  return normalized === 'zh' || normalized.startsWith('zh-');
}

export function resolveUiLocale(preference: LanguagePreference, systemTag: string): UiLocale {
  if (preference === 'en' || preference === 'zh') return preference;
  return isChineseLanguageTag(systemTag) ? 'zh' : 'en';
}
