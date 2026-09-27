import { parse } from 'svelte/compiler';
import { describe, expect, it } from 'vitest';

// Prettier splits closing tags as `</span\n>`; a duplicated `>` in that layout
// compiles to a visible text node instead of failing, so guard against it here.
const sources = import.meta.glob<string>('../**/*.svelte', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function strayTextNodes(file: string, source: string): string[] {
  const found: string[] = [];
  const walk = (node: unknown): void => {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) return node.forEach(walk);
    const current = node as { type?: string; data?: string; start?: number };
    if (current.type === 'Text' && /^\s*>+\s*$/.test(current.data ?? '')) {
      const line = source.slice(0, current.start).split('\n').length;
      found.push(`${file}:${line}`);
    }
    for (const [key, value] of Object.entries(node)) if (key !== 'parent') walk(value);
  };
  walk(parse(source, { modern: true }).fragment);
  return found;
}

describe('svelte markup', () => {
  it('renders no stray ">" text left over from split closing tags', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(10);
    expect(
      Object.entries(sources).flatMap(([file, source]) => strayTextNodes(file, source)),
    ).toEqual([]);
  });
});
