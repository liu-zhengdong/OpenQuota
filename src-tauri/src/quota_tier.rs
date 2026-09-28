const CRITICAL_THRESHOLD: f64 = 0.2;
const HEALTHY_THRESHOLD: f64 = 0.6;

/// Remaining-quota band shared by the Windows/Linux ring and the macOS compact menu bar marks.
/// Ordered from most to least urgent so the tightest of several windows is simply the minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum QuotaTier {
    Critical,
    Caution,
    Healthy,
}

pub(crate) fn quota_tier(remaining_fraction: f64) -> QuotaTier {
    let remaining = if remaining_fraction.is_finite() {
        remaining_fraction
    } else {
        0.0
    };
    if remaining >= HEALTHY_THRESHOLD {
        QuotaTier::Healthy
    } else if remaining > CRITICAL_THRESHOLD {
        QuotaTier::Caution
    } else {
        QuotaTier::Critical
    }
}

#[cfg(test)]
mod tests {
    use super::{quota_tier, QuotaTier};

    #[test]
    fn remaining_quota_tiers_use_the_shared_thresholds() {
        assert_eq!(quota_tier(1.0), QuotaTier::Healthy);
        assert_eq!(quota_tier(0.6), QuotaTier::Healthy);
        assert_eq!(quota_tier(0.59), QuotaTier::Caution);
        assert_eq!(quota_tier(0.21), QuotaTier::Caution);
        assert_eq!(quota_tier(0.2), QuotaTier::Critical);
        assert_eq!(quota_tier(0.0), QuotaTier::Critical);
        assert_eq!(quota_tier(f64::NAN), QuotaTier::Critical);
        assert_eq!(quota_tier(f64::INFINITY), QuotaTier::Critical);
        assert!(QuotaTier::Critical < QuotaTier::Caution);
        assert!(QuotaTier::Caution < QuotaTier::Healthy);
    }
}
