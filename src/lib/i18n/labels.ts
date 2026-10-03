import type { MetricDefinition, MetricSource, ProviderLink } from '../types';
import { t, windowLabel } from './store.svelte';

export function sourceWindowId(source: MetricSource): string {
  return source.sourceId;
}

export function metricLabel(definition: Pick<MetricDefinition, 'label' | 'source'>): string {
  return windowLabel(sourceWindowId(definition.source), definition.label);
}

const LINK_LABELS: Record<string, string> = {
  Status: 'common.linkStatus',
  Dashboard: 'common.linkDashboard',
  Activity: 'common.linkActivity',
  Credits: 'common.linkCredits',
  'API Keys': 'common.linkApiKeys',
};

export function linkLabel(label: string): string {
  const key = LINK_LABELS[label];
  return key ? t(key) : label;
}

export function translatedLinks(links: ProviderLink[]): ProviderLink[] {
  return links.map((link) => ({ ...link, label: linkLabel(link.label) }));
}

export function usageWord(display: 'used' | 'left'): string {
  return t(display === 'used' ? 'common.used' : 'common.left');
}

export function providerDescription(providerId: string): string {
  const key = `notices.${providerId}.description`;
  const description = t(key);
  return description === key ? '' : description;
}
