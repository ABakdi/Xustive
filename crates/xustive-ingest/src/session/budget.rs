//! Per-identity request budgets and pacing ([[Session Manager]] §4.5).
//!
//! Budgets are **per identity, not per IP** — platforms rate-limit the account far more tightly than
//! the address. Each identity has an hourly and a daily allowance, scaled by its lifecycle tier
//! (`warming` gets a fraction of `mature`), and requests are *shaped*, not merely spaced: a jittered
//! minimum gap, and a diurnal active window offset per identity so it is not uniformly busy at
//! 04:00. Starting values are deliberately conservative; the point is that the feedback signal (a
//! ban) arrives long after the behaviour that caused it, so the safe direction is slow.

use std::time::Duration;

/// The static allowance for an identity at full (mature) budget ([[Session Manager]] §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    pub hourly: u32,
    pub daily: u32,
    pub min_gap_ms: u64,
    /// Jitter as a fraction of the gap, e.g. `0.4` for ±40 %.
    pub jitter_frac_pct: u32,
    /// The identity's daily active window, in local (Africa/Algiers) hours: `[start, start+len)`.
    pub active_start_hour: u8,
    pub active_len_hours: u8,
}

impl Default for BudgetLimits {
    /// Instagram's conservative values — the tightest of the platforms, so the safe default.
    fn default() -> Self {
        Self::instagram()
    }
}

impl BudgetLimits {
    /// Instagram's conservative starting values (§4.5).
    pub fn instagram() -> Self {
        Self {
            hourly: 60,
            daily: 400,
            min_gap_ms: 2_500,
            jitter_frac_pct: 40,
            active_start_hour: 8,
            active_len_hours: 10,
        }
    }

    /// The effective limits for a tier: the mature allowance scaled by the tier's budget ratio, so a
    /// warming identity gets a quarter of the hourly and daily counts.
    pub fn scaled(&self, ratio: f64) -> (u32, u32) {
        let r = ratio.clamp(0.0, 1.0);
        (
            (self.hourly as f64 * r).floor() as u32,
            (self.daily as f64 * r).floor() as u32,
        )
    }

    /// Whether the identity's active window covers `local_hour` (0–23). A window that wraps past
    /// midnight (e.g. start 22, len 6 → 22,23,0..4) is handled.
    pub fn is_active_hour(&self, local_hour: u8) -> bool {
        let len = self.active_len_hours.min(24);
        let start = self.active_start_hour % 24;
        let h = local_hour % 24;
        // Distance forward from start to h, modulo 24, is within the window length.
        let dist = (24 + h as i32 - start as i32) % 24;
        (dist as u32) < len as u32
    }

    /// The gap before the next request: the minimum gap jittered by `±jitter_frac`. `dice` in
    /// `0.0..1.0` picks the point in the jitter range — a random draw in production, fixed in a test.
    pub fn next_gap(&self, dice: f64) -> Duration {
        let frac = self.jitter_frac_pct as f64 / 100.0;
        // Map dice∈[0,1) to a multiplier in [1-frac, 1+frac].
        let mult = 1.0 + (dice.clamp(0.0, 1.0) * 2.0 - 1.0) * frac;
        let ms = (self.min_gap_ms as f64 * mult).max(0.0) as u64;
        Duration::from_millis(ms)
    }
}

/// Live spend against a tier's scaled limits, within one hour/day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BudgetSpend {
    pub used_hour: u32,
    pub used_day: u32,
}

impl BudgetSpend {
    /// Whether another request fits within both the hourly and daily caps for `(hourly, daily)`.
    pub fn can_spend(&self, hourly: u32, daily: u32) -> bool {
        self.used_hour < hourly && self.used_day < daily
    }

    /// Requests still available this hour, given the effective hourly cap.
    pub fn remaining_hour(&self, hourly: u32) -> u32 {
        hourly.saturating_sub(self.used_hour)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warming_gets_a_quarter_of_the_mature_allowance() {
        let l = BudgetLimits::instagram();
        assert_eq!(l.scaled(1.0), (60, 400));
        assert_eq!(l.scaled(0.25), (15, 100));
        assert_eq!(l.scaled(0.0), (0, 0), "fresh/quarantined collect nothing");
    }

    #[test]
    fn the_active_window_is_a_per_identity_slice_of_the_day() {
        let l = BudgetLimits {
            active_start_hour: 8,
            active_len_hours: 10,
            ..BudgetLimits::instagram()
        };
        assert!(l.is_active_hour(8));
        assert!(l.is_active_hour(17));
        assert!(!l.is_active_hour(18), "the window ends before hour 18");
        assert!(!l.is_active_hour(4), "and an identity is idle at 04:00");
    }

    #[test]
    fn a_window_wrapping_past_midnight_is_handled() {
        let l = BudgetLimits {
            active_start_hour: 22,
            active_len_hours: 6, // 22,23,0,1,2,3
            ..BudgetLimits::instagram()
        };
        assert!(l.is_active_hour(23));
        assert!(l.is_active_hour(2));
        assert!(!l.is_active_hour(4));
        assert!(!l.is_active_hour(21));
    }

    #[test]
    fn the_gap_is_jittered_within_the_configured_band() {
        let l = BudgetLimits {
            min_gap_ms: 1000,
            jitter_frac_pct: 40,
            ..BudgetLimits::instagram()
        };
        // dice 0.0 → −40 %, 1.0 → +40 %, 0.5 → exactly the min gap.
        assert_eq!(l.next_gap(0.0), Duration::from_millis(600));
        assert_eq!(l.next_gap(1.0), Duration::from_millis(1400));
        assert_eq!(l.next_gap(0.5), Duration::from_millis(1000));
    }

    #[test]
    fn spend_stops_at_the_hourly_or_daily_cap() {
        let s = BudgetSpend {
            used_hour: 15,
            used_day: 50,
        };
        assert!(!s.can_spend(15, 100), "hourly cap reached");
        assert!(BudgetSpend {
            used_hour: 10,
            used_day: 50
        }
        .can_spend(15, 100));
        assert!(
            !BudgetSpend {
                used_hour: 5,
                used_day: 100
            }
            .can_spend(15, 100),
            "daily cap reached"
        );
        assert_eq!(s.remaining_hour(15), 0);
        assert_eq!(
            BudgetSpend {
                used_hour: 5,
                used_day: 0
            }
            .remaining_hour(15),
            10
        );
    }
}
