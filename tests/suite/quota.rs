//! Tests for `domain::quota`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::quota::*;

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
