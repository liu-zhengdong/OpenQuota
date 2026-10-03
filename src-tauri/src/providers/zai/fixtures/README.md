# Z.ai fixtures

`credits.json` 是合成响应，不是真实账号采样。字段形状依据国际 Z.ai 官方控制台公开代码；`remaining` 用于验证来源显式提供该字段时的透传，并不证明当前线上一定提供剩余字段。数值、重置时间仅用于断言，不作为产品默认值。

2026-10-03 核查：
- `https://z.ai/manage-apikey/coding-plan/personal/usage` 的公开 bundle `0rtke2bos45xk.js`：`getUsageQuotaLimitApi` GET `/monitor/usage/quota/limit`；积分按 `CREDIT_LIMIT` 读取 `percentage/currentValue/usage/nextResetTime`。
- 同站 bundle `2g762vxr39fvr.js`：`getCreditUsageLimitsDefaults` 将 unit 3/6 分别标为 5 小时/每周、单位 Credits。
- 官方 `zai-org/zai-coding-plugins` 的 `plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs`：国际 URL 为 `https://api.z.ai/api/monitor/usage/quota/limit`，Authorization 直接使用 token；中国站使用自己的域名。读取器仅接国际站，不跨区回退。

`quota.json` / `subscription.json` 是既有旧套餐 fixtures，来源沿原提交记录；本轮没有用它们宣称新积分套餐已经真实账号实测。
