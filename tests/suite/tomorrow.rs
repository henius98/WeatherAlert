//! Tests for `domain::tomorrow`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::tomorrow::*;
use weather_alert::domain::rules::{first_match, Minute, Op, Rule, Value};
use weather_alert::domain::tiers::Tier;

const NOW: i64 = 1787403600; // 2026-08-22T13:00:00Z

fn body() -> String {
    r#"{"data":{"timelines":[{"timestep":"1m","intervals":[
      {"startTime":"2026-08-22T13:00:00Z","values":{"rainIntensity":0,"windGust":5.1,"weatherCode":1000}},
      {"startTime":"2026-08-22T13:07:00Z","values":{"rainIntensity":3.4,"windGust":12.0,"weatherCode":4001}},
      {"startTime":"2026-08-22T13:19:00Z","values":{"rainIntensity":11.2,"windGust":48.0,"weatherCode":8000}}
    ]}]}}"#
        .to_string()
}

#[test]
fn maps_intervals_to_minute_offsets() {
    let tl = parse_timeline(&body(), NOW).unwrap();
    assert_eq!(tl.len(), 3);
    assert_eq!(tl.iter().map(|m| m.offset_min).collect::<Vec<_>>(), vec![0, 7, 19]);
    assert_eq!(tl[2].fields["rainIntensity"], 11.2);
    assert_eq!(tl[2].fields["weatherCode"], 8000.0);
}

#[test]
fn drops_intervals_already_in_the_past() {
    let tl = parse_timeline(&body(), NOW + 600).unwrap();
    assert_eq!(tl.iter().map(|m| m.offset_min).collect::<Vec<_>>(), vec![9]);
}

#[test]
fn null_and_non_numeric_values_are_dropped_not_zeroed() {
    let b = r#"{"data":{"timelines":[{"intervals":[
      {"startTime":"2026-08-22T13:00:00Z","values":{"rainIntensity":null,"cloudBase":"n/a","windGust":9.0}}
    ]}]}}"#;
    let tl = parse_timeline(b, NOW).unwrap();
    assert_eq!(tl.len(), 1);
    assert!(!tl[0].fields.contains_key("rainIntensity"), "null must not become 0.0");
    assert!(!tl[0].fields.contains_key("cloudBase"));
    assert_eq!(tl[0].fields["windGust"], 9.0);
}

#[test]
fn empty_and_malformed_payloads_are_handled() {
    assert!(parse_timeline(r#"{"data":{"timelines":[]}}"#, NOW).unwrap().is_empty());
    assert!(parse_timeline(r#"{"data":{}}"#, NOW).unwrap().is_empty());
    assert!(parse_timeline("not json", NOW).is_err());
}

#[test]
fn field_whitelist_catches_the_typo_that_would_break_every_fetch() {
    assert!(is_valid_field("rainIntensity"));
    assert!(is_valid_field("weatherCode"));
    assert!(!is_valid_field("rainIntesity"), "the typo must be rejected");
    assert!(!is_valid_field(""));
    assert!(!is_valid_field("rainintensity"), "case matters to the API");
}

#[test]
fn window_sizes_itself_to_the_widest_rule() {
    let mk = |lead: u32| Rule {
        id: "r".into(), group: None, field: "rainIntensity".into(),
        op: Op::Gte, value: Value::Num(1.0), lead_min: lead, enabled: true,
    };
    assert_eq!(window_minutes(&[mk(20)]), 30, "floor is 30");
    assert_eq!(window_minutes(&[mk(45)]), 50, "45 + 5 buffer");
    assert_eq!(window_minutes(&[mk(600)]), 120, "capped");
    assert_eq!(window_minutes(&[]), 30);
}

#[test]
fn required_fields_always_include_rain_intensity() {
    let rules = vec![Rule {
        id: "g".into(), group: None, field: "windGust".into(),
        op: Op::Gte, value: Value::Num(40.0), lead_min: 20, enabled: true,
    }];
    assert_eq!(required_fields(&rules), vec!["rainIntensity", "windGust"]);
    assert_eq!(required_fields(&[]), vec!["rainIntensity"]);
}

#[test]
fn end_to_end_rule_match_against_a_real_payload() {
    use weather_alert::domain::rules::first_match;
    use weather_alert::domain::tiers::Tier;
    let rules = vec![
        Rule { id: "a".into(), group: Some("g1".into()), field: "weatherCode".into(),
               op: Op::In, value: Value::Set(vec![8000.0]), lead_min: 20, enabled: true },
        Rule { id: "b".into(), group: Some("g1".into()), field: "windGust".into(),
               op: Op::Gte, value: Value::Num(40.0), lead_min: 20, enabled: true },
    ];
    let tl = parse_timeline(&body(), NOW).unwrap();
    let m = first_match(&rules, &tl).unwrap();
    assert_eq!(m.offset_min, 19);
    assert_eq!(Tier::from_rain_intensity(tl[2].fields["rainIntensity"]), Tier::Heavy);
}
