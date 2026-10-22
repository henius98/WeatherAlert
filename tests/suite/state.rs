//! Tests for `domain::state`.
//! Moved out of the source file so the whole suite lives in one place.

use std::collections::BTreeMap;
use weather_alert::domain::daily::DailyLog;
use weather_alert::domain::rules::{Hit, Minute, Op, Rule, Value};
use weather_alert::domain::state::*;
use weather_alert::domain::tiers::Tier;
const MIN: i64 = 60;

fn hit(peak_tier: Tier, onset_min: u32, peak_min: u32) -> Hit {
  Hit { peak_tier, onset_min, peak_min, rule_ids: vec!["r1".into()] }
}

#[test]
fn alerts_once_then_stays_silent_across_ticks() {
  let mut st = EventState::default();
  let h = hit(Tier::Moderate, 18, 18);
  let s = step(&mut st, Some(&h), 0, 30, true);
  assert_eq!(s.action, Action::Open { peak_tier: Tier::Moderate, onset_min: 18, peak_min: 18, rule_ids: vec!["r1".into()] });
  assert!(s.write_needed);
  assert_eq!(st.rule_ids, vec!["r1".to_string()]);

  // Seven consecutive 3-minute ticks still matching -> no further alerts.
  for i in 1..=7 {
    let h = hit(Tier::Moderate, 18 - i as u32, 18 - i as u32);
    let s = step(&mut st, Some(&h), i * 3 * MIN, 30, true);
    assert_eq!(s.action, Action::Nothing, "tick {i} must not re-alert");
    assert!(!s.write_needed, "tick {i} must not write to KV");
  }
}

#[test]
fn re_alerts_on_escalation_only_upward() {
  let mut st = EventState::default();
  let h1 = hit(Tier::Light, 20, 20);
  step(&mut st, Some(&h1), 0, 30, true);
  let h2 = Hit { peak_tier: Tier::Heavy, onset_min: 8, peak_min: 12, rule_ids: vec!["r2".into()] };
  let up = step(&mut st, Some(&h2), 3 * MIN, 30, true);
  assert_eq!(up.action, Action::Escalate { from: Tier::Light, to: Tier::Heavy, onset_min: 8, peak_min: 12, rule_ids: vec!["r2".into()] });
  assert_eq!(st.rule_ids, vec!["r2".to_string()]);

  let h3 = hit(Tier::Light, 2, 2);
  let down = step(&mut st, Some(&h3), 6 * MIN, 30, true);
  assert_eq!(down.action, Action::Nothing, "de-escalation is silent");
  assert_eq!(st.tier, Tier::Heavy as u8, "peak tier is retained");
}

#[test]
fn closes_only_after_the_quiet_gap() {
  let mut st = EventState::default();
  let h = hit(Tier::Moderate, 10, 10);
  step(&mut st, Some(&h), 0, 30, true);
  step(&mut st, None, 0, 30, true);
  assert_eq!(step(&mut st, None, 20 * MIN, 30, true).action, Action::Nothing);
  assert!(st.open, "20 min of quiet is inside a 30 min close window");

  let closed = step(&mut st, None, 30 * MIN, 30, true);
  assert_eq!(closed.action, Action::Close { duration_min: 0 });
  assert!(!st.open);
}

#[test]
fn all_clear_disabled_closes_silently_but_still_persists() {
  let mut st = EventState::default();
  let h = hit(Tier::Moderate, 10, 10);
  step(&mut st, Some(&h), 0, 30, false);
  step(&mut st, None, 0, 30, false);
  let closed = step(&mut st, None, 31 * MIN, 30, false);
  assert_eq!(closed.action, Action::Nothing);
  assert!(closed.write_needed, "state must still be cleared in KV");
}

#[test]
fn shower_longer_than_close_gap_does_not_close_on_first_quiet_tick() {
  let mut st = EventState::default();
  let h = hit(Tier::Moderate, 10, 10);
  // Event opens at t = 0
  step(&mut st, Some(&h), 0, 30, true);
  // Rain continues for 45 minutes without KV writes
  for i in 1..=15 {
    let s = step(&mut st, Some(&h), i * 3 * MIN, 30, true);
    assert!(!s.write_needed);
  }
  // Simulate KV restore: in KV, st had opened_at = 0, last_match_at = 0, quiet_since = None
  let mut restored = EventState { open: true, tier: st.tier, opened_at: 0, last_match_at: 0, rule_ids: st.rule_ids.clone(), quiet_since: None };
  // At t = 48 min, rain stops.
  let first_dry = step(&mut restored, None, 48 * MIN, 30, true);
  assert_eq!(first_dry.action, Action::Nothing, "must not close on first dry tick");
  assert!(first_dry.write_needed, "must write to KV to persist quiet_since and last_match_at");
  assert!(restored.open);
  assert_eq!(restored.last_match_at, 48 * MIN);

  // At t = 68 min (20 min of quiet), still open
  assert_eq!(step(&mut restored, None, 68 * MIN, 30, true).action, Action::Nothing);
  assert!(restored.open);

  // At t = 78 min (30 min of quiet), closes!
  let closed = step(&mut restored, None, 78 * MIN, 30, true);
  assert_eq!(closed.action, Action::Close { duration_min: 48 });
  assert!(!restored.open);
  assert!(closed.write_needed);
}

#[test]
fn quiet_ticks_with_no_open_event_never_write() {
  let mut st = EventState::default();
  for t in 0..50 {
    let s = step(&mut st, None, t * 3 * MIN, 30, true);
    assert!(!s.write_needed);
  }
}

#[test]
fn evaluate_tick_coordinates_event_alert_and_daily_tally() {
  let r = Rule {
    id: "r1".into(),
    group: None,
    field: "rainIntensity".into(),
    op: Op::Gte,
    value: Value::Num(2.5),
    lead_min: 30,
    enabled: true,
  };
  let mut fields = BTreeMap::new();
  fields.insert("rainIntensity".into(), 4.0); // Moderate tier
  let timeline = vec![Minute { offset_min: 10, fields }];

  let mut ev = EventState::default();
  let mut daily = DailyLog::default();

  // Tick 1: Opens event -> alert generated, write_event true, write_daily true
  let plan1 = evaluate_tick(&[r.clone()], &timeline, &mut ev, &mut daily, 1000, 30, true, "Sepang");
  assert!(plan1.alert_message.is_some());
  assert!(plan1.write_event);
  assert!(plan1.write_daily);
  assert_eq!(daily.opens.get("moderate"), Some(&1));

  // Tick 2: Same conditions -> silent, no writes
  let plan2 = evaluate_tick(&[r.clone()], &timeline, &mut ev, &mut daily, 1180, 30, true, "Sepang");
  assert!(plan2.alert_message.is_none());
  assert!(!plan2.write_event);
  assert!(!plan2.write_daily);
  assert_eq!(daily.opens.get("moderate"), Some(&1));

  // Tick 3: Intensity climbs to Heavy (15.0) -> Escalates!
  let mut fields_heavy = BTreeMap::new();
  fields_heavy.insert("rainIntensity".into(), 15.0);
  let timeline_heavy = vec![Minute { offset_min: 5, fields: fields_heavy }];

  let plan3 = evaluate_tick(&[r], &timeline_heavy, &mut ev, &mut daily, 1360, 30, true, "Sepang");
  assert!(plan3.alert_message.is_some());
  assert!(plan3.write_event);
  assert!(plan3.write_daily);
  assert_eq!(daily.escalations, 1);
}
