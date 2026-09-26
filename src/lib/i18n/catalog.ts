import { apiKey as enApiKey } from './en/apiKey';
import { chrome as enChrome } from './en/chrome';
import { common as enCommon } from './en/common';
import { customize as enCustomize } from './en/customize';
import { dashboard as enDashboard } from './en/dashboard';
import { errors as enErrors } from './en/errors';
import { metrics as enMetrics } from './en/metrics';
import { notifications as enNotifications } from './en/notifications';
import { settings as enSettings } from './en/settings';
import { tray as enTray } from './en/tray';
import { windows as enWindows } from './en/windows';
import type { MessageTable, UiLocale } from './types';
import { apiKey as zhApiKey } from './zh/apiKey';
import { chrome as zhChrome } from './zh/chrome';
import { common as zhCommon } from './zh/common';
import { customize as zhCustomize } from './zh/customize';
import { dashboard as zhDashboard } from './zh/dashboard';
import { errors as zhErrors } from './zh/errors';
import { metrics as zhMetrics } from './zh/metrics';
import { notifications as zhNotifications } from './zh/notifications';
import { settings as zhSettings } from './zh/settings';
import { tray as zhTray } from './zh/tray';
import { windows as zhWindows } from './zh/windows';

function flatten(prefix: string, messages: Record<string, string>): MessageTable {
  const table: MessageTable = {};
  for (const [key, value] of Object.entries(messages)) {
    table[`${prefix}.${key}`] = value;
  }
  return table;
}

function mergeLocale(parts: Record<string, Record<string, string>>): MessageTable {
  return Object.assign(
    {},
    ...Object.entries(parts).map(([prefix, messages]) => flatten(prefix, messages)),
  ) as MessageTable;
}

export const catalogs: Record<UiLocale, MessageTable> = {
  en: mergeLocale({
    common: enCommon,
    dashboard: enDashboard,
    settings: enSettings,
    customize: enCustomize,
    chrome: enChrome,
    metrics: enMetrics,
    windows: enWindows,
    tray: enTray,
    notifications: enNotifications,
    errors: enErrors,
    apiKey: enApiKey,
  }),
  zh: mergeLocale({
    common: zhCommon,
    dashboard: zhDashboard,
    settings: zhSettings,
    customize: zhCustomize,
    chrome: zhChrome,
    metrics: zhMetrics,
    windows: zhWindows,
    tray: zhTray,
    notifications: zhNotifications,
    errors: zhErrors,
    apiKey: zhApiKey,
  }),
};

export function catalogKeys(locale: UiLocale): string[] {
  return Object.keys(catalogs[locale]).sort();
}
