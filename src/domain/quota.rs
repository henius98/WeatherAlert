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
