import type { ProviderMessageParams, ProviderNotice, ProviderWarning } from '../types';
import { catalogs } from './catalog';
import { interpolate } from './interpolate';
import { language, t } from './store.svelte';
import type { MessageVars, UiLocale } from './types';

// Notices and warnings reach the panel as stable ids plus locale-independent params, so the
// interface renders its own language without waiting for another provider read. Anything the
// catalogs do not cover falls back to the English copy the provider shipped with.

const DATA_FROM_KEY = 'notices.dataFrom';

function render(locale: UiLocale, key: string, vars: MessageVars): string | null {
  const template = catalogs[locale][key] ?? catalogs.en[key];
  return template === undefined ? null : interpolate(template, vars);
}

/**
 * The provider names the catalog entry holding the message, which keeps this resolver free of
 * provider-specific branches: `claude.rateLimited` is looked up, not known.
 */
function messageVariant(notice: ProviderNotice): string | null {
  const variant = notice.params?.variant;
  return variant ? `notices.${notice.id}.${variant}` : null;
}

function formatRetryDuration(seconds: number): string {
  if (seconds < 60)
    return t('common.durationSeconds', { seconds: Math.max(1, Math.round(seconds)) });
  const minutes = Math.ceil(seconds / 60);
  return t(minutes === 1 ? 'common.durationOneMinute' : 'common.durationMinutes', { minutes });
}

function formatReadTime(iso: string): string | null {
  const parsed = Date.parse(iso);
  if (!Number.isFinite(parsed)) return null;
  const locale =
    typeof document !== 'undefined' ? document.documentElement.lang || undefined : undefined;
  return new Date(parsed).toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' });
}

/**
 * Expands the provider's raw params into the placeholders the catalog templates use, so a
 * template never has to know whether the provider sent seconds or a formatted string.
 */
function messageVars(params: ProviderMessageParams | undefined): MessageVars {
  const vars: MessageVars = { ...params };
  if (params?.retrySeconds !== undefined) {
    const seconds = Number(params.retrySeconds);
    vars.duration = formatRetryDuration(Number.isFinite(seconds) && seconds > 0 ? seconds : 0);
  }
  const rememberedAt = params?.rememberedAt;
  if (rememberedAt) {
    const time = formatReadTime(rememberedAt);
    if (time) vars.time = time;
  }
  return vars;
}

export function noticeTitle(notice: ProviderNotice): string {
  return (
    render(language.locale, `notices.${notice.id}.title`, messageVars(notice.params)) ??
    notice.title
  );
}

export function noticeMessage(notice: ProviderNotice): string {
  const locale = language.locale;
  const vars = messageVars(notice.params);
  const variant = messageVariant(notice);
  const body = variant ? render(locale, variant, vars) : null;
  if (body === null) return notice.message;
  // Remembered numbers carry their read time, otherwise they read as current.
  const rememberedAt = notice.params?.rememberedAt;
  if (!rememberedAt) return body;
  const dataFrom = render(locale, DATA_FROM_KEY, vars);
  return dataFrom === null ? body : `${body} · ${dataFrom}`;
}

export function warningText(warning: ProviderWarning): string {
  if (!warning.id) return warning.fallback;
  return (
    render(language.locale, `warnings.${warning.id}`, messageVars(warning.params)) ??
    warning.fallback
  );
}

export function warningTexts(warnings: ProviderWarning[]): string[] {
  return warnings.map(warningText);
}
