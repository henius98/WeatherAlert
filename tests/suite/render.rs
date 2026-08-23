//! Tests for `domain::render`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::render::*;
use weather_alert::domain::state::Action;
use weather_alert::domain::tiers::Tier;

#[test]
fn open_and_escalate_read_differently() {
    let ids = vec!["rain_mod".to_string()];
    let o = alert(
        &Action::Open { peak_tier: Tier::Moderate, onset_min: 18, peak_min: 18 },
        "Sepang", &ids,
    ).unwrap();
    assert!(o.contains("Moderate rain in ~18 min"), "got {o}");
    assert!(o.contains("Sepang") && o.contains("rain_mod"));

    let e = alert(
        &Action::Escalate { from: Tier::Moderate, to: Tier::Heavy, onset_min: 6, peak_min: 6 },
        "Sepang", &ids,
    ).unwrap();
    assert!(e.contains("moderate \u{2192} heavy"), "got {e}");
}

#[test]
fn a_worse_storm_behind_drizzle_is_stated_in_the_first_alert() {
    let o = alert(
        &Action::Open { peak_tier: Tier::Violent, onset_min: 2, peak_min: 15 },
        "Sepang", &[],
    ).unwrap();
    assert!(o.contains("in ~2 min"), "onset must survive: {o}");
    assert!(o.contains("violent"), "peak must be stated: {o}");
    assert!(o.contains("violent by ~15 min"), "got {o}");
    assert!(!o.starts_with("\u{1F327} Light"), "must not lead with the drizzle");
}

#[test]
fn nothing_renders_no_message() {
    assert!(alert(&Action::Nothing, "Sepang", &[]).is_none());
}

#[test]
fn eta_zero_reads_as_now() {
    let o = alert(
        &Action::Open { peak_tier: Tier::Heavy, onset_min: 0, peak_min: 0 },
        "Sepang", &[],
    ).unwrap();
    assert!(o.contains("rain now"), "got {o}");
}

#[test]
fn report_states_the_quiet_case_plainly() {
    let r = report("Sepang", Tier::None, None, &[], &[], false, true);
    assert!(r.contains("no rain expected"));
    assert!(!r.contains("MET forecast"));
    assert!(!r.contains("cached"));
}

#[test]
fn report_marks_a_cached_result() {
    let r = report("Sepang", Tier::None, None, &[], &[], true, true);
    assert!(r.contains("cached"));
}

#[test]
fn missing_data_never_renders_as_no_rain() {
    let none = report("Sepang", Tier::None, None, &[], &[], true, false);
    assert!(none.contains("NO DATA"), "got {none}");
    assert!(!none.contains("no rain expected"), "a false all-clear: {none}");
}

#[test]
fn every_message_fits_the_telegram_cap() {
    let warns: Vec<String> = (0..400).map(|i| format!("warning number {i} with padding text")).collect();
    let r = report("Sepang", Tier::Heavy, Some(3), &[], &warns, false, true);
    assert!(r.chars().count() <= TG_LIMIT, "got {}", r.chars().count());
    assert!(r.ends_with("[truncated]"));
}

#[test]
fn truncate_is_a_noop_below_the_limit() {
    assert_eq!(truncate("short".to_string(), 4096), "short");
}
