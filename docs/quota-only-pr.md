# 仅保留云端额度，切换 fork 更新源并支持 Claude 多模型窗口

删除 Claude、Codex、Grok、pi 和 OpenCode 本地日志／数据库用量扫描，以及 Cursor 历史 CSV 下载、token 与美元估算、模型明细、总花费和趋势界面。移除价目表下载、资源及专用依赖；快照不再包含 `usage`。启动时幂等删除旧 `daily_usage`、`log_file_cache` 表，旧快照和设置中的历史字段可忽略，账户认证及额度缓存保留。云端 credits、Extra Usage 等数值不受影响。

更新、发布和下载链接指向 `liu-zhengdong/OpenQuota`，保留 `io.github.deviffyy.openquota` 标识和当前 pubkey。Claude 读取所有具备模型名称的 scoped 窗口，保留 `fable` ID，新窗口进入界面、自定义设置和托盘；缺名称的记录跳过。

## 发布前必须完成

- 由维护者生成新的 minisign／Tauri updater 签名密钥。
- 在 fork 配置 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 两个 GitHub Actions secrets。
- 将 `src-tauri/tauri.conf.json` 中 updater 的 `pubkey` 替换为新公钥。
- 本次仅修改更新源，未更换 pubkey。旧安装信任上游密钥，无法验证 fork 新密钥签名的更新；第一次迁移需要手动安装使用新公钥构建的版本，此后才能自动更新。

本说明为本地 PR 草稿；本次不推送分支、不创建 PR。

## 验证

- 版本一致性和全部前后端契约检查通过。
- 前端 lint、类型检查通过，0 错误、0 告警；33 个测试文件共 200 项测试串行通过；生产构建通过。
- Rust `cargo fmt --check`、全目标 clippy（`-D warnings`）通过，0 告警；全目标测试串行通过 415 项。
- macOS Apple Silicon DMG 构建通过，使用 ad-hoc 签名；缺少 Apple 公证凭据时有 1 条预期的跳过公证告警。
- Windows/Linux 构建、独立系统凭据库集成和应用启动冒烟未验证。未启动已安装应用，未修改用户的 `~/Library/Application Support` 数据；数据库迁移仅在临时测试数据库中验证。
