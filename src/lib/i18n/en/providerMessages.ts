// Provider-authored notices and warnings, keyed by the stable ids the Rust providers send.
// Keys must stay identical to the chinese catalog: `catalog.test.ts` compares them.
export const providerNotices = {
  commandcode: {
    name: 'Command Code',
    description:
      'Sign in with `command-code auth login`. Reads monthly, purchased and free credits from Command Code.',
  },
  dataFrom: 'Data from {time}',
  claude: {
    rateLimited: {
      title: 'Live usage paused',
      ready: 'Ready to retry',
      retry: 'Retrying in about {duration}',
      rememberedRetry: 'Showing the last successful limits · Retrying in about {duration}',
    },
  },
};

export const providerWarnings = {
  claude: {
    reLoginRequired:
      'Re-login for live usage. Run `claude` and sign in again to restore subscription limits.',
    rateLimitedRetry: 'Claude live usage is rate limited; retrying in about {duration}.',
    rateLimitedStale: 'Claude live usage is rate limited; showing the last successful limits.',
    credentialNotSaved:
      'The refreshed Claude login is active for this session but could not be saved.',
  },
  codex: {
    credentialNotSaved:
      'The refreshed Codex login is active for this session but could not be saved.',
  },
  grok: {
    credentialNotSaved:
      'The refreshed Grok login is active for this session but could not be saved.',
  },
};
