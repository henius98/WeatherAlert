//! Tests for `domain::met`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::met::*;
use weather_alert::domain::timeutil::parse_naive_myt;

fn w(issued: &str, title: &str, from: Option<&str>, to: Option<&str>) -> Warning {
  Warning {
    warning_issue: WarningIssue { issued: Some(issued.into()), title_en: Some(title.into()) },
    valid_from: from.map(String::from),
    valid_to: to.map(String::from),
    heading_en: Some(title.to_uppercase()),
    text_en: Some("Thunderstorms over Sepang.".into()),
    instruction_en: None,
  }
}

#[test]
fn same_issued_and_title_but_different_window_are_distinct() {
  // Exactly the live case: 3 records, issued 20:25, identical titles.
  let a = w("2026-08-22T20:25:00", "Strong Winds", Some("2026-08-21T00:00:00"), Some("2026-08-27T00:00:00"));
  let b = w("2026-08-22T20:25:00", "Strong Winds", Some("2026-08-23T00:00:00"), Some("2026-08-27T00:00:00"));
  let c = w("2026-08-22T20:25:00", "Strong Winds", Some("2026-08-22T20:00:00"), Some("2026-08-23T01:00:00"));
  assert_ne!(a.dedup_key(), b.dedup_key());
  assert_ne!(b.dedup_key(), c.dedup_key());

  let now = parse_naive_myt("2026-08-22T21:00:00").unwrap();
  let (sel, st) = select_observe(&[a, b, c], &MetState::default(), now);
  assert_eq!(sel.len(), 3, "all three must be forwarded, not collapsed to one");
  assert_eq!(st.seen.len(), 3);
}

#[test]
fn null_validity_does_not_panic_and_is_not_expired() {
  let mut n = w("2026-08-22T17:30:00", "Thunderstorms Warning", None, None);
  n.heading_en = Some("THUNDERSTORMS WARNING".into());
  assert!(!n.is_expired(parse_naive_myt("2026-08-22T21:00:00").unwrap()));
}

#[test]
fn no_advisory_sentinel_is_dropped() {
  let mut s = w("2026-08-22T17:30:00", "No Advisory", None, None);
  s.heading_en = Some("No Advisory".into());
  assert!(s.is_no_advisory());
  let now = parse_naive_myt("2026-08-22T21:00:00").unwrap();
  assert!(select_observe(&[s], &MetState::default(), now).0.is_empty());
}

#[test]
fn expired_warnings_are_dropped() {
  let old = w("2026-08-22T19:30:00", "Thunderstorms Warning", Some("2026-08-22T19:00:00"), Some("2026-08-22T22:00:00"));
  let now = parse_naive_myt("2026-08-23T06:00:00").unwrap();
  assert!(old.is_expired(now));
  assert!(select_observe(&[old], &MetState::default(), now).0.is_empty());
}

#[test]
fn dedup_suppresses_the_same_warning_on_the_next_poll() {
  let one = w("2026-08-22T19:30:00", "Thunderstorms Warning", Some("2026-08-22T19:00:00"), Some("2026-08-22T22:00:00"));
  let now = parse_naive_myt("2026-08-22T20:00:00").unwrap();
  let (sel, st) = select_observe(&[one.clone()], &MetState::default(), now);
  assert_eq!(sel.len(), 1);
  let (again, _) = select_observe(&[one], &st, now);
  assert!(again.is_empty(), "a 10-min poll must not re-alert for six days");
}

#[test]
fn shipping_block_is_stripped_from_land_text() {
  let mut m = w("2026-08-22T20:25:00", "Strong Winds", None, None);
  m.text_en = Some(format!("SECTION A: land warning for Sepang. {SHIPPING_MARKER} waters of Phuket and Layang-Layang."));
  assert!(m.land_text().contains("Sepang"));
  assert!(!m.land_text().contains("Phuket"), "marine noise must not reach a land user");
}

#[test]
fn relevance_matching_is_case_insensitive() {
  let m = w("2026-08-22T20:25:00", "Thunderstorms Warning", None, None);
  assert!(m.mentions_any(&["sepang"]));
  assert!(!m.mentions_any(&["Kelantan"]));
  assert!(!m.mentions_any(&[""]), "empty needle must never match");
}

#[test]
fn filter_requires_both_severity_and_place_when_both_configured() {
  let mut m = w("2026-08-22T20:25:00", "Continuous Rain", None, None);
  m.heading_en = Some("KATEGORI KETIGA WARNING ON CONTINUOUS RAIN".into());
  m.text_en = Some("Heavy rain expected over Selangor and Sepang.".into());

  assert!(passes_filter(&m, &["kategori ketiga", "merah"], &["Selangor"]));
  assert!(!passes_filter(&m, &["kategori ketiga"], &["Kelantan"]), "wrong place");
  assert!(!passes_filter(&m, &["amaran bahaya"], &["Selangor"]), "wrong severity");
}

#[test]
fn an_unconfigured_axis_does_not_silence_the_warning() {
  let mut m = w("2026-08-22T20:25:00", "Continuous Rain", None, None);
  m.text_en = Some("Heavy rain over Selangor.".into());
  // A half-configured filter must degrade to forwarding, never to silence.
  assert!(passes_filter(&m, &[], &["Selangor"]));
  assert!(!passes_filter(&m, &["merah"], &[]), "severity still gates");
  assert!(passes_filter(&m, &[], &[]), "nothing configured => forward");
}

#[test]
fn place_ids_never_match_prose_only_names_do() {
  let mut m = w("2026-08-22T20:25:00", "Continuous Rain", None, None);
  m.text_en = Some("Heavy rain over Selangor and Sepang.".into());
  assert!(!passes_filter(&m, &[], &["St002"]), "an id must not match");
  assert!(passes_filter(&m, &[], &["Sepang"]));
}

#[test]
fn severity_match_ignores_the_shipping_block() {
  let mut m = w("2026-08-22T20:25:00", "Strong Winds", None, None);
  m.heading_en = Some("FIRST CATEGORY".into());
  m.text_en = Some(format!("SECTION A: winds over Terengganu. {SHIPPING_MARKER} KATEGORI KETIGA over Phuket."));
  assert!(!passes_filter(&m, &["kategori ketiga"], &[]), "a severity word inside the shipping block must not qualify the warning");
}

#[test]
fn observe_summary_is_short_but_keeps_the_severity_vocabulary() {
  let mut m = w("2026-08-22T20:25:00", "Strong Winds and Rough Seas Warning", Some("2026-08-21T00:00:00"), Some("2026-08-27T00:00:00"));
  m.heading_en = Some("FIRST CATEGORY WARNING ON STRONG WINDS AND ROUGH SEAS".into());
  m.text_en = Some("x".repeat(1549)); // the real payload size
  let out = format_summary(&m);
  assert!(out.contains("FIRST CATEGORY"), "severity wording must survive");
  assert!(out.contains("Strong Winds"));
  assert!(!out.contains("xxxx"), "1.5 KB of prose must not be forwarded");
  assert!(out.chars().count() < 200, "got {}", out.chars().count());
}

#[test]
fn formatting_respects_the_telegram_cap() {
  let mut m = w("2026-08-22T20:25:00", "Strong Winds", None, None);
  m.text_en = Some("x".repeat(9000));
  let out = format_observe(&m, 4096);
  assert!(out.chars().count() <= 4096, "got {}", out.chars().count());
  assert!(out.ends_with("[truncated]"));
}

#[test]
fn active_titles_drops_no_advisory_and_expired() {
  let json = r#"[
    {
      "warning_issue": {"issued": "2026-08-22T17:30:00", "title_en": "No Advisory"},
      "heading_en": "No Advisory"
    },
    {
      "warning_issue": {"issued": "2026-08-22T17:30:00", "title_en": "Past Warning"},
      "valid_to": "2026-08-22T18:00:00"
    },
    {
      "warning_issue": {"issued": "2026-08-22T20:25:00", "title_en": "Live Warning"},
      "valid_to": "2026-08-22T23:00:00"
    }
  ]"#;
  let now = parse_naive_myt("2026-08-22T21:00:00").unwrap();
  let titles = active_titles(json, now);
  assert_eq!(titles, vec!["Live Warning"]);
}

#[test]
fn format_observe_omits_observe_tag_in_filter_mode() {
  let m = w("2026-08-22T20:25:00", "Strong Winds", None, None);
  let out = format_observe(&m, 4096);
  assert!(!out.contains("[OBSERVE]"), "filter-mode warnings must not be tagged [OBSERVE]");
}

#[test]
fn active_titles_deduplicates_identical_titles() {
  let json = r#"[
    {
      "warning_issue": {"issued": "2026-08-22T20:25:00", "title_en": "Strong Winds"},
      "valid_to": "2026-08-22T23:00:00"
    },
    {
      "warning_issue": {"issued": "2026-08-22T20:25:00", "title_en": "Strong Winds"},
      "valid_to": "2026-08-23T01:00:00"
    }
  ]"#;
  let now = parse_naive_myt("2026-08-22T21:00:00").unwrap();
  let titles = active_titles(json, now);
  assert_eq!(titles, vec!["Strong Winds"]);
}
