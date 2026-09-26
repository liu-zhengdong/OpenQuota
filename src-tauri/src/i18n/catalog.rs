use super::locale::UiLocale;

pub fn t(locale: UiLocale, key: &str) -> &'static str {
    if locale == UiLocale::Zh {
        if let Some(value) = zh(key) {
            return value;
        }
    }
    en(key).unwrap_or("")
}

pub fn window_label(locale: UiLocale, id: &str, fallback: &str) -> String {
    let key = window_key(id);
    if let Some(value) = match locale {
        UiLocale::Zh => zh(key).or_else(|| en(key)),
        UiLocale::En => en(key),
    } {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn window_key(id: &str) -> &'static str {
    match id {
        "session" => "windows.session",
        "weekly" => "windows.weekly",
        "daily" => "windows.daily",
        "monthly" => "windows.monthly",
        "extra" => "windows.extra",
        "credits" => "windows.credits",
        "rateLimitResets" => "windows.rateLimitResets",
        "spark" => "windows.spark",
        "sparkWeekly" => "windows.sparkWeekly",
        "geminiPro" => "windows.geminiPro",
        "geminiWeekly" => "windows.geminiWeekly",
        "claude" => "windows.claude",
        "claudeWeekly" => "windows.claudeWeekly",
        "webSearches" => "windows.webSearches",
        "extraUsageBalance" => "windows.extraUsageBalance",
        "extraBalance" => "windows.extraBalance",
        "payAsYouGo" => "windows.payAsYouGo",
        "orgCredits" => "windows.orgCredits",
        "orgSpend" => "windows.orgSpend",
        "usage" => "windows.usage",
        "auto" => "windows.auto",
        "api" => "windows.api",
        "onDemand" => "windows.onDemand",
        "requests" => "windows.requests",
        "premium" => "windows.premium",
        "chat" => "windows.chat",
        "completions" => "windows.completions",
        "balance" => "windows.balance",
        "today" => "windows.today",
        "week" => "windows.week",
        "month" => "windows.month",
        "keyLimit" => "windows.keyLimit",
        "sonnet" => "windows.sonnet",
        _ => "",
    }
}

fn en(key: &str) -> Option<&'static str> {
    Some(match key {
        "tray.open" => "Open OpenQuota",
        "tray.customize" => "Customize…",
        "tray.settings" => "Settings",
        "tray.settingsEllipsis" => "Settings…",
        "tray.quit" => "Quit OpenQuota",
        "notifications.almostOutTitle" => "Almost Out",
        "notifications.almostOutBody" => "Under 10% usage remaining for this window.",
        "notifications.cuttingItCloseTitle" => "Cutting It Close",
        "notifications.cuttingItCloseBody" => "Projected to finish close to your limit.",
        "notifications.willRunOutTitle" => "Will Run Out",
        "notifications.willRunOutBody" => "Projected to run out before the limit resets.",
        "notifications.openAction" => "Open OpenQuota",
        "windows.session" => "Session",
        "windows.weekly" => "Weekly",
        "windows.daily" => "Daily",
        "windows.monthly" => "Monthly",
        "windows.extra" => "Extra Usage",
        "windows.credits" => "Credits",
        "windows.rateLimitResets" => "Rate Limit Resets",
        "windows.spark" => "Spark",
        "windows.sparkWeekly" => "Spark Weekly",
        "windows.geminiPro" => "Session",
        "windows.geminiWeekly" => "Weekly",
        "windows.claude" => "Claude",
        "windows.claudeWeekly" => "Claude Weekly",
        "windows.webSearches" => "Web Searches",
        "windows.extraUsageBalance" => "Extra Usage Balance",
        "windows.extraBalance" => "Extra Balance",
        "windows.payAsYouGo" => "Extra Usage",
        "windows.orgCredits" => "Org Credits",
        "windows.orgSpend" => "Org Spend",
        "windows.usage" => "Total Usage",
        "windows.auto" => "Auto Usage",
        "windows.api" => "API Usage",
        "windows.onDemand" => "Extra Usage",
        "windows.requests" => "Requests",
        "windows.premium" => "Credits",
        "windows.chat" => "Chat",
        "windows.completions" => "Completions",
        "windows.balance" => "Balance",
        "windows.today" => "Today",
        "windows.week" => "This Week",
        "windows.month" => "This Month",
        "windows.keyLimit" => "Key Limit",
        "windows.sonnet" => "Sonnet",
        _ => return None,
    })
}

fn zh(key: &str) -> Option<&'static str> {
    Some(match key {
        "tray.open" => "打开 OpenQuota",
        "tray.customize" => "自定义…",
        "tray.settings" => "设置",
        "tray.settingsEllipsis" => "设置…",
        "tray.quit" => "退出 OpenQuota",
        "notifications.almostOutTitle" => "即将用尽",
        "notifications.almostOutBody" => "该窗口剩余用量不足 10%。",
        "notifications.cuttingItCloseTitle" => "余量偏紧",
        "notifications.cuttingItCloseBody" => "预计重置时会接近上限。",
        "notifications.willRunOutTitle" => "将会用尽",
        "notifications.willRunOutBody" => "预计在上限重置前用尽。",
        "notifications.openAction" => "打开 OpenQuota",
        "windows.session" => "5 小时窗口",
        "windows.weekly" => "每周",
        "windows.daily" => "每天",
        "windows.monthly" => "每月",
        "windows.extra" => "额外用量",
        "windows.credits" => "额度",
        "windows.rateLimitResets" => "速率限制重置",
        "windows.spark" => "Spark",
        "windows.sparkWeekly" => "Spark 每周",
        "windows.geminiPro" => "5 小时窗口",
        "windows.geminiWeekly" => "每周",
        "windows.claude" => "Claude",
        "windows.claudeWeekly" => "Claude 每周",
        "windows.webSearches" => "网页搜索",
        "windows.extraUsageBalance" => "额外用量余额",
        "windows.extraBalance" => "额外余额",
        "windows.payAsYouGo" => "额外用量",
        "windows.orgCredits" => "组织额度",
        "windows.orgSpend" => "组织支出",
        "windows.usage" => "总用量",
        "windows.auto" => "Auto 用量",
        "windows.api" => "API 用量",
        "windows.onDemand" => "额外用量",
        "windows.requests" => "请求",
        "windows.premium" => "额度",
        "windows.chat" => "对话",
        "windows.completions" => "补全",
        "windows.balance" => "余额",
        "windows.today" => "今天",
        "windows.week" => "本周",
        "windows.month" => "本月",
        "windows.keyLimit" => "密钥限额",
        "windows.sonnet" => "Sonnet",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{en, t, window_label, zh};
    use crate::i18n::locale::UiLocale;

    #[test]
    fn chinese_catalog_keys_match_english() {
        let keys = [
            "tray.open",
            "tray.customize",
            "tray.settings",
            "tray.settingsEllipsis",
            "tray.quit",
            "notifications.almostOutTitle",
            "notifications.almostOutBody",
            "notifications.cuttingItCloseTitle",
            "notifications.cuttingItCloseBody",
            "notifications.willRunOutTitle",
            "notifications.willRunOutBody",
            "notifications.openAction",
            "windows.session",
            "windows.weekly",
            "windows.daily",
            "windows.monthly",
            "windows.extra",
            "windows.credits",
            "windows.rateLimitResets",
            "windows.spark",
            "windows.sparkWeekly",
            "windows.geminiPro",
            "windows.geminiWeekly",
            "windows.claude",
            "windows.claudeWeekly",
            "windows.webSearches",
            "windows.extraUsageBalance",
            "windows.extraBalance",
            "windows.payAsYouGo",
            "windows.orgCredits",
            "windows.orgSpend",
            "windows.usage",
            "windows.auto",
            "windows.api",
            "windows.onDemand",
            "windows.requests",
            "windows.premium",
            "windows.chat",
            "windows.completions",
            "windows.balance",
            "windows.today",
            "windows.week",
            "windows.month",
            "windows.keyLimit",
            "windows.sonnet",
        ];
        for key in keys {
            assert!(en(key).is_some(), "missing english key {key}");
            assert!(zh(key).is_some(), "missing chinese key {key}");
        }
    }

    #[test]
    fn window_labels_use_id_not_english_fallback_when_known() {
        assert_eq!(
            window_label(UiLocale::Zh, "session", "Session"),
            "5 小时窗口"
        );
        assert_eq!(window_label(UiLocale::Zh, "weekly", "Weekly"), "每周");
        assert_eq!(
            window_label(UiLocale::Zh, "scoped-opus-5-5", "Opus 5.5"),
            "Opus 5.5"
        );
        assert_eq!(t(UiLocale::Zh, "tray.settings"), "设置");
    }
}
