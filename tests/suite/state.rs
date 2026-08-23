//! Tests for `domain::state`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::state::*;
use weather_alert::domain::tiers::Tier;
const MIN: i64 = 60;

#[test]
fn alerts_once_then_stays_silent_across_ticks() {
    let mut st = EventState::default();
    let s = step(&mut st, Some((Tier::Moderate, 18, 18)), 0, 30, true);
    assert_eq!(s.action, Action::Open { peak_tier: Tier::Moderate, onset_min: 18, peak_min: 18 });
    assert!(s.write_needed);

    // Seven consecutive 3-minute ticks still matching -> no further alerts.
    for i in 1..=7 {
        let s = step(&mut st, Some((Tier::Moderate, 18 - i as u32, 18 - i as u32)), i * 3 * MIN, 30, true);
        assert_eq!(s.action, Action::Nothing, "tick {i} must not re-alert");
        assert!(!s.write_needed, "tick {i} must not write to KV");
    }
}

#[test]
fn re_alerts_on_escalation_only_upward() {
    let mut st = EventState::default();
    step(&mut st, Some((Tier::Light, 20, 20)), 0, 30, true);
    let up = step(&mut st, Some((Tier::Heavy, 8, 12)), 3 * MIN, 30, true);
    assert_eq!(up.action, Action::Escalate { from: Tier::Light, to: Tier::Heavy, onset_min: 8, peak_min: 12 });

    let down = step(&mut st, Some((Tier::Light, 2, 2)), 6 * MIN, 30, true);
    assert_eq!(down.action, Action::Nothing, "de-escalation is silent");
    assert_eq!(st.tier, Tier::Heavy as u8, "peak tier is retained");
}

#[test]
fn closes_only_after_the_quiet_gap() {
    let mut st = EventState::default();
    step(&mut st, Some((Tier::Moderate, 10, 10)), 0, 30, true);
    assert_eq!(step(&mut st, None, 20 * MIN, 30, true).action, Action::Nothing);
    assert!(st.open, "20 min of quiet is inside a 30 min close window");

    let closed = step(&mut st, None, 30 * MIN, 30, true);
    assert_eq!(closed.action, Action::Close { duration_min: 0 });
    assert!(!st.open);
}

#[test]
fn all_clear_disabled_closes_silently_but_still_persists() {
    let mut st = EventState::default();
    step(&mut st, Some((Tier::Moderate, 10, 10)), 0, 30, false);
    let closed = step(&mut st, None, 31 * MIN, 30, false);
    assert_eq!(closed.action, Action::Nothing);
    assert!(closed.write_needed, "state must still be cleared in KV");
}

#[test]
fn quiet_ticks_with_no_open_event_never_write() {
    let mut st = EventState::default();
    for t in 0..50 {
        let s = step(&mut st, None, t * 3 * MIN, 30, true);
        assert!(!s.write_needed);
    }
}
