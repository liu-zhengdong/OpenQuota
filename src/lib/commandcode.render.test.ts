import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import Dashboard from './Dashboard.svelte';
import CustomizeProviderDetail from './CustomizeProviderDetail.svelte';
import { ProviderCatalogIndex, metricHasSource } from './metrics';
import { language } from './i18n';
import { settingsState } from '../test/appFixtures';
import definitionJson from '../../src-tauri/src/providers/commandcode/fixtures/definition.json';
import snapshotJson from '../../src-tauri/src/providers/commandcode/fixtures/snapshot.json';
import type { ProviderDefinition, ProviderSnapshot, AppSettings } from './types';

const definition = definitionJson as ProviderDefinition;
const snapshot = snapshotJson as ProviderSnapshot;
const catalog = new ProviderCatalogIndex({ providers: [definition], apiKeyProviderIds: [] });
const settings: AppSettings = {
  ...settingsState.settings,
  providers: [
    {
      id: definition.id,
      enabled: true,
      detected: true,
      expanded: false,
      metrics: definition.metrics.map((m) => ({
        id: m.id,
        enabled: m.defaultEnabled,
        section: m.defaultSection,
        pinned: m.defaultPinned,
      })),
    },
  ],
};
afterEach(() => {
  cleanup();
  language.resetForTests();
});
function show(reading: ProviderSnapshot) {
  return render(Dashboard, {
    catalog,
    settings,
    now: Date.parse(reading.refreshedAt),
    viewState: {
      providers: {
        commandcode: {
          snapshot: reading,
          source: 'live',
          refreshing: false,
          stale: false,
          error: null,
          errorKind: null,
          lastAttemptAt: null,
        },
      },
    },
    renamableProviderIds: [],
    reducedMotion: true,
    updateStatus: null,
    installingUpdate: false,
    updateProgress: null,
    updateError: null,
    onSettingsChange: vi.fn(),
    onCustomizationChange: vi.fn(),
    onReorderStart: vi.fn(),
    onReorderEnd: vi.fn(),
    onCustomize: vi.fn(),
    onOpenProviderCustomize: vi.fn(),
    onRenameProvider: vi.fn(),
    onShare: vi.fn(),
    onRefresh: vi.fn(),
    onOpenProviderLink: vi.fn(),
    onContentMorph: vi.fn(),
    onInstallUpdate: vi.fn(),
    onOpenUpdatePage: vi.fn(),
  });
}
it.each(['en', 'zh'] as const)(
  'renders the backend plan, remaining, free credits and windows in %s',
  (locale) => {
    language.setPreference(locale);
    const { container } = show(snapshot);
    expect(screen.getByText('Command Code')).toBeInTheDocument();
    expect(screen.getByText('individual-pro')).toBeInTheDocument();
    expect(screen.getByText(locale === 'zh' ? '总剩余' : 'Total remaining')).toBeInTheDocument();
    expect(screen.getByText(locale === 'zh' ? '免费额度' : 'Free credits')).toBeInTheDocument();
    expect(screen.getByText('$20.00')).toBeInTheDocument();
    expect(screen.getByText('$5.00')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: locale === 'zh' ? '5 小时' : '5-hour' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: locale === 'zh' ? '每周' : 'Weekly' }),
    ).toBeInTheDocument();
    expect(container.querySelector('.provider-icon path')?.getAttribute('d')).toBeTruthy();
  },
);
it('hides absent windows without hiding total or free credits', () => {
  show({ ...snapshot, quotas: [] });
  expect(screen.queryByRole('heading', { name: '5-hour' })).not.toBeInTheDocument();
  expect(screen.queryByRole('heading', { name: 'Weekly' })).not.toBeInTheDocument();
  expect(screen.getByText('Free credits')).toBeInTheDocument();
});
it('shows the CLI login description and all configurable metrics without a second API-key store', () => {
  language.setPreference('zh');
  render(CustomizeProviderDetail, {
    settings,
    providerId: 'commandcode',
    catalog,
    renamableProviderIds: [],
    reducedMotion: true,
    onChange: vi.fn(),
    onNameChange: vi.fn(),
    onReorderStart: vi.fn(),
    onReorderEnd: vi.fn(),
  });
  expect(screen.getByText(/command-code auth login/)).toBeInTheDocument();
  expect(screen.getByText('免费额度')).toBeInTheDocument();
  expect(screen.getByText('5 小时')).toBeInTheDocument();
  expect(screen.queryByText('API Key')).not.toBeInTheDocument();
  expect(catalog.supportsApiKeyConfiguration('commandcode')).toBe(false);
});
it('only hides metrics declared optional, preserving other providers missing-data states', () => {
  const absent = { ...snapshot, quotas: [] };
  expect(metricHasSource(catalog.metric('commandcode.fiveHour'), absent)).toBe(false);
  expect(catalog.metric('commandcode.credits')).toBeUndefined();
  expect(metricHasSource(catalog.metric('commandcode.remaining'), absent)).toBe(true);
  expect(metricHasSource(catalog.metric('commandcode.weekly'), snapshot)).toBe(true);
});
