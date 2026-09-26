import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../../App.svelte';
import { language } from './store.svelte';
import { liveState, providerCatalog, settingsState } from '../../test/appFixtures';
import type { SettingsViewState } from '../types';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  currentMonitor: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('@tauri-apps/api/window', () => ({
  currentMonitor: mocks.currentMonitor,
  getCurrentWindow: () => ({
    scaleFactor: () => Promise.resolve(1),
    innerSize: () => Promise.resolve({ width: 320, height: 600 }),
  }),
}));

type InvokeArgs = { settings?: SettingsViewState['settings'] };

afterEach(() => {
  cleanup();
  language.resetForTests();
});

describe('chinese interface rendering', () => {
  beforeEach(() => {
    mocks.currentMonitor.mockResolvedValue({
      scaleFactor: 1,
      workArea: { size: { width: 1280, height: 700 } },
    });
    mocks.listen.mockReset().mockResolvedValue(vi.fn());
    mocks.invoke.mockReset().mockImplementation((command: string, args?: InvokeArgs) => {
      if (command === 'get_bootstrap_state') {
        return Promise.resolve({
          usage: liveState,
          settings: settingsState,
          catalog: providerCatalog,
        });
      }
      if (command === 'get_app_settings') return Promise.resolve(settingsState);
      if (command === 'save_app_settings') {
        return Promise.resolve({
          ...settingsState,
          settings: args?.settings ?? settingsState.settings,
        });
      }
      if (command === 'set_resolved_ui_locale') return Promise.resolve();
      if (command === 'get_panel_resize_edge') return Promise.resolve('bottom');
      if (command === 'get_panel_height_mode') return Promise.resolve('automatic');
      if (command === 'fit_panel_to_content') return Promise.resolve(true);
      if (command === 'check_for_updates') {
        return Promise.resolve({
          available: false,
          currentVersion: '0.1.0',
          version: null,
          body: null,
          installable: true,
          releaseUrl: 'https://github.com/liu-zhengdong/OpenQuota/releases/latest',
        });
      }
      return Promise.resolve(null);
    });
  });

  it('renders Dashboard and Settings in chinese after switching language', async () => {
    render(App);
    expect(await screen.findByText('Plus')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Session' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Weekly' })).toBeInTheDocument();

    await fireEvent.click(screen.getByLabelText('Open options'));
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByRole('region', { name: 'Settings' })).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('combobox', { name: 'Language' }));
    await fireEvent.click(screen.getByRole('option', { name: '中文' }));

    await waitFor(() => {
      expect(screen.getByRole('region', { name: '设置' })).toBeInTheDocument();
    });
    expect(screen.getByText('语言')).toBeInTheDocument();
    expect(screen.getByText('通用')).toBeInTheDocument();
    expect(screen.getByText('外观')).toBeInTheDocument();

    const languageRow = screen.getByText('语言').closest('.setting-row');
    expect(languageRow).not.toBeNull();
    if (languageRow instanceof HTMLElement) {
      languageRow.style.width = '280px';
      expect(languageRow.scrollWidth).toBeLessThanOrEqual(Math.max(languageRow.clientWidth, 280));
    }

    await fireEvent.click(screen.getByLabelText('返回'));
    expect(await screen.findByRole('heading', { name: '5 小时窗口' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '每周' })).toBeInTheDocument();
    expect(screen.getByText('选项')).toBeInTheDocument();
  });
});
