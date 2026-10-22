//! Tests for `domain::forecast`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::forecast::*;

// Verbatim from api.data.gov.my.
const BODY: &str = r#"[
  {"location":{"location_id":"Ds001","location_name":"Langkawi"},"date":"2026-08-28",
   "morning_forecast":"Tiada Hujan","afternoon_forecast":"Tiada Hujan","night_forecast":"Tiada Hujan",
   "summary_forecast":"Tiada Hujan","summary_when":"Sepanjang Hari","min_temp":25,"max_temp":32},
  {"location":{"location_id":"Ds001","location_name":"Langkawi"},"date":"2026-08-27",
   "summary_forecast":"Ribut petir di beberapa tempat","summary_when":"Petang","min_temp":24,"max_temp":33},
  {"location":{"location_id":"Ds064","location_name":"Sepang"},"date":"2026-08-22",
   "summary_forecast":"Hujan di beberapa tempat","summary_when":"Petang","min_temp":24,"max_temp":33}
]"#;

#[test]
fn filters_by_district_and_sorts_ascending() {
  let f = parse(BODY, "Ds001", 7).unwrap();
  assert_eq!(f.len(), 2);
  assert_eq!(f[0].date, "2026-08-27", "must be sorted, the feed is not");
  assert_eq!(f[1].date, "2026-08-28");
}

#[test]
fn truncates_to_requested_days() {
  assert_eq!(parse(BODY, "Ds001", 1).unwrap().len(), 1);
  assert!(parse(BODY, "Ds999", 7).unwrap().is_empty());
}

#[test]
fn translates_the_observed_malay_vocabulary() {
  assert_eq!(translate("Tiada Hujan"), "No rain");
  assert_eq!(translate("Ribut petir di beberapa tempat"), "Thunderstorms in some areas");
  assert_eq!(translate("Hujan di kebanyakan tempat"), "Rain in most areas");
  assert_eq!(translate("Jerebu"), "Haze");
  assert_eq!(translate("Sepanjang Hari"), "all day");
}

#[test]
fn compound_phrases_beat_their_substrings() {
  // "Hujan" alone must not win over the longer phrase.
  assert_eq!(translate("Hujan di beberapa tempat di kawasan pedalaman"), "Rain in some inland areas");
  assert_eq!(translate("Ribut petir di beberapa tempat di kawasan perairan"), "Thunderstorms over some coastal waters");
}

#[test]
fn unknown_strings_pass_through_rather_than_vanishing() {
  assert_eq!(translate("Cuaca Aneh"), "Cuaca Aneh");
}

#[test]
fn optional_fields_absent_do_not_panic() {
  let f = parse(BODY, "Ds001", 7).unwrap();
  assert_eq!(f[0].summary_en(), "Thunderstorms in some areas");
  assert_eq!(f[0].temp_range().unwrap(), "24-33C");
  assert!(f[0].morning_forecast.is_none());
}

#[test]
fn every_filter_carries_its_column_suffix() {
  // The bug this pins: bare `date_start=2026-08-23` is answered with HTTP 400,
  // "Use <YYYY-MM-DD>@<date_column>". Both callers fold a non-200 into an
  // empty Vec, so the only symptom was a morning brief with no forecast in it.
  let q = forecast_query("Ds064", "2026-08-23", "2026-08-24");
  assert_eq!(q, "contains=Ds064@location__location_id&date_start=2026-08-23@date&date_end=2026-08-24@date");
}

#[test]
fn column_suffixes_survive_urlencoding_of_the_values() {
  // `@` is query syntax, so it must stay literal even though urlencode would
  // escape it to %40 if the suffix were folded into the encoded value.
  let q = forecast_query("Ds 064", "2026-08-23", "2026-08-23");
  assert!(q.starts_with("contains=Ds%20064@location__location_id"), "{q}");
  assert_eq!(q.matches("@date").count(), 2, "{q}");
  assert!(!q.contains("%40"), "{q}");
}
