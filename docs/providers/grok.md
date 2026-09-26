# Grok

OpenQuota tracks Grok allowance information reported by Grok.

## What it tracks

| Metric      | Meaning                                     |
| ----------- | ------------------------------------------- |
| Weekly      | Weekly allowance remaining                  |
| Extra Usage | Pay-as-you-go availability reported by Grok |

Accounts that still use Grok's older billing model may not report a weekly pool, in which case the
Weekly row shows **No data**.

## Sign-in

Sign in by running `grok login`. OpenQuota reads the authentication data stored by
the Grok CLI. `GROK_HOME` is respected when it is set.

## Troubleshooting

- **Not logged in** — run `grok login`, then refresh OpenQuota.
- **Login invalid or expired** — sign in again with the Grok CLI.
- **Billing request failed** — check the connection and try another refresh.
