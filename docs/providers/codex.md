# Codex

OpenQuota tracks Codex subscription limits and credits reported by the Codex cloud API.

## What it tracks

| Metric               | Meaning                                                      |
| -------------------- | ------------------------------------------------------------ |
| Session              | Usage remaining in the current session window                |
| Weekly               | Usage remaining in the weekly window                         |
| Spark / Spark Weekly | Model-specific limits when they are reported for the account |
| Extra Usage          | Additional usage credits reported by Codex                   |
| Rate Limit Resets    | Available reset credits                                      |

## Sign-in

Sign in with the Codex CLI by running `codex` and choosing your ChatGPT account. OpenQuota reads the
same authentication data and respects `CODEX_HOME` when it is set. API-key-only sessions cannot provide ChatGPT subscription limits.

## Troubleshooting

- **Not logged in** — run `codex`, sign in with ChatGPT, then refresh OpenQuota.
- **Subscription usage unavailable** — replace an API-key-only login with a ChatGPT login.
- **Session expired or revoked** — sign in again with `codex`.
