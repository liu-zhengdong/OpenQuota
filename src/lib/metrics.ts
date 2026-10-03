import { t } from './i18n';
import type { MetricDefinition, ProviderCatalog, ProviderDefinition } from './types';

export class ProviderCatalogIndex {
  readonly providers: ProviderDefinition[];
  readonly #providersById: Map<string, ProviderDefinition>;
  readonly #metricsById: Map<string, MetricDefinition>;
  readonly #apiKeyProviderIds: Set<string>;

  constructor(catalog: ProviderCatalog) {
    this.providers = catalog.providers;
    this.#providersById = new Map();
    this.#metricsById = new Map();
    this.#apiKeyProviderIds = new Set(catalog.apiKeyProviderIds ?? []);

    for (const provider of catalog.providers) {
      if (this.#providersById.has(provider.id)) {
        throw new Error(`Duplicate provider definition: ${provider.id}`);
      }
      this.#providersById.set(provider.id, provider);
      for (const metric of provider.metrics) {
        if (this.#metricsById.has(metric.id)) {
          throw new Error(`Duplicate metric definition: ${metric.id}`);
        }
        this.#metricsById.set(metric.id, metric);
      }
    }
  }

  provider(id: string) {
    return this.#providersById.get(id);
  }

  metric(id: string) {
    return this.#metricsById.get(id);
  }

  displayName(id: string, providerNames?: Record<string, string>) {
    const customName = providerNames?.[id]?.trim();
    if (customName) return customName;
    const key = `notices.${id}.name`;
    const translated = t(key);
    return translated === key ? (this.provider(id)?.displayName ?? id) : translated;
  }

  supportsApiKeyConfiguration(id: string) {
    return this.#apiKeyProviderIds.has(id);
  }
}

export const emptyProviderCatalog = new ProviderCatalogIndex({ providers: [] });

// Cloud model windows can appear after bootstrap. Keep persisted layouts, including Fable's ID.
export function withSnapshotMetrics(
  catalog: ProviderCatalogIndex,
  view: import('./types').UsageViewState,
): ProviderCatalogIndex {
  return new ProviderCatalogIndex({
    apiKeyProviderIds: catalog.providers
      .filter((p) => catalog.supportsApiKeyConfiguration(p.id))
      .map((p) => p.id),
    providers: catalog.providers.map((provider) => {
      if (!provider.scopedQuotaPrefix) return provider;
      const metrics = [...provider.metrics];
      for (const quota of view.providers[provider.id]?.snapshot?.quotas ?? []) {
        if (
          !quota.id.startsWith(provider.scopedQuotaPrefix) ||
          metrics.some((m) => m.id === `${provider.id}.${quota.id}`)
        )
          continue;
        metrics.push({
          id: `${provider.id}.${quota.id}`,
          label: quota.label,
          source: { kind: 'quota', sourceId: quota.id, sessionWindow: false },
          pinnable: true,
          defaultEnabled: true,
          defaultSection: 'alwaysVisible',
          defaultPinned: false,
          tray: { shortLabel: quota.label, suffix: null },
        });
      }
      return { ...provider, metrics };
    }),
  });
}

export function withCatalogLayouts(
  state: import('./types').SettingsViewState | null,
  catalog: ProviderCatalogIndex,
): import('./types').SettingsViewState | null {
  if (!state) return null;
  return {
    ...state,
    settings: {
      ...state.settings,
      providers: state.settings.providers.map((provider) => {
        const metrics = [...provider.metrics];
        for (const definition of catalog.provider(provider.id)?.metrics ?? []) {
          if (metrics.some((m) => m.id === definition.id)) continue;
          metrics.push({
            id: definition.id,
            enabled: definition.defaultEnabled,
            section: definition.defaultSection,
            pinned: definition.defaultPinned,
          });
        }
        return { ...provider, metrics };
      }),
    },
  };
}

export function metricHasSource(
  definition: MetricDefinition | undefined,
  snapshot: import('./types').ProviderSnapshot,
): boolean {
  if (!definition?.hideWhenMissing) return true;
  const source = definition.source;
  if (source.kind === 'status')
    return snapshot.statusMetrics.some((metric) => metric.id === source.sourceId);
  if (source.kind === 'value')
    return snapshot.valueMetrics.some((metric) => metric.id === source.sourceId);
  return (
    snapshot.quotas.some((metric) => metric.id === source.sourceId) ||
    (source.kind === 'quotaOrValue' &&
      snapshot.valueMetrics.some((metric) => metric.id === source.sourceId))
  );
}
