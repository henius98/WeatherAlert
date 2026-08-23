//! Tests for `domain::timeutil`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::timeutil::*;

#[test]
fn parses_met_timestamp_as_myt_not_utc() {
    // The live feed reported issued=20:25 MYT while UTC was 13:00.
    let e = parse_naive_myt("2026-08-22T20:25:00").unwrap();
    assert_eq!(myt_hour(e), 20);
    assert_eq!(e, 1787401500); // 12:25 UTC
}

#[test]
fn rejects_malformed_and_null_like_input() {
    for s in ["", "None", "2026-08-22", "not-a-date-at-all!!"] {
        assert!(parse_naive_myt(s).is_none(), "should reject {s:?}");
    }
}

#[test]
fn parses_tomorrow_io_utc_with_z_suffix() {
    let a = parse_naive_utc("2026-08-22T13:00:00Z").unwrap();
    let b = parse_naive_utc("2026-08-22T13:00:00").unwrap();
    assert_eq!(a, b, "trailing Z must not shift the value");
    assert_eq!(myt_hour(a), 21, "13:00Z is 21:00 MYT");
    assert!(parse_naive_utc("2026-08-22T13:00:00.500Z").is_some());
}

#[test]
fn ymd_round_trips_and_respects_the_myt_date_boundary() {
    for iso in ["2026-08-22", "2026-01-01", "2026-12-31", "2024-02-29", "2026-03-01"] {
        let e = parse_naive_myt(&format!("{iso}T12:00:00")).unwrap();
        assert_eq!(ymd_myt(e), iso);
    }
    // 23:30 MYT on the 22nd is 15:30 UTC the same day; the MYT date must win.
    let late = parse_naive_myt("2026-08-22T23:30:00").unwrap();
    assert_eq!(ymd_myt(late), "2026-08-22");
    // 00:30 MYT on the 23rd is 16:30 UTC on the 22nd.
    let early = parse_naive_myt("2026-08-23T00:30:00").unwrap();
    assert_eq!(ymd_myt(early), "2026-08-23");
}

#[test]
fn active_window_respects_myt_offset() {
    let nine_am = parse_naive_myt("2026-08-22T09:00:00").unwrap();
    let eleven_pm = parse_naive_myt("2026-08-22T23:00:00").unwrap();
    let three_am = parse_naive_myt("2026-08-22T03:00:00").unwrap();
    assert!(in_active_window(nine_am, 9, 23));
    assert!(!in_active_window(eleven_pm, 9, 23)); // end is exclusive
    assert!(!in_active_window(three_am, 9, 23));
}
