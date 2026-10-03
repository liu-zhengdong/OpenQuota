import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import QuotaMetric from './QuotaMetric.svelte';
import type { QuotaWindow } from './types';

const now = Date.parse('2026-07-10T12:00:00Z');
const periodSeconds = 10_000;

afterEach(cleanup);

function quota(usedPercent: number, elapsedFraction = 0.5): QuotaWindow {
  return {
    id: 'weekly',
    label: 'Weekly',
    usedPercent,
    format: 'percent',
    usedValue: null,
    limitValue: null,
    estimated: false,
    periodSeconds,
    resetsAt: new Date(now + (1 - elapsedFraction) * periodSeconds * 1000).toISOString(),
  };
}

function show(value: QuotaWindow, onToggleReset = vi.fn(), isSessionWindow = false) {
  return {
    onToggleReset,
    ...render(QuotaMetric, {
      quota: value,
      now,
      usageDisplay: 'left',
      resetDisplay: 'countdown',
      timeFormat: 'system',
      alwaysShowPacing: false,
      isSessionWindow,
      onToggleUsage: vi.fn(),
      onToggleReset,
    }),
  };
}

function showAlways(value: QuotaWindow) {
  return render(QuotaMetric, {
    quota: value,
    now,
    usageDisplay: 'left',
    resetDisplay: 'countdown',
    timeFormat: 'system',
    alwaysShowPacing: true,
    isSessionWindow: false,
    onToggleUsage: vi.fn(),
    onToggleReset: vi.fn(),
  });
}

describe('quota pacing presentation', () => {
  it('shows the flame, run-out time, and projection tooltip', async () => {
    const onToggleReset = vi.fn();
    const { container } = show(quota(60), onToggleReset);
    const warning = screen.getByRole('button', { name: 'Limit in 56m' });
    expect(container.querySelector('.pace-warning__icon')).toBeInTheDocument();
    expect(warning).toHaveAttribute('data-tooltip', '~20% over limit at reset');
    expect(container.querySelector('.meter-shell')).toHaveAttribute(
      'data-tooltip',
      '~20% over limit at reset',
    );
    expect(container.querySelector('.meter__fill')).toHaveStyle('--fill-percent: 40%');
    expect(screen.getByRole('button', { name: '40% left' })).toHaveAttribute(
      'data-tooltip',
      '60% used',
    );
    expect(screen.getByRole('button', { name: /Resets in/ })).toHaveAttribute(
      'data-tooltip',
      expect.stringContaining('Resets today at'),
    );
    await fireEvent.click(warning);
    expect(onToggleReset).toHaveBeenCalledOnce();
  });

  it('shows a flame without a misleading time at the exact-limit edge', () => {
    const { container } = show(quota(50));
    expect(screen.getByLabelText('Will reach limit')).toBeInTheDocument();
    expect(screen.queryByText(/Limit in/)).not.toBeInTheDocument();
    expect(container.querySelector('.pace-warning')).toHaveAttribute(
      'data-tooltip',
      '~100% used at reset',
    );
  });

  it('shows the rounded spare copy and projected-use tooltip when close', () => {
    show(quota(46));
    expect(screen.getByText('~8% spare')).toHaveAttribute('data-tooltip', '~92% used at reset');
  });

  it('frames the even-pace tick in the selected remaining mode', () => {
    const { container } = show(quota(30, 0.25));
    expect(container.querySelector('.meter__pace')).toHaveStyle('--pace-percent: 75%');
  });

  it('shows an unused rolling session as not started without pacing decoration', () => {
    const { container } = show(quota(0), vi.fn(), true);
    expect(screen.getByText('Not started')).toHaveAttribute(
      'data-tooltip',
      'Sessions start after you send your first message.',
    );
    expect(container.querySelector('.pace-warning')).not.toBeInTheDocument();
    expect(container.querySelector('.meter-shell')).not.toHaveAttribute('data-tooltip');
  });

  it('does not decorate unused non-session quotas as healthy pacing', () => {
    const { container } = showAlways(quota(0));
    expect(screen.queryByText(/left at reset/)).not.toBeInTheDocument();
    expect(container.querySelector('.meter__pace')).not.toBeInTheDocument();
    expect(container.querySelector('.meter-shell')).not.toHaveAttribute('data-tooltip');
  });

  it('renders provider-supplied count units instead of a hardcoded request label', () => {
    show({
      ...quota(24),
      id: 'requests',
      label: 'Requests',
      format: 'count',
      usedValue: 120,
      limitValue: 500,
      remainingValue: 380,
      unit: 'searches',
    });
    expect(screen.getByRole('button', { name: '380 searches left' })).toHaveAttribute(
      'data-tooltip',
      '120 searches used',
    );
  });

  it('keeps a missing count unit unknown', () => {
    show({
      ...quota(24),
      format: 'count',
      usedValue: 120,
      limitValue: 500,
      remainingValue: 380,
      unit: null,
    });
    expect(screen.getByRole('button', { name: '380 (unit unknown) left' })).toBeInTheDocument();
    expect(screen.queryByText(/requests left/)).not.toBeInTheDocument();
  });

  it.each([50, 0, 0.4, 0.123456789])(
    'shows source remaining %s, not a local difference',
    (remainingValue) => {
      show({
        ...quota(40),
        format: 'count',
        usedValue: 40,
        limitValue: 100,
        remainingValue,
        unit: 'credits',
      });
      expect(
        screen.getByRole('button', { name: `${remainingValue} credits left` }),
      ).toHaveAttribute('data-tooltip', '40 credits used');
      expect(screen.queryByRole('button', { name: '60 credits left' })).not.toBeInTheDocument();
    },
  );

  it('keeps remaining-only values and falls back to percent for missing absolute remaining', () => {
    const first = show({
      ...quota(40),
      format: 'count',
      usedValue: null,
      limitValue: null,
      remainingValue: 50,
      unit: 'credits',
    });
    expect(screen.getByRole('button', { name: '50 credits left' })).toHaveAttribute(
      'data-tooltip',
      '40% used',
    );
    first.unmount();
    show({
      ...quota(40),
      format: 'count',
      usedValue: 40,
      limitValue: 100,
      remainingValue: null,
      unit: 'credits',
    });
    expect(screen.getByRole('button', { name: '60% left' })).toBeInTheDocument();
    expect(screen.queryByText('60 credits left')).not.toBeInTheDocument();
  });

  it.each([48, 0])('only marks actual zero remaining as spent (%s)', (remainingValue) => {
    show({
      ...quota(99.6),
      format: 'count',
      usedValue: 11952,
      limitValue: 12000,
      remainingValue,
      unit: 'credits',
    });
    expect(
      screen.getByRole('button', { name: `${remainingValue} credits left` }),
    ).toBeInTheDocument();
    expect(screen.queryByText('Limit reached') !== null).toBe(remainingValue === 0);
  });

  it('marks inferred quotas with their source note', () => {
    show({
      ...quota(24),
      estimated: true,
      sourceNote: 'Estimated from provider records.',
    });

    expect(screen.getByLabelText('Estimated quota')).toHaveAttribute(
      'data-tooltip',
      'Estimated from provider records.',
    );
  });
});
