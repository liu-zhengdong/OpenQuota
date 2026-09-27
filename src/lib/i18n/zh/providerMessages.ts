// Providers 的提示与警告文案，键名必须与英文目录完全一致（`catalog.test.ts` 会比对）。
export const providerNotices = {
  dataFrom: '数据来自 {time}',
  claude: {
    rateLimited: {
      title: '实时用量已暂停',
      ready: '可以重试了',
      retry: '约 {duration}后重试',
      rememberedRetry: '显示上次成功的限额 · 约 {duration}后重试',
    },
  },
};

export const providerWarnings = {
  claude: {
    reLoginRequired: '实时用量需要重新登录。在终端运行 `claude` 并重新登录，以恢复订阅限额。',
    rateLimitedRetry: 'Claude 实时用量接口限流，约 {duration}后重试。',
    rateLimitedStale: 'Claude 实时用量接口限流，当前显示上次成功的限额。',
    credentialNotSaved: '刷新后的 Claude 登录本次会话可用，但未能保存。',
  },
  codex: {
    credentialNotSaved: '刷新后的 Codex 登录本次会话可用，但未能保存。',
  },
  grok: {
    credentialNotSaved: '刷新后的 Grok 登录本次会话可用，但未能保存。',
  },
};
