import type { MessageVars } from './types';

const TOKEN = /\{([a-zA-Z0-9_]+)\}/g;

export function interpolate(template: string, vars?: MessageVars): string {
  if (!vars) return template;
  return template.replace(TOKEN, (match, name: string) => {
    const value = vars[name];
    return value === undefined ? match : String(value);
  });
}
