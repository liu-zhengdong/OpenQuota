<script lang="ts">
  import type { PanelHeightMode } from './backend';
  import Icon from './Icon.svelte';
  import { applyLanguagePreference, notifyBackendLocale } from './i18n/apply';
  import { language, t } from './i18n';
  import type { DesktopPlatform } from './platform';
  import SelectMenu from './SelectMenu.svelte';
  import type {
    AppSettings,
    NotificationPreferences,
    SettingsViewState,
    UpdateFailure,
  } from './types';

  interface Props {
    settingsView: SettingsViewState;
    platform: DesktopPlatform;
    panelHeightMode: PanelHeightMode;
    onChange: (settings: AppSettings) => void;
    onPanelHeightModeChange: (mode: PanelHeightMode) => void;
    onRequestNotifications: () => void;
    onOpenNotificationSettings: () => void;
    updateError: UpdateFailure | null;
    checkingUpdate: boolean;
    onCheckForUpdates: () => void;
    onCustomize: () => void;
    onCopyLogPath: () => Promise<void>;
    onOpenLogFolder: () => Promise<void>;
    onResetAllSettings: () => void;
  }
  let {
    settingsView,
    platform,
    panelHeightMode,
    onChange,
    onPanelHeightModeChange,
    onRequestNotifications,
    onOpenNotificationSettings,
    updateError,
    checkingUpdate,
    onCheckForUpdates,
    onCustomize,
    onCopyLogPath,
    onOpenLogFolder,
    onResetAllSettings,
  }: Props = $props();
  let recording = $state(false);
  let logActionError = $state<string | null>(null);
  const settings = $derived(settingsView.settings);
  const revealLogLabel = $derived(
    platform === 'macos'
      ? t('settings.revealInFinder')
      : platform === 'windows'
        ? t('settings.revealInExplorer')
        : t('settings.openContainingFolder'),
  );
  const anyNotificationEnabled = $derived(
    settings.notifications.almostOut ||
      settings.notifications.cuttingItClose ||
      settings.notifications.willRunOut,
  );
  const notificationsNeedAttention = $derived(
    anyNotificationEnabled && settingsView.notificationPermission !== 'granted',
  );

  function patch(value: Partial<AppSettings>) {
    onChange({ ...settings, ...value });
  }
  function patchLanguage(value: AppSettings['uiLanguage']) {
    applyLanguagePreference(value);
    notifyBackendLocale(language.locale);
    patch({ uiLanguage: value });
  }
  function patchNotification(key: keyof NotificationPreferences, enabled: boolean) {
    patch({ notifications: { ...settings.notifications, [key]: enabled } });
    if (enabled && settingsView.notificationPermission === 'prompt') onRequestNotifications();
  }
  async function copyLogPath() {
    try {
      await onCopyLogPath();
      logActionError = null;
    } catch {
      logActionError = t('settings.copyLogFailed');
    }
  }
  async function revealLogFile() {
    try {
      await onOpenLogFolder();
      logActionError = null;
    } catch {
      logActionError = t('settings.revealLogFailed');
    }
  }
  function record(event: KeyboardEvent) {
    if (!recording) return;
    if (event.key === 'Tab') {
      recording = false;
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape') {
      recording = false;
      return;
    }
    if (event.key === 'Delete' || event.key === 'Backspace') {
      patch({ globalShortcut: null });
      recording = false;
      return;
    }
    if (
      !(event.ctrlKey || event.altKey || event.metaKey) ||
      ['Control', 'Alt', 'Meta', 'Shift'].includes(event.key)
    )
      return;
    const modifiers = [
      event.ctrlKey && 'Ctrl',
      event.altKey && 'Alt',
      event.shiftKey && 'Shift',
      event.metaKey && 'Super',
    ].filter(Boolean);
    const key = event.code.startsWith('Key')
      ? event.code.slice(3)
      : event.code.startsWith('Digit')
        ? event.code.slice(5)
        : event.key.length === 1
          ? event.key.toUpperCase()
          : event.key;
    patch({ globalShortcut: [...modifiers, key].join('+') });
    recording = false;
  }
</script>

<section class="screen settings-screen" aria-label={t('settings.title')}>
  {#if settingsView.integrationError}<p class="notice" role="alert">
      {settingsView.integrationError}
    </p>{/if}

  {#if settingsView.platformSummary}<div class="settings-section">
      <h2>{t('settings.linux')}</h2>
      <div class="setting-row">
        <span
          ><b>{t('settings.desktopIntegration')}</b><small>{settingsView.platformSummary}</small
          ></span
        >
      </div>
    </div>{/if}

  <div class="settings-section">
    <h2>{t('settings.general')}</h2>
    <div class="setting-row">
      <span><b>{t('settings.language')}</b></span><SelectMenu
        label={t('settings.language')}
        value={settings.uiLanguage}
        options={[
          { value: 'system', label: t('settings.languageFollowSystem') },
          { value: 'en', label: t('settings.languageEnglish') },
          { value: 'zh', label: t('settings.languageChinese') },
        ]}
        onChange={(value) => patchLanguage(value as AppSettings['uiLanguage'])}
      />
    </div>
    <label class="setting-row"
      ><span><b>{t('settings.launchAtLogin')}</b></span><input
        type="checkbox"
        checked={settings.launchAtLogin}
        onchange={(event) => patch({ launchAtLogin: event.currentTarget.checked })}
      /></label
    >
    <div class="setting-row">
      <span><b>{t('settings.globalShortcut')}</b></span>
      <div class="shortcut-field">
        <button
          class:recording
          type="button"
          aria-pressed={recording}
          aria-describedby="shortcut-recording-help"
          data-tooltip={t('settings.shortcutTooltip')}
          onclick={(e) => {
            // WKWebView 点击 <button> 不会给它焦点,必须手动 focus,
            // 否则后续 keydown 到不了按钮,快捷键录不上。
            recording = !recording;
            if (recording) e.currentTarget.focus();
          }}
          onkeydown={record}
          onblur={() => (recording = false)}
          >{recording
            ? t('settings.typeShortcut')
            : (settings.globalShortcut ?? t('settings.recordShortcut'))}</button
        >{#if settings.globalShortcut}<button
            class="shortcut-clear"
            type="button"
            aria-label={t('settings.clearShortcut')}
            onclick={() => patch({ globalShortcut: null })}
            ><Icon name="close" size={10} strokeWidth={2.2} /></button
          >{/if}
      </div>
      <small id="shortcut-recording-help" class="sr-only">{t('settings.shortcutHelp')}</small>
    </div>
  </div>

  <div class="settings-section">
    <h2>{t('settings.appearance')}</h2>
    {#if platform === 'macos'}
      <div class="setting-row">
        <span><b>{t('settings.iconStyle')}</b></span><SelectMenu
          label={t('settings.iconStyle')}
          value={settings.menuBarStyle}
          options={[
            { value: 'text', label: t('settings.iconText') },
            { value: 'bars', label: t('settings.iconBars') },
          ]}
          onChange={(value) => patch({ menuBarStyle: value as AppSettings['menuBarStyle'] })}
        />
      </div>
    {/if}
    <div class="setting-row">
      <span><b>{t('settings.theme')}</b></span><SelectMenu
        label={t('settings.theme')}
        value={settings.theme}
        options={[
          { value: 'system', label: t('settings.themeSystem') },
          { value: 'light', label: t('settings.themeLight') },
          { value: 'dark', label: t('settings.themeDark') },
        ]}
        onChange={(value) => patch({ theme: value as AppSettings['theme'] })}
      />
    </div>
    <div class="setting-row">
      <span><b>{t('settings.density')}</b></span><SelectMenu
        label={t('settings.density')}
        value={settings.density}
        options={[
          { value: 'default', label: t('settings.densityDefault') },
          { value: 'compact', label: t('settings.densityCompact') },
        ]}
        onChange={(value) => patch({ density: value as AppSettings['density'] })}
      />
    </div>
    <label class="setting-row"
      ><span><b>{t('settings.reduceAnimations')}</b></span><input
        type="checkbox"
        checked={settings.reduceAnimations}
        onchange={(event) => patch({ reduceAnimations: event.currentTarget.checked })}
      /></label
    >
    {#if settingsView.trayAvailable}
      <div class="setting-row">
        <span><b>{t('settings.windowMode')}</b></span><SelectMenu
          label={t('settings.windowMode')}
          value={settings.windowMode}
          options={[
            { value: 'popup', label: t('settings.windowTrayPopup') },
            { value: 'floating', label: t('settings.windowFloating') },
          ]}
          onChange={(value) => patch({ windowMode: value as AppSettings['windowMode'] })}
        />
      </div>
    {/if}
    <div class="setting-row">
      <span><b>{t('settings.panelHeight')}</b></span><SelectMenu
        label={t('settings.panelHeight')}
        value={panelHeightMode}
        options={[
          { value: 'automatic', label: t('settings.panelAutomatic') },
          { value: 'manual', label: t('settings.panelManual') },
        ]}
        onChange={(value) => onPanelHeightModeChange(value as PanelHeightMode)}
      />
    </div>
    <div class="setting-row">
      <span><b>{t('settings.timeFormat')}</b></span><SelectMenu
        label={t('settings.timeFormat')}
        value={settings.timeFormat}
        options={[
          { value: 'system', label: t('settings.timeFormatAuto') },
          { value: 'twelveHour', label: t('settings.timeFormat12') },
          { value: 'twentyFourHour', label: t('settings.timeFormat24') },
        ]}
        onChange={(value) => patch({ timeFormat: value as AppSettings['timeFormat'] })}
      />
    </div>
  </div>

  <div class="settings-section">
    <h2>{t('settings.usageDisplay')}</h2>
    <div class="setting-row">
      <span><b>{t('settings.showUsageAs')}</b></span><SelectMenu
        label={t('settings.showUsageAs')}
        value={settings.usageDisplay}
        options={[
          { value: 'left', label: t('settings.usageLeft') },
          { value: 'used', label: t('settings.usageUsed') },
        ]}
        onChange={(value) => patch({ usageDisplay: value as AppSettings['usageDisplay'] })}
      />
    </div>
    <div class="setting-row">
      <span><b>{t('settings.resetTimes')}</b></span><SelectMenu
        label={t('settings.resetTimes')}
        value={settings.resetDisplay}
        options={[
          { value: 'countdown', label: t('settings.resetCountdown') },
          { value: 'exact', label: t('settings.resetExact') },
        ]}
        onChange={(value) => patch({ resetDisplay: value as AppSettings['resetDisplay'] })}
      />
    </div>
    <label class="setting-row"
      ><span
        ><b>{t('settings.alwaysShowPacing')}</b><i
          class="setting-info"
          data-tooltip={t('settings.alwaysShowPacingHint')}
          aria-label={t('settings.alwaysShowPacingHint')}
          ><Icon name="about" size={12} strokeWidth={1.8} /></i
        ></span
      ><input
        type="checkbox"
        checked={settings.alwaysShowPacing}
        onchange={(event) => patch({ alwaysShowPacing: event.currentTarget.checked })}
      /></label
    >
  </div>

  <div class="settings-section">
    <h2>
      {t('settings.notifications')}
      {#if notificationsNeedAttention}<span class="permission-warning">!</span>{/if}
    </h2>
    <label class="setting-row"
      ><span
        ><b>{t('settings.almostOut')}</b><i
          class="setting-info"
          data-tooltip={t('settings.almostOutHint')}
          aria-label={t('settings.almostOutHint')}
          ><Icon name="about" size={12} strokeWidth={1.8} /></i
        ></span
      ><input
        type="checkbox"
        checked={settings.notifications.almostOut}
        onchange={(event) => patchNotification('almostOut', event.currentTarget.checked)}
      /></label
    >
    <label class="setting-row"
      ><span
        ><b>{t('settings.cuttingItClose')}</b><i
          class="setting-info"
          data-tooltip={t('settings.cuttingItCloseHint')}
          aria-label={t('settings.cuttingItCloseHint')}
          ><Icon name="about" size={12} strokeWidth={1.8} /></i
        ></span
      ><input
        type="checkbox"
        checked={settings.notifications.cuttingItClose}
        onchange={(event) => patchNotification('cuttingItClose', event.currentTarget.checked)}
      /></label
    >
    <label class="setting-row"
      ><span
        ><b>{t('settings.willRunOut')}</b><i
          class="setting-info"
          data-tooltip={t('settings.willRunOutHint')}
          aria-label={t('settings.willRunOutHint')}
          ><Icon name="about" size={12} strokeWidth={1.8} /></i
        ></span
      ><input
        type="checkbox"
        checked={settings.notifications.willRunOut}
        onchange={(event) => patchNotification('willRunOut', event.currentTarget.checked)}
      /></label
    >
    {#if notificationsNeedAttention}
      <div class="notification-actions">
        <div class="notification-attention" role="status">
          <span
            ><b
              >{settingsView.notificationPermission === 'denied'
                ? t('settings.notificationsBlocked')
                : t('settings.permissionRequired')}</b
            ><small
              >{settingsView.notificationPermission === 'denied'
                ? t('settings.notificationsBlockedHint')
                : t('settings.permissionRequiredHint')}</small
            ></span
          >
          <button
            class="secondary-button"
            type="button"
            onclick={settingsView.notificationPermission === 'denied'
              ? onOpenNotificationSettings
              : onRequestNotifications}
            >{settingsView.notificationPermission === 'denied'
              ? t('settings.openSettings')
              : t('settings.allow')}</button
          >
        </div>
      </div>
    {/if}
  </div>

  <div class="settings-section">
    <h2>{t('settings.advanced')}</h2>
    <div class="setting-row">
      <span><b>{t('settings.logLevel')}</b></span><SelectMenu
        label={t('settings.logLevel')}
        value={settings.logLevel}
        options={[
          { value: 'error', label: t('settings.logError') },
          { value: 'warn', label: t('settings.logWarning') },
          { value: 'info', label: t('settings.logInfo') },
          { value: 'debug', label: t('settings.logDebug') },
        ]}
        onChange={(value) => patch({ logLevel: value as AppSettings['logLevel'] })}
      />
    </div>
    <div class="setting-row setting-row--button">
      <button class="secondary-button settings-wide-button" type="button" onclick={copyLogPath}
        >{t('settings.copyLogPath')}</button
      >
    </div>
    <div class="setting-row setting-row--button">
      <button class="secondary-button settings-wide-button" type="button" onclick={revealLogFile}
        >{revealLogLabel}</button
      >
    </div>
    {#if logActionError}<p class="settings-note log-action-error" role="alert">
        {logActionError}
      </p>{/if}
    <div class="setting-row setting-row--button">
      <button
        class="secondary-button settings-wide-button settings-reset-button"
        type="button"
        onclick={onResetAllSettings}>{t('settings.resetAllSettings')}</button
      >
    </div>
  </div>

  <div class="settings-section">
    <h2>{t('settings.updates')}</h2>
    <label class="setting-row"
      ><span><b>{t('settings.autoCheckUpdates')}</b></span><input
        type="checkbox"
        checked={settings.autoCheckUpdates}
        onchange={(event) => patch({ autoCheckUpdates: event.currentTarget.checked })}
      /></label
    >
    <div class="setting-row setting-row--button">
      <button
        type="button"
        class="secondary-button settings-wide-button"
        disabled={checkingUpdate}
        onclick={onCheckForUpdates}
        >{checkingUpdate ? t('settings.checkingUpdates') : t('settings.checkForUpdates')}</button
      >
    </div>
    {#if updateError}<div class="settings-update-error" role="alert">
        <b>{updateError.message}</b><small>{updateError.action}</small>
      </div>{/if}
  </div>

  <button
    class="screen-cross-link"
    type="button"
    aria-label={t('settings.customizeTitle')}
    onclick={onCustomize}
  >
    <Icon name="sliders" size={17} />
    <span><b>{t('settings.customizeTitle')}</b><small>{t('settings.customizeHint')}</small></span>
    <Icon name="chevron-right" size={13} strokeWidth={2.2} />
  </button>
</section>

<style>
  :global {
    .settings-section {
      margin-bottom: 10px;
    }

    .setting-row {
      display: flex;
      min-height: 40px;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      padding: 6px 10px;
      border-top: 1px solid var(--separator);
      font-size: 11px;
    }

    .settings-section h2 + .setting-row {
      border-top: 0;
    }

    .setting-row > span {
      display: flex;
      min-width: 0;
      flex-direction: column;
      gap: 1px;
    }

    .setting-row b {
      font-weight: 550;
    }

    .setting-row small {
      color: var(--secondary);
      font-size: 9px;
      line-height: 12px;
    }

    input[type='checkbox'] {
      width: 15px;
      height: 15px;
      accent-color: var(--meter-fill);
    }

    input[type='checkbox']:focus-visible {
      outline: 2px solid var(--meter-fill);
      outline-offset: 2px;
    }

    .settings-reset-button {
      color: var(--error);
    }

    .shortcut-field {
      display: flex;
      align-items: center;
      gap: 3px;
    }

    .shortcut-field button {
      max-width: 115px;
      padding: 4px 7px;
      overflow: hidden;
      border: 1px solid var(--separator);
      border-radius: 6px;
      color: var(--secondary);
      background: var(--tray);
      font-family: ui-monospace, monospace;
      font-size: 12px;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .shortcut-field button.recording {
      border-color: var(--meter-fill);
      color: var(--text);
    }

    .shortcut-field button.shortcut-clear {
      display: grid;
      width: 24px;
      height: 24px;
      padding: 0;
      color: var(--secondary);
      font-family: inherit;
      place-items: center;
    }

    .shortcut-field button.shortcut-clear:hover,
    .shortcut-field button.shortcut-clear:focus-visible {
      outline: none;
      color: var(--text);
      background: var(--button-hover);
    }

    .secondary-button {
      flex: 0 0 auto;
      padding: 4px 8px;
      border: 1px solid var(--separator);
      border-radius: 6px;
      color: var(--text);
      background: var(--tray);
      font-size: 12px;
      font-weight: 500;
    }

    .secondary-button:disabled {
      opacity: 0.55;
    }

    .permission-warning {
      display: inline-grid;
      width: 13px;
      height: 13px;
      margin-left: 3px;
      border-radius: 50%;
      color: white;
      background: var(--warning);
      font-size: 8px;
      place-items: center;
    }

    .settings-note,
    .version-row {
      margin: 0;
      padding: 6px 10px 9px;
      color: var(--warning);
      font-size: 9px;
    }

    .version-row {
      padding: 3px 0 8px;
      color: var(--tertiary);
      text-align: center;
    }

    .settings-section {
      margin-bottom: 14px;
      overflow: visible;
      background: transparent;
    }

    .settings-section > .setting-row {
      border-top: 0;
      background: var(--card);
    }

    .settings-section > h2 + .setting-row {
      border-radius: 12px 12px 0 0;
    }

    .settings-section > .setting-row:last-child,
    .settings-section > .settings-note:last-child {
      border-radius: 0 0 12px 12px;
    }

    .settings-section > h2 + .setting-row:last-child {
      border-radius: 12px;
    }

    .setting-row {
      min-height: 40px;
      padding: 9px 12px;
      border: 0;
      font-size: 13px;
    }

    .setting-row b {
      font-weight: 400;
    }

    .setting-row .select-menu__trigger {
      font-size: 13px;
    }

    .setting-row small {
      font-size: 10px;
      line-height: 12px;
    }

    input[type='checkbox'] {
      width: 28px;
      height: 16px;
      flex: 0 0 auto;
      margin: 0;
      appearance: none;
      border-radius: 9px;
      background: var(--meter-track);
      cursor: pointer;
      transition: background-color 160ms ease;
    }

    input[type='checkbox']::after {
      display: block;
      width: 12px;
      height: 12px;
      margin: 2px;
      border-radius: 50%;
      background: white;
      box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
      content: '';
      transition: transform 160ms ease;
    }

    input[type='checkbox']:checked {
      background: var(--meter-fill);
    }

    input[type='checkbox']:checked::after {
      transform: translateX(12px);
    }

    .version-row {
      font-size: 10px;
    }

    .setting-row > span:has(.setting-info) {
      align-items: center;
      flex-direction: row;
      gap: 6px;
    }

    .setting-info {
      display: inline-grid;
      flex: 0 0 auto;
      color: var(--secondary);
      font-style: normal;
      place-items: center;
    }

    .setting-row--button {
      display: block;
    }

    .settings-wide-button {
      width: 100%;
      min-height: 28px;
      font-size: 12px;
    }

    .settings-note.log-action-error {
      background: var(--card);
    }

    .notification-actions {
      padding: 8px 12px 10px;
      border-top: 1px solid var(--separator);
      border-radius: 0 0 12px 12px;
      background: var(--card);
    }

    .notification-attention {
      display: flex;
      align-items: center;
      gap: 10px;
      color: var(--warning);
    }

    .notification-attention > span {
      display: flex;
      min-width: 0;
      flex: 1;
      flex-direction: column;
      gap: 2px;
    }

    .notification-attention b {
      font-size: 11px;
      font-weight: 600;
      line-height: 13px;
    }

    .notification-attention small {
      color: var(--secondary);
      font-size: 9px;
      line-height: 12px;
    }

    .notification-attention .secondary-button {
      flex: 0 0 auto;
    }

    .settings-update-error {
      display: flex;
      flex-direction: column;
      gap: 2px;
      margin: 0 12px 8px;
      padding: 8px;
      border-radius: 8px;
      color: var(--error);
      background: var(--error-bg);
    }

    .settings-update-error b {
      font-size: 11px;
      line-height: 14px;
    }

    .settings-update-error small {
      color: var(--error);
      font-size: 9px;
      line-height: 12px;
    }

    :root[data-density='compact'] .setting-row {
      gap: 8px;
      padding-right: 10px;
      padding-left: 10px;
    }

    :root[data-density='compact'] .screen-cross-link {
      min-height: 42px;
      margin-top: 8px;
    }
  }
</style>
