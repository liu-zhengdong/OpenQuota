export interface QuotaWindow {
  id: string;
  label: string;
  usedPercent: number;
  resetsAt: string | null;
  periodSeconds: number;
  format: 'percent' | 'dollars' | 'count';
  usedValue: number | null;
  limitValue: number | null;
  remainingValue?: number | null;
  unit?: string | null;
  estimated: boolean;
  sourceNote?: string | null;
}

export interface MetricValue {
  number: number;
  kind: 'count' | 'dollars';
  label?: string | null;
  estimated: boolean;
}

export interface ValueMetric {
  id: string;
  label: string;
  values: MetricValue[];
  expiriesAt: string[];
}

export interface StatusMetric {
  id: string;
  label: string;
  text: string;
  tone: 'neutral' | 'positive' | 'warning' | 'danger';
  subtitle?: string | null;
}

export type ResetClaimOutcome = 'success' | 'nothingToReset' | 'noCredit' | 'failed';

/**
 * Locale-independent values the provider filled in. The interface renders its own language
 * from these plus the notice id, so the copy follows the language instead of the write time.
 */
export type ProviderMessageParams = Record<string, string>;

export interface ProviderNotice {
  /** Catalog id under `notices.`, namespaced by provider, e.g. `claude.rateLimited`. */
  id: string;
  /** English copy used when the interface language has no catalog entry. */
  title: string;
  /** English copy used when the interface language has no catalog entry. */
  message: string;
  params?: ProviderMessageParams;
  tone: 'info' | 'warning';
}

export interface ProviderWarning {
  /** Catalog id under `warnings.`, empty for text with no catalog entry. */
  id: string;
  params?: ProviderMessageParams;
  /** English copy used when the interface language has no catalog entry. */
  fallback: string;
}

export interface AccountIdentity {
  kind: 'accountHash';
  value: string;
  source: string;
}

export interface SharedScope {
  id: string;
  source: string;
  windowIds: string[];
}

export interface ProviderSnapshot {
  providerId: string;
  plan: string | null;
  quotas: QuotaWindow[];
  valueMetrics: ValueMetric[];
  statusMetrics: StatusMetric[];
  notices: ProviderNotice[];
  warnings: ProviderWarning[];
  refreshedAt: string;
  /** No live read produced these numbers, so they must not be presented as current. */
  remembered?: boolean;
  accountIdentity?: AccountIdentity | null;
  sharedScope?: SharedScope | null;
}

export type ProviderErrorKind =
  | 'authentication'
  | 'permission'
  | 'rateLimited'
  | 'network'
  | 'invalidResponse'
  | 'credentialStorage'
  | 'storage'
  | 'internal';

export interface ProviderViewState {
  snapshot: ProviderSnapshot | null;
  source: 'none' | 'cache' | 'live';
  refreshing: boolean;
  stale: boolean;
  error: string | null;
  errorKind: ProviderErrorKind | null;
  lastAttemptAt: string | null;
  cacheIdentityMatch?: 'matched' | 'mismatched' | 'unknown';
}

export interface UsageViewState {
  providers: Record<string, ProviderViewState>;
  lastFullRefreshAt?: string | null;
}

export type MetricSection = 'alwaysVisible' | 'onDemand';

export type MetricSource =
  | { kind: 'quota'; sourceId: string; sessionWindow: boolean }
  | { kind: 'quotaOrValue'; sourceId: string; sessionWindow: boolean }
  | { kind: 'value'; sourceId: string }
  | { kind: 'status'; sourceId: string };

export interface TrayMetricDefinition {
  shortLabel: string;
  suffix: string | null;
}

export interface MetricDefinition {
  hideWhenMissing?: boolean;
  id: string;
  label: string;
  source: MetricSource;
  pinnable: boolean;
  defaultEnabled: boolean;
  defaultSection: MetricSection;
  defaultPinned: boolean;
  tray: TrayMetricDefinition | null;
}

export interface ProviderLink {
  label: string;
  url: string;
}

export type ApiKeyStatus = 'notSet' | 'fromEnvironment' | 'fromConfig' | 'saved' | 'overrideActive';

export interface ProviderApiKeyState {
  providerId: string;
  status: ApiKeyStatus;
}

export interface ApiKeyMutationOutcome extends ProviderApiKeyState {
  warning?: string;
}

export interface ProviderDefinition {
  id: string;
  displayName: string;
  shortName: string;
  fallbackEnabled: boolean;
  scopedQuotaPrefix?: string | null;
  links: ProviderLink[];
  metrics: MetricDefinition[];
}

export interface ProviderCatalog {
  providers: ProviderDefinition[];
  apiKeyProviderIds?: string[];
}

export interface MetricLayout {
  id: string;
  enabled: boolean;
  section: MetricSection;
  pinned: boolean;
}

export interface ProviderLayout {
  id: string;
  enabled: boolean;
  detected: boolean;
  expanded: boolean;
  metrics: MetricLayout[];
}

export interface NotificationPreferences {
  almostOut: boolean;
  cuttingItClose: boolean;
  willRunOut: boolean;
}

export interface AppSettings {
  schemaVersion: number;
  providers: ProviderLayout[];
  knownProviderIds: string[];
  providerNames: Record<string, string>;
  theme: 'system' | 'light' | 'dark';
  density: 'default' | 'compact';
  reduceAnimations: boolean;
  windowMode: 'popup' | 'floating';
  menuBarStyle: 'text' | 'bars' | 'compact';
  usageDisplay: 'used' | 'left';
  resetDisplay: 'countdown' | 'exact';
  timeFormat: 'system' | 'twelveHour' | 'twentyFourHour';
  uiLanguage: 'system' | 'en' | 'zh';
  providerSort: 'custom' | 'spare';
  alwaysShowPacing: boolean;
  launchAtLogin: boolean;
  autoCheckUpdates: boolean;
  dismissedUpdateVersion: string | null;
  lastUpdateCheckAt: string | null;
  globalShortcut: string | null;
  logLevel: 'error' | 'warn' | 'info' | 'debug';
  notifications: NotificationPreferences;
  detectionNoticeDismissed: boolean;
}

export interface UpdateStatus {
  available: boolean;
  currentVersion: string;
  version: string | null;
  body: string | null;
  installable: boolean;
  releaseUrl: string;
}

export interface UpdateProgress {
  phase: 'downloading' | 'retrying' | 'installing';
  downloaded: number;
  total: number | null;
  percent: number | null;
}

export interface UpdateFailure {
  code: string;
  message: string;
  action: string;
  retryable: boolean;
}

export interface SettingsViewState {
  settings: AppSettings;
  settingsRevision: number;
  accountRevision: number;
  renamableProviderIds: string[];
  notificationPermission: 'granted' | 'denied' | 'prompt' | 'unavailable';
  integrationError: string | null;
  trayAvailable: boolean;
  platformSummary: string | null;
}

export interface BootstrapState {
  usage: UsageViewState;
  settings: SettingsViewState;
  catalog: ProviderCatalog;
}
