import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import ProviderNoticeRow from './ProviderNoticeRow.svelte';
import { language } from './i18n/store.svelte';
import type { ProviderNotice } from './types';

// Mirrors what `claude::rate_limit_notice` sends over the Tauri boundary.
const rateLimited = (params: ProviderNotice['params']): ProviderNotice => ({
  id: 'claude.rateLimited',
  title: 'Live usage paused',
  message: 'Showing the last successful limits · Retrying in about 5 minutes',
  params,
  tone: 'warning',
});

afterEach(() => {
  cleanup();
  language.resetForTests();
});

describe('ProviderNoticeRow', () => {
  it('renders a compact warning status without hiding its retry context', () => {
    render(ProviderNoticeRow, {
      notice: {
        id: 'rateLimited',
        title: 'Live usage paused',
        message: 'Retrying in about 5 minutes',
        tone: 'warning',
      },
    });

    expect(screen.getByRole('status')).toHaveTextContent(
      'Live usage pausedRetrying in about 5 minutes',
    );
  });

  it('falls back to the provider copy when the id has no catalog entry', () => {
    language.resetForTests('zh');
    render(ProviderNoticeRow, {
      notice: {
        id: 'someProvider.unknownNotice',
        title: 'Live usage paused',
        message: 'Retrying in about 5 minutes',
        tone: 'warning',
      },
    });

    expect(screen.getByRole('status')).toHaveTextContent(
      'Live usage pausedRetrying in about 5 minutes',
    );
  });

  it('translates the claude rate limit notice into chinese', () => {
    language.resetForTests('zh');
    render(ProviderNoticeRow, {
      notice: rateLimited({ variant: 'retry', retrySeconds: '300' }),
    });

    const status = screen.getByRole('status');
    expect(status).toHaveTextContent('实时用量已暂停');
    expect(status).toHaveTextContent('约 5 分钟后重试');
    expect(status).not.toHaveTextContent('Live usage paused');
  });

  it('labels remembered limits with the time they were read in chinese', () => {
    language.resetForTests('zh');
    const { container } = render(ProviderNoticeRow, {
      notice: rateLimited({
        variant: 'rememberedRetry',
        retrySeconds: '300',
        rememberedAt: '2026-09-27T06:32:00Z',
      }),
    });

    const status = screen.getByRole('status');
    expect(status).toHaveTextContent('显示上次成功的限额 · 约 5 分钟后重试');
    expect(container.textContent).toMatch(/数据来自 \d{2}:\d{2}/);
    expect(container.textContent).not.toContain('Retrying in about');
  });

  it('uses one minute for a single minute retry in both languages', () => {
    language.resetForTests('en');
    const { unmount } = render(ProviderNoticeRow, {
      notice: rateLimited({ variant: 'retry', retrySeconds: '60' }),
    });
    expect(screen.getByRole('status')).toHaveTextContent('Retrying in about 1 minute');
    unmount();

    language.resetForTests('zh');
    render(ProviderNoticeRow, {
      notice: rateLimited({ variant: 'retry', retrySeconds: '60' }),
    });
    expect(screen.getByRole('status')).toHaveTextContent('约 1 分钟后重试');
  });
});
