import { describe, expect, it } from 'vitest';
import { catalogKeys, catalogs } from './catalog';
import { lookup, lookupWindowLabel } from './lookup';

describe('i18n catalogs', () => {
  it('keeps chinese keys identical to english', () => {
    expect(catalogKeys('zh')).toEqual(catalogKeys('en'));
  });

  it('falls back to the english phrase then the key', () => {
    expect(lookup('zh', 'settings.title')).toBe('设置');
    expect(lookup('zh', 'missing.phrase')).toBe('missing.phrase');
    expect(Object.values(catalogs.zh).every((value) => value.length > 0)).toBe(true);
  });

  it('translates known window ids and keeps unknown labels', () => {
    expect(lookupWindowLabel('zh', 'session', 'Session')).toBe('5 小时窗口');
    expect(lookupWindowLabel('zh', 'weekly', 'Weekly')).toBe('每周');
    expect(lookupWindowLabel('zh', 'scoped-opus-5-5', 'Opus 5.5')).toBe('Opus 5.5');
  });
});
