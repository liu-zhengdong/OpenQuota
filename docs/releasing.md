# Releasing OpenQuota

## 准备版本与变更说明

打 tag 前，在同一个 `chore(release): X.Y.Z` 提交中完成：

1. 更新 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 的版本号，
   同步 `src-tauri/Cargo.lock`。
2. 将 `docs/release-notes/` 中的上一版文件替换为 `vX.Y.Z.md`，只保留当前版本一份。
   正文先写中文「本版变更」：用户能感知的变化、受影响的平台、必要的升级操作；
   再保留平台构建与更新签名说明。旧版正文保存在 Git 历史与 GitHub release 中。
3. 运行 `node scripts/verify/verify-versions.js vX.Y.Z`，审阅完整正文后提交版本改动。

版本提交合入后再打对应的 `vX.Y.Z` tag。Release workflow 从已验证的 tag 提交读取
`docs/release-notes/$RELEASE_TAG.md`，通过 `--notes-file` 原样写入 draft 正文；
重跑复用 draft 时也会同步。正文文件必须存在，不再自动生成 PR 列表。

`verify_only` 在生成更新元数据前同样同步这份正文，因此该模式所用 tag 提交也必须
包含说明文件。`.github/scripts/create-updater-json.mjs` 将 `release.body` 直接写入
`latest.json.notes`，GitHub release 页与 App 更新提示使用同一份说明，发布后无需补写。

## Signing policy

OpenQuota treats updater signatures and native operating-system signatures as separate trust
layers. Updater artifacts must always be signed with `TAURI_SIGNING_PRIVATE_KEY`. Native Windows
and macOS signing are independent opt-ins because they require externally provisioned certificates.
If the updater key is encrypted, also configure `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The updater
key is project-generated and does not require a paid certificate authority or signing account.

## Default release policy

Leave both native-signing repository variables unset or set them to `false`:

- `ENABLE_WINDOWS_NATIVE_SIGNING`
- `ENABLE_MACOS_NATIVE_SIGNING`

The release workflow then builds an unsigned Windows installer and an ad-hoc-signed, unnotarized
macOS application. Package installation and startup smoke tests still run, but Authenticode,
Gatekeeper, and notarization checks are skipped. The workflow emits warnings, and the download
documentation describes the unavailable native trust layers.

This default does not weaken updater verification. Tauri updater signatures are still generated,
uploaded, and verified with the bundled public key before publication.

## Enabling Windows native signing

Set `ENABLE_WINDOWS_NATIVE_SIGNING` to `true` only after configuring all of the following:

| Kind             | Name                     |
| ---------------- | ------------------------ |
| Actions secret   | `ES_USERNAME`            |
| Actions secret   | `ES_PASSWORD`            |
| Actions secret   | `ES_CREDENTIAL_ID`       |
| Actions secret   | `ES_TOTP_SECRET`         |
| Actions variable | `WINDOWS_SIGNER_SUBJECT` |

This enables the reviewed SSL.com CodeSignTool configuration. The workflow then requires a valid,
timestamped Authenticode signature on both the installer and installed executable. Missing or
incorrect values stop the release rather than silently producing an unsigned Windows artifact.

## Enabling macOS native signing

Set `ENABLE_MACOS_NATIVE_SIGNING` to `true` only after configuring all of the following Actions
secrets:

- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`

`APPLE_PASSWORD` is an app-specific password used for notarization, not the account's normal login
password.

The direct-download DMG uses a Developer ID Application certificate, not an App Store Distribution
certificate. When enabled, the workflow requires the expected team identity, hardened runtime,
secure timestamp, Gatekeeper approval, and a valid notarization staple. Missing or incorrect values
stop the release rather than falling back to ad-hoc signing.

Both opt-ins accept only the exact strings `true` and `false`. An invalid value stops validation so a
typo cannot silently change release trust policy. A `verify_only` run publishes an already-built
draft and therefore does not require private signing credentials. The two policy variables must
still match the draft's native-signing state; a mismatch stops publication.
