<script lang="ts">
  import Icon from './Icon.svelte';
  import { t, windowLabel } from './i18n';
  import { quotaFillPercent, quotaReading } from './quotaReading';
  import {
    formatLimit,
    formatReset,
    isFreshSessionWindow,
    paceTooltip,
    projectPace,
  } from './pacing';
  import type { QuotaWindow } from './types';

  interface Props {
    quota: QuotaWindow;
    now: number;
    usageDisplay: 'used' | 'left';
    resetDisplay: 'countdown' | 'exact';
    timeFormat: 'system' | 'twelveHour' | 'twentyFourHour';
    alwaysShowPacing: boolean;
    isSessionWindow?: boolean;
    onToggleUsage: () => void;
    onToggleReset: () => void;
  }

  let {
    quota,
    now,
    usageDisplay,
    resetDisplay,
    timeFormat,
    alwaysShowPacing,
    isSessionWindow = false,
    onToggleUsage,
    onToggleReset,
  }: Props = $props();
  const used = $derived(Math.min(100, Math.max(0, quota.usedPercent)));
  const title = $derived(windowLabel(quota.id, quota.label));
  const estimateNote = $derived(quota.sourceNote?.trim() || t('metrics.estimatedNote'));
  const reading = $derived(quotaReading(quota, usageDisplay));
  const readingTooltip = $derived(
    quotaReading(quota, usageDisplay === 'left' ? 'used' : 'left', true),
  );
  const fillPercent = $derived(quotaFillPercent(quota, usageDisplay));
  const freshSession = $derived(isFreshSessionWindow(quota, now, isSessionWindow));
  const pace = $derived(projectPace(quota, now));
  const paceDetail = $derived(paceTooltip(pace));
  const roundedUsed = $derived(Math.round(used));
  const severity = $derived(
    pace.severity === 'level'
      ? roundedUsed >= 90
        ? 'critical'
        : roundedUsed >= 80
          ? 'warning'
          : 'normal'
      : pace.severity === 'healthy'
        ? 'normal'
        : pace.severity === 'close'
          ? 'warning'
          : 'critical',
  );
  const showPace = $derived(
    pace.severity === 'close' ||
      pace.severity === 'runningOut' ||
      pace.severity === 'spent' ||
      (alwaysShowPacing && pace.severity === 'healthy'),
  );
  const paceLabel = $derived.by(() => {
    if (pace.severity === 'spent') return t('metrics.limitReached');
    if (pace.severity === 'runningOut')
      return pace.runOutAt === null
        ? null
        : formatLimit(pace.runOutAt, now, resetDisplay, timeFormat);
    if (pace.projectedUsedPercent === null) return null;
    const left = Math.max(0, 100 - pace.projectedUsedPercent);
    return pace.severity === 'close'
      ? t('metrics.sparePercent', { percent: Math.max(1, Math.round(left)) })
      : paceDetail;
  });
  const paceTickPercent = $derived(
    pace.evenPacePercent === null
      ? null
      : usageDisplay === 'left'
        ? 100 - pace.evenPacePercent
        : pace.evenPacePercent,
  );
  const resetTooltip = $derived(
    quota.resetsAt
      ? formatReset(
          quota.resetsAt,
          now,
          resetDisplay === 'countdown' ? 'exact' : 'countdown',
          timeFormat,
        )
      : null,
  );
</script>

<section class="metric" aria-label={t('metrics.quotaAria', { label: title })}>
  <div class="metric__heading">
    <h2>
      {title}
      {#if quota.estimated}
        <span
          class="metric-estimate"
          data-tooltip={estimateNote}
          aria-label={t('metrics.estimatedQuota')}
          role="img"><Icon name="about" size={11} strokeWidth={1.9} /></span
        >
      {/if}
    </h2>
    {#if showPace}
      {#if pace.severity === 'spent' || pace.severity === 'runningOut'}
        {#if pace.severity === 'runningOut' && paceLabel}
          <button
            type="button"
            class="pace-warning"
            data-tooltip={paceDetail ?? undefined}
            aria-label={paceLabel}
            onclick={onToggleReset}
            ><span class="pace-warning__icon"
              ><Icon name="flame-filled" size={11} strokeWidth={1.8} /></span
            >{paceLabel}</button
          >
        {:else}
          <span
            class="pace-warning"
            data-tooltip={paceDetail ?? undefined}
            aria-label={pace.severity === 'spent'
              ? t('metrics.limitReached')
              : t('metrics.willReachLimit')}
            ><span class="pace-warning__icon"
              ><Icon name="flame-filled" size={11} strokeWidth={1.8} /></span
            >{paceLabel ?? ''}</span
          >
        {/if}
      {:else if paceLabel}
        <span data-tooltip={pace.severity === 'close' ? (paceDetail ?? undefined) : undefined}
          >{paceLabel}</span
        >
      {/if}
    {/if}
  </div>

  <div class="meter-shell" data-tooltip={paceDetail ?? undefined}>
    <div
      class="meter meter--{severity}"
      role="progressbar"
      aria-label={t('metrics.usedAria', { label: title })}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={used}
    >
      <span
        class="meter__fill"
        class:meter__fill--visible={fillPercent > 0}
        style={`--fill-percent: ${fillPercent}%`}
      ></span>
    </div>
    {#if showPace && paceTickPercent !== null}
      <span
        class="meter__pace"
        style={`--pace-percent: ${Math.min(100, Math.max(0, paceTickPercent))}%`}
        aria-hidden="true"
      ></span>
    {/if}
  </div>

  <div class="metric__reading">
    <button type="button" data-tooltip={readingTooltip ?? undefined} onclick={onToggleUsage}>
      {reading}
    </button>
    {#if freshSession}
      <span data-tooltip={t('metrics.sessionNotStarted')}>{t('metrics.notStarted')}</span>
    {:else}
      <button type="button" data-tooltip={resetTooltip ?? undefined} onclick={onToggleReset}>
        {formatReset(quota.resetsAt, now, resetDisplay, timeFormat)}
      </button>
    {/if}
  </div>
</section>

<style>
  :global {
    .metric__heading .pace-warning {
      display: inline-flex;
      align-items: center;
      gap: 3px;
      margin: 0;
      padding: 0;
      border: 0;
      color: var(--secondary);
      background: transparent;
      font: inherit;
      font-size: 13px;
      line-height: 17px;
    }

    .metric__heading .metric-estimate {
      display: inline-flex;
      margin-left: 3px;
      color: var(--secondary);
      vertical-align: -1px;
    }

    .metric__heading .pace-warning__icon {
      color: var(--meter-critical);
    }

    .meter__fill {
      position: absolute;
      inset: 0 auto 0 0;
      width: 0;
      border-radius: inherit;
      background: var(--meter-fill);
      transition:
        width var(--motion-switch),
        background-color var(--motion-switch);
    }

    .meter__fill--visible {
      width: max(5px, var(--fill-percent));
    }

    .meter--warning .meter__fill {
      background: var(--meter-warning);
    }

    .meter--critical .meter__fill {
      background: var(--meter-critical);
    }

    .meter__pace {
      position: absolute;
      top: -2px;
      left: clamp(1px, var(--pace-percent), calc(100% - 1px));
      width: 2px;
      height: 9px;
      border-radius: 2px;
      background: var(--secondary);
      transform: translateX(-1px);
    }

    .metric__reading strong {
      color: var(--text);
      font-weight: 450;
    }

    .metric__reading button {
      padding: 0;
      border: 0;
      border-radius: 6px;
      color: inherit;
      background: none;
      cursor: pointer;
      transition:
        color 120ms ease,
        background-color 120ms ease;
    }

    .metric__reading button:first-child {
      color: var(--text);
      font-weight: 500;
    }

    .metric__reading button:hover,
    .metric__reading button:focus-visible {
      background: var(--button-hover);
      box-shadow: 0 0 0 3px var(--button-hover);
    }
  }
</style>
