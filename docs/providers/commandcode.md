# Command Code

OpenQuota shows your Command Code plan, total remaining credits and free credits separately.
Credit utilization, monthly and purchased credits are available under **Show more**. The panel also shows days until
renewal and 5-hour or weekly limits when Command Code reports them.

## Setup

Run `command-code auth login`, then enable **Command Code** in OpenQuota's **Customize** panel.
OpenQuota reads the CLI's `~/.commandcode/auth.json` without changing it. If that file has no key,
OpenQuota checks `COMMAND_CODE_API_KEY`. It does not save another copy of the key.

## Troubleshooting

- **Session expired** — run `command-code auth login` again, then retry the refresh.
- **Rate limited** — retry later; the last successful reading remains visible.
- **No window shown** — Command Code has not reported a supported window. Credits remain visible.
