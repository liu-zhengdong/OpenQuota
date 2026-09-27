import { afterEach, describe, expect, it } from 'vitest';
import { language } from './store.svelte';
import { warningText, warningTexts } from './providerMessages';
import type { ProviderWarning } from '../types';

const warning = (id: string, params: ProviderWarning['params'] = {}): ProviderWarning => ({
  id,
  params,
  fallback: 'fallback copy',
});

afterEach(() => {
  language.resetForTests();
});

describe('provider warnings', () => {
  it('translates every catalogued warning into chinese', () => {
    language.resetForTests('zh');

    expect(warningText(warning('claude.reLoginRequired'))).toBe(
      '实时用量需要重新登录。在终端运行 `claude` 并重新登录，以恢复订阅限额。',
    );
    expect(warningText(warning('claude.rateLimitedStale'))).toBe(
      'Claude 实时用量接口限流，当前显示上次成功的限额。',
    );
    expect(warningText(warning('claude.credentialNotSaved'))).toBe(
      '刷新后的 Claude 登录本次会话可用，但未能保存。',
    );
    expect(warningText(warning('codex.credentialNotSaved'))).toBe(
      '刷新后的 Codex 登录本次会话可用，但未能保存。',
    );
    expect(warningText(warning('grok.credentialNotSaved'))).toBe(
      '刷新后的 Grok 登录本次会话可用，但未能保存。',
    );
  });

  it('interpolates the retry duration in the active language', () => {
    const rateLimitedRetry = warning('claude.rateLimitedRetry', { retrySeconds: '120' });

    language.resetForTests('zh');
    expect(warningText(rateLimitedRetry)).toBe('Claude 实时用量接口限流，约 2 分钟后重试。');

    language.resetForTests('en');
    expect(warningText(rateLimitedRetry)).toBe(
      'Claude live usage is rate limited; retrying in about 2 minutes.',
    );
  });

  it('keeps the provider fallback for ids and legacy strings with no catalog entry', () => {
    language.resetForTests('zh');

    expect(warningText(warning('someProvider.somethingNew'))).toBe('fallback copy');
    // Snapshots cached before warnings carried an id arrive as a bare id with the text inline.
    expect(warningText(warning(''))).toBe('fallback copy');
    expect(
      warningTexts([warning('claude.rateLimitedStale'), warning('someProvider.somethingNew')]),
    ).toEqual(['Claude 实时用量接口限流，当前显示上次成功的限额。', 'fallback copy']);
  });
});
