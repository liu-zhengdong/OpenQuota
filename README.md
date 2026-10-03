<p align="center">
  <img src="assets/openquota-icon.png" alt="OpenQuota logo" width="88">
</p>

<h1 align="center">OpenQuota</h1>

<p align="center">
  Track usage and limits across your AI coding tools.
</p>

<p align="center">
  <a href="https://github.com/liu-zhengdong/OpenQuota/actions/workflows/ci.yml"><img src="https://github.com/liu-zhengdong/OpenQuota/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <a href="https://github.com/liu-zhengdong/OpenQuota/releases/latest"><img src="https://img.shields.io/github/v/release/liu-zhengdong/OpenQuota" alt="Latest release"></a>
  <a href="https://github.com/liu-zhengdong/OpenQuota/releases"><img src="https://img.shields.io/github/downloads/liu-zhengdong/OpenQuota/total" alt="Total downloads"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT license"></a>
</p>

OpenQuota brings usage data from Claude Code, Codex, Cursor, Copilot, and other AI coding providers
into one compact panel. See session and weekly limits, reset times, and cloud-reported credits at a glance.

<p align="center">
  <img src="assets/openquota-demo.gif" alt="OpenQuota dashboard showing animated AI usage in light and dark themes" width="840">
</p>

## Download

| Platform | Available builds                           | Download                                                                           |
| -------- | ------------------------------------------ | ---------------------------------------------------------------------------------- |
| Windows  | x64 and ARM64 installers                   | [Download for Windows](https://github.com/liu-zhengdong/OpenQuota/releases/latest) |
| macOS    | Universal DMG for Apple Silicon and Intel  | [Download for macOS](https://github.com/liu-zhengdong/OpenQuota/releases/latest)   |
| Linux    | x64 and ARM64 AppImage and Debian packages | [Download for Linux](https://github.com/liu-zhengdong/OpenQuota/releases/latest)   |

Open the latest release and choose the file for your platform:

- **Windows:** `_x64-setup.exe` or `_arm64-setup.exe`
- **macOS:** `_universal.dmg` — requires macOS 11 or later
- **Linux:** `.AppImage` or `.deb`

OpenQuota checks for updates automatically. Update payloads are cryptographically signed with the
project's updater key, independently from operating-system package signing.

When native signing is disabled (the current default), Windows installers are not
Authenticode-signed, while the macOS app uses an ad-hoc signature and is not Apple-notarized.
Windows SmartScreen or macOS Gatekeeper may therefore ask for confirmation. Download OpenQuota only
from this repository's official release page; on macOS, manual approval may be required in Privacy
& Security. Each release states its exact native-signing status in its notes.

## Supported providers

- **[Claude Code](docs/providers/claude.md)** — multiple accounts, session and weekly limits,
  model-specific quota windows
- **[Codex](docs/providers/codex.md)** — session and weekly limits, credits, and model-specific limits
- **[Cursor](docs/providers/cursor.md)** — total, Auto and API usage, and credits
- **[Antigravity](docs/providers/antigravity.md)** — shared Gemini and Claude quota pools
- **[Copilot](docs/providers/copilot.md)** — premium requests, extra usage, chat and completion
  quotas, plus organization billing
- **[Devin](docs/providers/devin.md)** — daily and weekly limits, reset times, and extra usage balance
- **[Grok](docs/providers/grok.md)** — weekly allowance and extra usage status
- **[OpenCode](docs/providers/opencode.md)** — OpenCode Go session, weekly and monthly spend caps
- **[OpenRouter](docs/providers/openrouter.md)** — credit balance and daily, weekly and monthly spend
  (API key)
- **[Z.ai](docs/providers/zai.md)** — GLM Coding Plan session, weekly, and web-search quotas (API key)
- **[Kimi](docs/providers/kimi.md)** — Kimi Code session and weekly quotas (API key)
- **[MiniMax](docs/providers/minimax.md)** — Token Plan session and weekly quotas (API key)
- **[Command Code](docs/providers/commandcode.md)** — monthly, purchased and free credits, renewal
  date, and available 5-hour and weekly windows (CLI login)

Most providers use credentials already available on your computer. OpenRouter, Z.ai, Kimi, and
MiniMax require API keys, which you can add in Customize; OpenQuota stores them securely in your
operating system's credential store. Codex subscription limits require a ChatGPT login and are not
available in API-key-only sessions.

## Features

- **Tray or floating dashboard.** View quotas in a compact popup, or keep the panel open and move it
  around your desktop.
- **Pinned metrics.** Keep important values visible in the tray or macOS menu bar.
- **Used or left.** Display how much quota you have consumed or how much remains.
- **Pacing alerts.** See whether your current usage is likely to last until the next reset.
- **Custom layouts.** Reorder providers and metrics, hide rows, and choose what stays visible.
- **Desktop integration.** Launch at login, use a global shortcut, and follow the system theme.
- **Fast refresh.** Cached values appear immediately and providers refresh automatically in the
  background.

OpenQuota runs locally and has no account, cloud backend, analytics, or usage telemetry of its own.

## Command line

The same executable doubles as a query tool. Passing the `pace` subcommand skips the GUI and the
single-instance hand-off: it opens the local database read-only, prints the latest cached snapshots
of the providers turned on in the panel, and exits. Providers turned off in the panel are left out.

```sh
/Applications/OpenQuota.app/Contents/MacOS/openquota pace          # aligned text table
/Applications/OpenQuota.app/Contents/MacOS/openquota pace --json   # stable JSON array
/Applications/OpenQuota.app/Contents/MacOS/openquota pace --refresh --only cursor,grok --timeout 15
```

Each provider shows the window used for comparison (weekly when one exists, otherwise the longest
percent window), used percent, elapsed percent of the period, spare percent (elapsed − used, so a
positive value means usage is running behind an even pace), hours to reset, the short session
window's used percent, the snapshot age, and a `DATA` column reading `live`, `cached`, or `stale`.
Rows are sorted by spare, highest first. `stale` means the row is older than the staleness window
the panel marks, or a requested refresh failed, so the numbers are the last successful read rather
than a current one. When the OpenQuota app is not running, nothing keeps the cached readings up to
date: the command prints a line on stderr with the age of the oldest one, and the text table marks
those rows with `*`.

Without `--refresh` the command never touches the network and never writes. `--refresh` first pulls
current readings from the enabled providers (or only those named with `--only`) in parallel, with
a per-provider time limit set by `--timeout` in seconds (default 20). A pull that fails or times out
keeps the earlier reading, marked `stale`, without failing the command; readings are never
estimated. A pull still running at its limit may hold the exit back by up to 15 seconds after the
report is printed, so a login it is renewing gets saved. A provider read within the last 60 seconds, by the app or an earlier run, is reused
instead of pulled again. Pulled readings are not written to the application database, so they
cannot collide with the running app; they are kept in `pace-live.json` next to it, a file only this
command uses. As in the app, a pull may renew and save that provider's own expired login. Claude
and Codex keep account records in the application database, so only the app refreshes them.

`--json` prints a JSON array with camelCase fields and ISO 8601 UTC timestamps for scripts. Exit
codes are `0` for a printed report, `1` when the database could not be read, `2` when it holds no
snapshots, and `64` for an invalid command line. Snapshots that fail to parse are reported on
stderr and skipped. `--db <path>` and `OPENQUOTA_PACE_DB` point the command at another database
file, which is intended for testing.

## Development

Requirements:

- Node.js 22 or later
- pnpm 11.11.0
- Stable Rust toolchain
- [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/)

Install dependencies and start the development app:

```sh
corepack pnpm install --frozen-lockfile
corepack pnpm tauri dev
```

Run the complete quality checks:

```sh
corepack pnpm verify
```

Build an installer for the current platform:

```sh
corepack pnpm build:installer             # Windows
corepack pnpm build:linux                 # Linux
corepack pnpm tauri build --bundles dmg   # macOS
```

Maintainers can review updater and optional native-signing requirements in
[docs/releasing.md](docs/releasing.md).

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before contributing,
and report security problems privately as described in [SECURITY.md](SECURITY.md).

## Acknowledgements

OpenQuota was inspired by [OpenUsage](https://github.com/robinebers/openusage) and developed as a
cross-platform alternative for Windows, Linux, and macOS.

## License

[MIT](LICENSE)
