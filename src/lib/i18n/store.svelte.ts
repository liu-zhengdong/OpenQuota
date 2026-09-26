import { detectSystemLanguageTag, resolveUiLocale } from './locale';
import { lookup, lookupWindowLabel } from './lookup';
import type { LanguagePreference, MessageVars, UiLocale } from './types';

class LanguageStore {
  preference = $state<LanguagePreference>('system');
  systemTag = $state(detectSystemLanguageTag());
  locale = $derived(resolveUiLocale(this.preference, this.systemTag));

  setPreference(preference: LanguagePreference) {
    this.preference = preference;
  }

  setSystemTag(tag: string) {
    this.systemTag = tag;
  }

  resetForTests(preference: LanguagePreference = 'system', systemTag = 'en-US') {
    this.preference = preference;
    this.systemTag = systemTag;
  }
}

export const language = new LanguageStore();

export function t(key: string, vars?: MessageVars): string {
  return lookup(language.locale, key, vars);
}

export function windowLabel(id: string, fallback: string): string {
  return lookupWindowLabel(language.locale, id, fallback);
}

export function currentLocale(): UiLocale {
  return language.locale;
}
