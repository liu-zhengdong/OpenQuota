import { t } from './i18n';
import type { QuotaWindow } from './types';

export type PaceSeverity = 'level' | 'healthy' | 'close' | 'runningOut' | 'spent';

export interface PaceProjection {
  severity: PaceSeverity;
  projectedUsedPercent: number | null;
  evenPacePercent: number | null;
  runOutAt: number | null;
}

export function projectPace(window: QuotaWindow, now: number): PaceProjection {
  const used = clamp(window.usedPercent, 0, 100);
  if (isVisiblySpent(window, used)) {
    return { severity: 'spent', projectedUsedPercent: 100, evenPacePercent: null, runOutAt: now };
  }
  if (used <= 0) return level();
  const reset = window.resetsAt ? new Date(window.resetsAt).getTime() : Number.NaN;
  if (!Number.isFinite(reset) || reset <= now || window.periodSeconds <= 0) return level();
  const periodMs = window.periodSeconds * 1000;
  const start = reset - periodMs;
  const elapsed = Math.max(0, now - start);
  const progress = clamp(elapsed / periodMs, 0, 1);
  if (elapsed < Math.max(60_000, periodMs * 0.01)) return level();
  const projected = used / progress;
  if (projected <= 90) {
    return {
      severity: 'healthy',
      projectedUsedPercent: projected,
      evenPacePercent: progress * 100,
      runOutAt: null,
    };
  }
  if (used < 5) return level();
  if (projected <= 100) {
    const spare = Math.round(100 - projected);
    return {
      severity: spare >= 1 ? 'close' : 'runningOut',
      projectedUsedPercent: projected,
      evenPacePercent: progress * 100,
      runOutAt: null,
    };
  }
  const candidate = start + (elapsed * 100) / used;
  return {
    severity: 'runningOut',
    projectedUsedPercent: projected,
    evenPacePercent: progress * 100,
    runOutAt: candidate > now && candidate < reset ? candidate : null,
  };
}

/**
 * Window used to compare a provider against an even pace: the weekly percent
 * window when one exists, otherwise the percent window with the longest period.
 */
export function selectComparisonWindow(windows: QuotaWindow[]): QuotaWindow | null {
  const percent = windows.filter((window) => window.format === 'percent');
  const weekly = percent.filter(
    (window) =>
      window.id.toLowerCase().includes('week') || window.label.toLowerCase().includes('week'),
  );
  const candidates = weekly.length > 0 ? weekly : percent;
  let longest: QuotaWindow | null = null;
  for (const window of candidates) {
    if (!longest || window.periodSeconds > longest.periodSeconds) longest = window;
  }
  return longest;
}

/**
 * Percentage points between elapsed time and consumption for a window, or null
 * when the pacing projection reports no elapsed time for it.
 */
export function sparePercent(window: QuotaWindow | null, now: number): number | null {
  if (!window) return null;
  const pace = projectPace(window, now);
  if (pace.evenPacePercent === null) return null;
  const used = Math.min(100, Math.max(0, window.usedPercent));
  return pace.evenPacePercent - used;
}

export function isFreshSessionWindow(window: QuotaWindow, now: number, isSessionWindow: boolean) {
  if (!isSessionWindow || window.usedPercent > 0 || !window.resetsAt) return false;
  const reset = new Date(window.resetsAt).getTime();
  return Number.isFinite(reset) && now < reset;
}

export function paceTooltip(value: PaceProjection) {
  if (value.severity === 'level') return null;
  if (value.severity === 'spent') return t('metrics.limitReached');
  const projected = value.projectedUsedPercent;
  if (projected === null) return null;
  if (value.severity === 'healthy')
    return t('metrics.healthyPace', { percent: Math.round(100 - projected) });
  if (value.severity === 'close') return t('metrics.closePace', { percent: Math.round(projected) });
  if (projected <= 100) return t('metrics.evenPace');
  return t('metrics.overPace', { percent: Math.max(1, Math.round(projected - 100)) });
}

type TimeFormat = 'system' | 'twelveHour' | 'twentyFourHour';

export function formatReset(
  value: string | null,
  now: number,
  mode: 'countdown' | 'exact',
  timeFormat: TimeFormat = 'system',
) {
  if (!value) return t('metrics.resetsUnavailable');
  const reset = new Date(value).getTime();
  if (!Number.isFinite(reset)) return t('metrics.resetsUnavailable');
  return formatDeadline('resets', reset, now, mode, timeFormat);
}

export function formatLimit(
  value: number | null,
  now: number,
  mode: 'countdown' | 'exact',
  timeFormat: TimeFormat = 'system',
) {
  if (value === null) return t('metrics.limitReached');
  return formatDeadline('limit', value, now, mode, timeFormat);
}

export function formatResetDuration(value: string | null, now: number) {
  const remaining = remainingMs(value, now);
  if (remaining === null || remaining <= 5 * 60_000) return null;
  return formatDuration(remaining);
}

export function formatResetWhen(
  value: string | null,
  now: number,
  timeFormat: TimeFormat = 'system',
) {
  const remaining = remainingMs(value, now);
  if (remaining === null) return t('metrics.resetsUnavailable');
  if (remaining <= 0) return t('metrics.expiringSoon');
  return formatClock(new Date(value as string).getTime(), now, timeFormat, 'when');
}

function remainingMs(value: string | null, now: number) {
  if (!value) return null;
  const reset = new Date(value).getTime();
  if (!Number.isFinite(reset)) return null;
  return reset - now;
}

function formatDeadline(
  kind: 'resets' | 'limit',
  value: number,
  now: number,
  mode: 'countdown' | 'exact',
  timeFormat: TimeFormat,
) {
  const remaining = value - now;
  if (remaining <= 0 || (mode === 'countdown' && remaining <= 5 * 60_000)) {
    return t(kind === 'resets' ? 'metrics.resetsSoon' : 'metrics.limitSoon');
  }
  if (mode === 'countdown') {
    return t(kind === 'resets' ? 'metrics.resetsIn' : 'metrics.limitIn', {
      duration: formatDuration(remaining),
    });
  }
  return formatClock(value, now, timeFormat, kind);
}

function formatClock(
  value: number,
  now: number,
  timeFormat: TimeFormat,
  kind: 'resets' | 'limit' | 'when',
) {
  const date = new Date(value);
  const current = new Date(now);
  const currentDay = Date.UTC(current.getFullYear(), current.getMonth(), current.getDate());
  const targetDay = Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
  const dayDifference = Math.round((targetDay - currentDay) / 86_400_000);
  const dateLocale = documentLocale();
  const time = date.toLocaleTimeString(dateLocale, {
    hour: 'numeric',
    minute: '2-digit',
    hour12: timeFormat === 'system' ? undefined : timeFormat === 'twelveHour',
  });
  if (dayDifference <= 0) return t(clockKey(kind, 'today'), { time });
  if (dayDifference === 1) return t(clockKey(kind, 'tomorrow'), { time });
  const monthDay = new Intl.DateTimeFormat(dateLocale, {
    month: 'short',
    day: 'numeric',
  }).format(date);
  return t(clockKey(kind, 'on'), { date: monthDay, time });
}

function clockKey(kind: 'resets' | 'limit' | 'when', when: 'today' | 'tomorrow' | 'on') {
  if (kind === 'when') {
    if (when === 'today') return 'metrics.whenToday';
    if (when === 'tomorrow') return 'metrics.whenTomorrow';
    return 'metrics.whenOn';
  }
  if (kind === 'resets') {
    if (when === 'today') return 'metrics.resetsToday';
    if (when === 'tomorrow') return 'metrics.resetsTomorrow';
    return 'metrics.resetsOn';
  }
  if (when === 'today') return 'metrics.limitToday';
  if (when === 'tomorrow') return 'metrics.limitTomorrow';
  return 'metrics.limitOn';
}

function documentLocale(): string | undefined {
  return typeof document !== 'undefined' && document.documentElement.lang
    ? document.documentElement.lang
    : undefined;
}

function formatDuration(milliseconds: number) {
  const minutes = Math.max(1, Math.ceil(milliseconds / 60_000));
  const days = Math.floor(minutes / 1_440);
  const hours = Math.floor((minutes % 1_440) / 60);
  const remainder = minutes % 60;
  if (days > 0) return t('metrics.durationDaysHours', { days, hours });
  if (hours > 0)
    return remainder > 0
      ? t('metrics.durationHoursMinutes', { hours, minutes: remainder })
      : t('metrics.durationHours', { hours });
  return t('metrics.durationMinutes', { minutes: remainder });
}

function level(): PaceProjection {
  return { severity: 'level', projectedUsedPercent: null, evenPacePercent: null, runOutAt: null };
}

function isVisiblySpent(window: QuotaWindow, usedPercent: number) {
  if (
    window.format === 'dollars' &&
    window.usedValue !== null &&
    window.limitValue !== null &&
    window.limitValue > 0
  ) {
    return Math.round((window.limitValue - window.usedValue) * 100) / 100 <= 0;
  }
  return Math.round(100 - usedPercent) <= 0;
}

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(maximum, Math.max(minimum, value));
}
