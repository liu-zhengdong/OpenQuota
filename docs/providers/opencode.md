# OpenCode

OpenQuota tracks cloud-reported OpenCode Go quota information.

## What it tracks

| Metric  | Meaning                          |
| ------- | -------------------------------- |
| Session | OpenCode Go rolling-window usage |
| Weekly  | OpenCode Go weekly usage         |
| Monthly | OpenCode Go monthly usage        |

Go quota rows require a compatible OpenCode Go login.

The Go meters come from OpenCode's account usage endpoint, so they include usage from all devices
and reflect the limits enforced by OpenCode.

## Sign-in

Sign in to OpenCode Go. OpenQuota reads OpenCode's local authentication
file from its data directory. `OPENCODE_DATA_DIR` and `XDG_DATA_HOME` are respected
when present.

## Troubleshooting

- **OpenCode was not detected** — sign in to OpenCode Go.
- **Login data could not be read** — sign in to OpenCode Go again.
