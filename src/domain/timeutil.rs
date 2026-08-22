//! Zero-dependency epoch<->civil conversion. Avoids pulling chrono into the
//! 3 MB gzipped Worker bundle. Algorithm: Howard Hinnant's days_from_civil.

pub const MYT_OFFSET_SECS: i64 = 8 * 3600;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses `YYYY-MM-DDTHH:MM:SS`, tolerating a trailing `Z` and fractional
/// seconds. Returns seconds since epoch treating the input as UTC.
pub fn parse_naive_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if b[10] != b'T' && b[10] != b' ' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec)
}

/// data.gov.my emits timestamps with NO offset and they are Malaysia time, so we
/// attach +08:00 explicitly. Parsing these as UTC puts every warning 8 hours out.
pub fn parse_naive_myt(s: &str) -> Option<i64> {
    parse_naive_utc(s).map(|t| t - MYT_OFFSET_SECS)
}

/// Hour-of-day 0..=23 in Malaysia time, for the quiet-hours window.
pub fn myt_hour(epoch: i64) -> i64 {
    (epoch + MYT_OFFSET_SECS).rem_euclid(86_400) / 3600
}

/// Civil date `YYYY-MM-DD` in Malaysia time, for the forecast date filter.
pub fn ymd_myt(epoch: i64) -> String {
    let days = (epoch + MYT_OFFSET_SECS).div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Inclusive-start, exclusive-end active window in MYT.
pub fn in_active_window(epoch: i64, start_h: i64, end_h: i64) -> bool {
    let h = myt_hour(epoch);
    if start_h <= end_h { h >= start_h && h < end_h } else { h >= start_h || h < end_h }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
