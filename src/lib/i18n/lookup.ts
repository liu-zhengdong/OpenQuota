import { catalogs } from './catalog';
import { interpolate } from './interpolate';
import type { MessageVars, UiLocale } from './types';

export function lookup(locale: UiLocale, key: string, vars?: MessageVars): string {
  const template = catalogs[locale][key] ?? catalogs.en[key] ?? key;
  return interpolate(template, vars);
}

export function lookupWindowLabel(locale: UiLocale, id: string, fallback: string): string {
  return catalogs[locale][`windows.${id}`] ?? catalogs.en[`windows.${id}`] ?? fallback;
}
