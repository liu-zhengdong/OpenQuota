# Claude Code

OpenQuota tracks Claude subscription limits and cloud-reported model quota windows.

## What it tracks

| Metric        | Meaning                                                      |
| ------------- | ------------------------------------------------------------ |
| Session       | Usage remaining in the current session window                |
| Weekly        | Usage remaining in the weekly window                         |
| Model windows | Model-specific limits when they are reported for the account |
| Extra Usage   | Extra-usage allowance or spending reported by Claude         |

## Sign-in

Sign in with Claude Code by running `claude`. OpenQuota reuses the credentials maintained by the
CLI, including `CLAUDE_CONFIG_DIR` when it is set. Refreshed CLI credentials are saved back to the
same source when possible.

## Multiple accounts

OpenQuota discovers separate Claude Code logins that use custom `CLAUDE_CONFIG_DIR` homes and shows
each account as its own card with independent limits and plan. Logins belonging
to the same Claude account are combined automatically.

Account cards can be renamed from Customize or from the dashboard. If a login is removed, its card
is hidden and returns with its previous customization when the login is detected again.

Live subscription limits currently require a Claude Code login. On macOS, OpenQuota can recognize
that Claude Desktop is installed, but it does not reuse Desktop's encrypted session. Run `claude`
and sign in once if Desktop is your only Claude login.

## Rate limits

Anthropic rate limits the usage endpoint, so a limited card shows a *Live usage paused* notice
with the time until the next attempt. Claude answers 429 with a `Retry-After` header; when there is
none, OpenQuota waits five minutes.

The card keeps showing the last limits it managed to read, labelled with the time they were read,
so a rate limit does not blank out the numbers. That last successful snapshot is stored in the
local database, which means it also survives a restart: reopening OpenQuota inside a rate limit
window replays the same numbers instead of showing an empty card. Remembered values never replace
newer data on disk, and they are reported as `stale` by `openquota pace` once they pass the
staleness window.

## Troubleshooting

- **Not logged in** — run `claude`, complete sign-in, then refresh OpenQuota.
- **Claude Desktop login found** — sign in once through the Claude Code CLI.
- **Session or token expired** — sign in again with `claude`.
- **Live usage paused** — expected while Anthropic is rate limiting the account. The limits shown
  are the last successful read; the card recovers on its own once the wait elapses.
