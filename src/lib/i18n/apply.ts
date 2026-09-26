import { setResolvedUiLocale } from '../backend';
import { detectSystemLanguageTag } from './locale';
import { language } from './store.svelte';
import type { LanguagePreference, UiLocale } from './types';

export function applyDocumentLocale(locale: UiLocale) {
  if (typeof document === 'undefined') return;
  document.documentElement.lang = locale === 'zh' ? 'zh-CN' : 'en';
}

export function applyLanguagePreference(preference: LanguagePreference) {
  language.setPreference(preference);
  applyDocumentLocale(language.locale);
}

export function notifyBackendLocale(locale: UiLocale) {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
  void setResolvedUiLocale(locale);
}

export function listenForSystemLanguageChanges() {
  const sync = () => {
    language.setSystemTag(detectSystemLanguageTag());
    applyDocumentLocale(language.locale);
    notifyBackendLocale(language.locale);
  };
  window.addEventListener('languagechange', sync);
  return () => window.removeEventListener('languagechange', sync);
}
