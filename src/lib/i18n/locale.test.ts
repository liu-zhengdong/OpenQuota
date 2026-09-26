import { describe, expect, it } from 'vitest';
import { detectSystemLanguageTag, isChineseLanguageTag, resolveUiLocale } from './locale';

describe('ui locale resolution', () => {
  it('treats zh-prefixed system tags as chinese when following the system', () => {
    expect(resolveUiLocale('system', 'zh-CN')).toBe('zh');
    expect(resolveUiLocale('system', 'zh')).toBe('zh');
    expect(resolveUiLocale('system', 'zh-Hans-CN')).toBe('zh');
    expect(resolveUiLocale('system', 'en-US')).toBe('en');
    expect(resolveUiLocale('system', 'fr-FR')).toBe('en');
  });

  it('honors an explicit language preference over the system tag', () => {
    expect(resolveUiLocale('en', 'zh-CN')).toBe('en');
    expect(resolveUiLocale('zh', 'en-US')).toBe('zh');
  });

  it('detects chinese tags with underscores', () => {
    expect(isChineseLanguageTag('zh_CN')).toBe(true);
    expect(isChineseLanguageTag('ZH-TW')).toBe(true);
    expect(isChineseLanguageTag('en-GB')).toBe(false);
    expect(detectSystemLanguageTag('zh-CN', [])).toBe('zh-CN');
    expect(detectSystemLanguageTag('', ['en-US', 'zh-CN'])).toBe('en-US');
  });
});
