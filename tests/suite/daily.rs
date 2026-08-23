//! Tests for `domain::daily`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::daily::*;
use weather_alert::domain::tiers::Tier;

#[test]
fn tally_rolls_when_the_date_changes() {
    let mut l = DailyLog::default();
    l.record("2026-08-21", Tier::Light, false);
    l.record("2026-08-21", Tier::Heavy, false);
    assert_eq!(l.total_opens(), 2);

    l.record("2026-08-22", Tier::Light, false);
    assert_eq!(l.date, "2026-08-22");
    assert_eq!(l.total_opens(), 1, "yesterday's counts must not carry over");
}

#[test]
fn escalations_are_counted_separately_from_opens() {
    let mut l = DailyLog::default();
    l.record("2026-08-22", Tier::Light, false);
    l.record("2026-08-22", Tier::Violent, true);
    assert_eq!(l.total_opens(), 1);
    assert_eq!(l.escalations, 1);
    let line = l.tuning_line();
    assert!(line.contains("1 rain alert "), "got {line}");
    assert!(line.contains("light 1"), "got {line}");
    assert!(line.contains("1 escalation."), "got {line}");
}

#[test]
fn a_quiet_day_says_so_plainly() {
    assert_eq!(DailyLog::default().tuning_line(), "No rain alerts fired.");
}

#[test]
fn summary_states_liveness_even_with_no_data_at_all() {
    let out = morning_summary("Sepang", "2026-08-22", &[], &[], &DailyLog::default(), (9, 23));
    assert!(out.contains("Good morning"));
    assert!(out.contains("MET forecast unavailable"));
    assert!(out.contains("No rain alerts fired."));
    assert!(
        out.contains("If this message stops arriving, the monitor is down."),
        "the liveness contract must always be stated: {out}"
    );
    assert!(out.contains("09:00-23:00 MYT"), "got {out}");
}

#[test]
fn summary_carries_forecast_and_warnings_when_present() {
    let body = r#"[{"location":{"location_id":"Ds064","location_name":"Sepang"},
      "date":"2026-08-22","summary_forecast":"Ribut petir di beberapa tempat",
      "min_temp":24,"max_temp":33}]"#;
    let days = weather_alert::domain::forecast::parse(body, "Ds064", 2).unwrap();
    let mut log = DailyLog::default();
    log.record("2026-08-21", Tier::Moderate, false);
    let out = morning_summary(
        "Sepang", "2026-08-22", &days,
        &["Thunderstorms Warning".to_string()], &log, (9, 23),
    );
    assert!(out.contains("Thunderstorms in some areas"), "translated: {out}");
    assert!(out.contains("24-33C"));
    assert!(out.contains("Thunderstorms Warning"));
    assert!(out.contains("moderate 1"));
}
