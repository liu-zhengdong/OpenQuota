import { t } from './i18n';
import { usageWord } from './i18n/labels';
import type { QuotaWindow } from './types';

type UsageDisplay = 'used' | 'left';

/** Absolute readings must come from the source, never limit minus used. */
export function quotaReading(quota: QuotaWindow, display: UsageDisplay, full = false): string {
  const value = display === 'left' ? quota.remainingValue : quota.usedValue;
  if (value != null && Number.isFinite(value)) {
    if (quota.format === 'count') {
      return t('metrics.countReading', {
        value: String(value),
        unit: quota.unit?.trim() || t('common.unknownUnit'),
        direction: usageWord(display),
      });
    }
    if (quota.format === 'dollars') {
      const rounded = value.toFixed(2);
      return t(display === 'left' ? 'metrics.dollarsLeft' : 'metrics.dollarsSpent', {
        value: full || (value > 0 && Number(rounded) === 0) ? String(value) : rounded,
      });
    }
  }
  const percent = quotaFillPercent(quota, display);
  const rounded = Math.round(percent);
  // Display rounding must not turn a non-empty window into 0% left / 100% used.
  const boundary = percent > 0 && percent < 100 && (rounded === 0 || rounded === 100);
  return t('metrics.percentReading', {
    percent: full || boundary ? String(Number(percent.toPrecision(12))) : String(rounded),
    direction: usageWord(display),
  });
}

export function quotaFillPercent(quota: QuotaWindow, display: UsageDisplay): number {
  const used = Math.min(100, Math.max(0, quota.usedPercent));
  return display === 'used' ? used : 100 - used;
}
