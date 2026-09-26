import { describe, expect, it } from 'vitest';
import { providerCatalog, claudeState, settingsState } from '../test/appFixtures';
import { ProviderCatalogIndex, withSnapshotMetrics, withCatalogLayouts } from './metrics';

describe('provider catalog index', () => {
  it('indexes provider identity and metric metadata from bootstrap data', () => {
    const catalog = new ProviderCatalogIndex(providerCatalog);

    expect(catalog.displayName('codex')).toBe('Codex');
    expect(catalog.displayName('codex', { codex: '  Work Account  ' })).toBe('Work Account');
    expect(catalog.metric('claude.session')).toMatchObject({
      label: 'Session',
      source: { kind: 'quota', sourceId: 'session', sessionWindow: true },
    });
    expect(catalog.supportsApiKeyConfiguration('openrouter')).toBe(true);
    expect(catalog.supportsApiKeyConfiguration('codex')).toBe(false);
    expect(catalog.metric('openrouter.balance')).toMatchObject({
      label: 'Balance',
      source: { kind: 'value', sourceId: 'balance' },
    });
    expect(catalog.provider('codex')?.links).toEqual([
      { label: 'Status', url: 'https://status.openai.com/' },
      { label: 'Dashboard', url: 'https://chatgpt.com/codex/settings/usage' },
    ]);
  });

  it('uses safe unknown-provider fallbacks without borrowing another provider identity', () => {
    const catalog = new ProviderCatalogIndex(providerCatalog);

    expect(catalog.displayName('future-provider')).toBe('future-provider');
    expect(catalog.metric('future-provider.session')).toBeUndefined();
  });

  it('rejects duplicate provider and metric ids at the frontend boundary', () => {
    const provider = structuredClone(providerCatalog.providers[1]);
    expect(
      () => new ProviderCatalogIndex({ providers: [provider, structuredClone(provider)] }),
    ).toThrow('Duplicate provider definition: codex');

    const duplicateMetric = structuredClone(provider);
    duplicateMetric.metrics.push(structuredClone(duplicateMetric.metrics[0]));
    expect(() => new ProviderCatalogIndex({ providers: [duplicateMetric] })).toThrow(
      'Duplicate metric definition: codex.session',
    );
  });
});

describe('cloud scoped quota layouts', () => {
  it('adds all model windows without replacing saved Fable and custom model layouts', () => {
    const base = new ProviderCatalogIndex(providerCatalog);
    const state = structuredClone(claudeState);
    state.snapshot!.quotas = [
      { ...state.snapshot!.quotas[0], id: 'fable', label: 'Fable' },
      { ...state.snapshot!.quotas[0], id: 'scoped-opus-5-5', label: 'Opus 5.5' },
    ];
    const catalog = withSnapshotMetrics(base, { providers: { claude: state } });
    expect(catalog.metric('claude.scoped-opus-5-5')).toMatchObject({
      label: 'Opus 5.5',
      source: { kind: 'quota', sourceId: 'scoped-opus-5-5' },
    });
    const saved = structuredClone(settingsState);
    saved.settings.providers = [
      {
        id: 'claude',
        enabled: true,
        detected: true,
        expanded: true,
        metrics: [{ id: 'claude.fable', enabled: false, section: 'onDemand', pinned: true }],
      },
    ];
    const merged = withCatalogLayouts(saved, catalog)!;
    expect(merged.settings.providers[0].metrics[0]).toEqual(saved.settings.providers[0].metrics[0]);
    expect(
      merged.settings.providers[0].metrics.find((m) => m.id === 'claude.scoped-opus-5-5'),
    ).toMatchObject({ enabled: true, section: 'alwaysVisible' });
    const opus = merged.settings.providers[0].metrics.find(
      (m) => m.id === 'claude.scoped-opus-5-5',
    )!;
    opus.enabled = false;
    opus.pinned = true;
    expect(
      withCatalogLayouts(merged, catalog)!.settings.providers[0].metrics.find(
        (m) => m.id === opus.id,
      ),
    ).toEqual(opus);
    expect(base.metric(opus.id)).toBeUndefined();
  });
});
