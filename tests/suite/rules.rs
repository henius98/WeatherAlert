//! Tests for `domain::rules`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::rules::*;
use weather_alert::domain::tiers::Tier;

fn minute(off: u32, kv: &[(&str, f64)]) -> Minute {
  Minute { offset_min: off, fields: kv.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
}
fn rule(id: &str, group: Option<&str>, field: &str, op: Op, v: Value) -> Rule {
  Rule { id: id.into(), group: group.map(String::from), field: field.into(), op, value: v, lead_min: 20, enabled: true }
}

#[test]
fn shipped_seed_file_parses_and_means_what_the_docs_say() {
  // rules.seed.json is what gets written to KV; a typo there would deploy
  // a Worker with no rules and no alerts, silently.
  let seed = include_str!("../../rules.seed.json");
  let rules: Vec<Rule> = serde_json::from_str(seed).expect("rules.seed.json must parse");
  assert_eq!(rules.len(), 3);

  let solo: Vec<_> = rules.iter().filter(|r| r.group.is_none()).collect();
  assert_eq!(solo.len(), 1, "one independent rain rule");
  assert_eq!(solo[0].field, "rainIntensity");

  // The thunderstorm pair must be AND-ed: both, same minute, or nothing.
  let tl = vec![minute(5, &[("weatherCode", 8000.0), ("windGust", 10.0)])];
  assert!(first_match(&rules, &tl).is_none(), "code alone must not fire");
  let tl2 = vec![minute(5, &[("weatherCode", 8000.0), ("windGust", 55.0)])];
  assert!(first_match(&rules, &tl2).is_some(), "code + gust must fire");
}

#[test]
fn reports_onset_and_peak_not_just_the_drizzle_in_front() {
  // Drizzle at minute 2, violent downpour at minute 15.
  let rules = vec![rule("rain", None, "rainIntensity", Op::Gte, Value::Num(0.25))];
  let tl = vec![minute(2, &[("rainIntensity", 0.3)]), minute(15, &[("rainIntensity", 60.0)])];
  let h = evaluate(&rules, &tl).unwrap();
  assert_eq!(h.onset_min, 2, "rain still starts at minute 2");
  assert_eq!(h.peak_tier, Tier::Violent, "and the alert must say how bad it gets");
  assert_eq!(h.peak_min, 15);
}

#[test]
fn peak_ignores_minutes_beyond_the_lead_window() {
  let mut r = rule("rain", None, "rainIntensity", Op::Gte, Value::Num(0.25));
  r.lead_min = 10;
  let tl = vec![
    minute(3, &[("rainIntensity", 0.3)]),
    minute(25, &[("rainIntensity", 60.0)]), // outside a 10-min lead
  ];
  let h = evaluate(&[r], &tl).unwrap();
  assert_eq!(h.peak_tier, Tier::Light, "minute 25 is not our problem yet");
}

#[test]
fn a_rainless_rule_still_reports_something_is_coming() {
  let r = rule("gust", None, "windGust", Op::Gte, Value::Num(40.0));
  let tl = vec![minute(6, &[("windGust", 55.0)])];
  let h = evaluate(&[r], &tl).unwrap();
  assert_eq!(h.peak_tier, Tier::Light, "no rain field at all");
  assert_eq!(h.onset_min, 6);
}

#[test]
fn max_lead_min_sizes_the_fetch_window() {
  let mut a = rule("a", None, "rainIntensity", Op::Gte, Value::Num(1.0));
  a.lead_min = 20;
  let mut b = rule("b", None, "windGust", Op::Gte, Value::Num(40.0));
  b.lead_min = 45;
  assert_eq!(max_lead_min(&[a.clone(), b.clone()]), 45);
  b.enabled = false;
  assert_eq!(max_lead_min(&[a, b]), 20, "disabled rules must not widen it");
  assert_eq!(max_lead_min(&[]), 20);
}

#[test]
fn independent_rule_fires_alone() {
  let rules = vec![rule("rain", None, "rainIntensity", Op::Gte, Value::Num(2.5))];
  let tl = vec![minute(0, &[("rainIntensity", 0.1)]), minute(7, &[("rainIntensity", 3.0)])];
  assert_eq!(first_match(&rules, &tl).unwrap().offset_min, 7);
}

#[test]
fn and_group_requires_same_minute() {
  let rules = vec![rule("storm_a", Some("g1"), "weatherCode", Op::In, Value::Set(vec![8000.0])), rule("storm_b", Some("g1"), "windGust", Op::Gte, Value::Num(40.0))];
  // Conditions present but on DIFFERENT minutes -> must not fire.
  let split = vec![minute(4, &[("weatherCode", 8000.0), ("windGust", 10.0)]), minute(9, &[("weatherCode", 1000.0), ("windGust", 55.0)])];
  assert!(first_match(&rules, &split).is_none());

  let together = vec![minute(6, &[("weatherCode", 8000.0), ("windGust", 55.0)])];
  let m = first_match(&rules, &together).unwrap();
  assert_eq!(m.offset_min, 6);
  assert_eq!(m.rule_ids, vec!["storm_a", "storm_b"]);
}

#[test]
fn lead_min_bounds_the_window() {
  let mut r = rule("rain", None, "rainIntensity", Op::Gte, Value::Num(2.5));
  r.lead_min = 20;
  let tl = vec![minute(27, &[("rainIntensity", 9.0)])];
  assert!(first_match(&[r], &tl).is_none(), "27 min out exceeds a 20 min lead");
}

#[test]
fn disabled_rules_and_missing_fields_never_fire() {
  let mut off = rule("rain", None, "rainIntensity", Op::Gte, Value::Num(2.5));
  off.enabled = false;
  let tl = vec![minute(2, &[("rainIntensity", 99.0)])];
  assert!(first_match(&[off], &tl).is_none());

  let r = rule("gust", None, "windGust", Op::Gte, Value::Num(40.0));
  assert!(first_match(&[r], &tl).is_none(), "windGust absent from timeline");
}

#[test]
fn mismatched_op_and_value_shape_does_not_fire() {
  let bad = rule("bad", None, "rainIntensity", Op::Gte, Value::Set(vec![1.0]));
  let tl = vec![minute(1, &[("rainIntensity", 99.0)])];
  assert!(first_match(&[bad], &tl).is_none());
}

#[test]
fn two_and_groups_or_against_each_other() {
  let rules = vec![
    rule("a1", Some("g1"), "windGust", Op::Gte, Value::Num(90.0)),
    rule("a2", Some("g1"), "rainIntensity", Op::Gte, Value::Num(90.0)),
    rule("b1", Some("g2"), "rainIntensity", Op::Gte, Value::Num(2.5)),
    rule("b2", Some("g2"), "precipitationProbability", Op::Gte, Value::Num(60.0)),
  ];
  let tl = vec![minute(5, &[("rainIntensity", 4.0), ("precipitationProbability", 80.0), ("windGust", 5.0)])];
  let m = first_match(&rules, &tl).unwrap();
  assert_eq!(m.rule_ids, vec!["b1", "b2"], "g2 satisfied, g1 not");
}

#[test]
fn peak_cannot_precede_onset() {
  // Moderate rain below threshold at minute 1, threshold met at minute 15.
  let r = rule("heavy", None, "rainIntensity", Op::Gte, Value::Num(5.0));
  let tl = vec![minute(1, &[("rainIntensity", 4.0)]), minute(15, &[("rainIntensity", 5.0)])];
  let h = evaluate(&[r], &tl).unwrap();
  assert_eq!(h.onset_min, 15);
  assert_eq!(h.peak_min, 15, "peak must not be chosen from before onset");
}

#[test]
fn float_equality_and_in_set_tolerates_precision() {
  let eq_rule = rule("code_eq", None, "weatherCode", Op::Eq, Value::Num(8000.0));
  let in_rule = rule("code_in", None, "weatherCode", Op::In, Value::Set(vec![8000.0]));
  // Float with slight precision deviation
  let tl = vec![minute(2, &[("weatherCode", 8000.00001)])];
  assert!(first_match(&[eq_rule], &tl).is_some());
  assert!(first_match(&[in_rule], &tl).is_some());
}
