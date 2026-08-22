//! Manual-report budget. The rain cron consumes 20 of Tomorrow.io's 25
//! calls/hour; manual reports draw on what is left, so a user hammering /now
//! can never push the cron path into a 429.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Quota {
    #[serde(default)]
    pub hour_bucket: i64,
    #[serde(default)]
    pub manual_calls: u32,
}

impl Quota {
    /// True if a live fetch is allowed. Resets when the wall-clock hour rolls.
    pub fn try_spend(&mut self, now: i64, cap: u32) -> bool {
        let bucket = now / 3600;
        if bucket != self.hour_bucket {
            self.hour_bucket = bucket;
            self.manual_calls = 0;
        }
        if self.manual_calls >= cap {
            return false;
        }
        self.manual_calls += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spends_up_to_the_cap_then_refuses() {
        let mut q = Quota::default();
        for i in 0..4 {
            assert!(q.try_spend(7200, 4), "call {i} should be allowed");
        }
        assert!(!q.try_spend(7200, 4), "5th call must fall back to cache");
        assert_eq!(q.manual_calls, 4);
    }

    #[test]
    fn resets_on_the_next_hour() {
        let mut q = Quota::default();
        for _ in 0..4 {
            q.try_spend(7200, 4);
        }
        assert!(!q.try_spend(7200, 4));
        assert!(q.try_spend(7200 + 3600, 4), "new hour, fresh budget");
        assert_eq!(q.manual_calls, 1);
    }

    #[test]
    fn never_exceeds_the_five_calls_the_cron_leaves_spare() {
        // 20 cron calls/hour + cap 4 == 24, inside Tomorrow.io's 25/hour.
        let mut q = Quota::default();
        let spent = (0..100).filter(|_| q.try_spend(7200, 4)).count();
        assert_eq!(spent, 4);
        assert!(20 + spent <= 25);
    }
}
