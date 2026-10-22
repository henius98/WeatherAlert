//! Tests for `domain::errors`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::errors::*;

#[test]
fn auth_failures_always_notify() {
  assert_eq!(classify(Some(401), 0), Class::Auth);
  assert_eq!(classify(Some(403), 0), Class::Auth);
  assert!(Class::Auth.is_notifiable());
}

#[test]
fn a_single_429_is_absorbed_but_a_sustained_one_is_not() {
  assert_eq!(classify(Some(429), 0), Class::Transient);
  assert_eq!(classify(Some(429), 1), Class::Transient);
  assert_eq!(classify(Some(429), 2), Class::QuotaSustained);
}

#[test]
fn transient_5xx_stays_silent_until_it_is_sustained() {
  assert_eq!(classify(Some(503), 0), Class::Transient);
  assert_eq!(classify(Some(503), 4), Class::Transient);
  assert_eq!(classify(Some(503), 5), Class::SustainedFailure);
}

#[test]
fn a_six_hour_outage_sends_one_message_per_hour_not_one_per_tick() {
  let mut st = ErrState::default();
  let mut sent = 0;
  // 120 ticks at 3-minute spacing == 6 hours.
  for tick in 0..120i64 {
    let now = tick * 180;
    st.note_failure();
    let c = classify(Some(500), st.consecutive - 1);
    if st.should_send(c, now) {
      sent += 1;
    }
  }
  assert_eq!(sent, 6, "expected hourly, got {sent}");
}

#[test]
fn recovery_is_reported_once_then_the_throttle_resets() {
  let mut st = ErrState::default();
  for _ in 0..5 {
    st.note_failure();
  }
  assert!(st.note_success(), "recovery owed after sustained failure");
  assert!(!st.note_success(), "not owed twice");
  assert_eq!(st.consecutive, 0);
}

#[test]
fn a_clean_tick_costs_no_kv_write() {
  let mut st = ErrState::default();
  // Healthy tick: nothing failed, nothing sent, nothing to persist.
  assert!(!st.note_success());
  assert!(!st.dirty, "a healthy tick must not dirty the error state");
  // 280 healthy ticks/day must stay at zero writes.
  for _ in 0..280 {
    st.note_success();
  }
  assert!(!st.dirty);
}

#[test]
fn failures_and_sends_mark_state_dirty() {
  let mut st = ErrState::default();
  st.note_failure();
  assert!(st.dirty);

  let mut st2 = ErrState::default();
  assert!(st2.should_send(Class::Auth, 0));
  assert!(st2.dirty, "a sent message updates the throttle and must persist");

  let mut st3 = ErrState::default();
  st3.note_failure();
  st3.dirty = false;
  assert!(!st3.note_success(), "recovery not owed below the threshold");
  assert!(st3.dirty, "clearing a failure streak must persist");
}

#[test]
fn transient_class_never_sends_even_on_first_occurrence() {
  let mut st = ErrState::default();
  assert!(!st.should_send(Class::Transient, 0));
}

#[test]
fn storage_class_notifies_and_throttles() {
  let mut st = ErrState::default();
  assert!(Class::Storage.is_notifiable());
  assert_eq!(Class::Storage.key(), "storage");
  assert!(st.should_send(Class::Storage, 0));
  assert!(!st.should_send(Class::Storage, 1800), "must throttle within the hour");
  assert!(st.should_send(Class::Storage, 3601), "must send again after throttle period");
}
