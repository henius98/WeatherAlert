//! Tests for `domain::util`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::util::*;

#[test]
fn ct_eq_matches_only_identical_strings() {
  assert!(ct_eq("abc123", "abc123"));
  assert!(!ct_eq("abc123", "abc124"));
  assert!(!ct_eq("abc", "abcd"));
  assert!(ct_eq("", ""));
}

#[test]
fn ct_eq_rejects_prefixes() {
  // A naive starts_with check would accept this.
  assert!(!ct_eq("secret", "secretlonger"));
  assert!(!ct_eq("secretlonger", "secret"));
}

#[test]
fn urlencode_escapes_reserved_characters() {
  assert_eq!(urlencode("2.8333,101.7"), "2.8333%2C101.7");
  assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
  assert_eq!(urlencode("safe-_.~"), "safe-_.~");
}

#[test]
fn redact_strips_an_api_key_quoted_back_in_an_error() {
  let key = "abcd1234efgh5678";
  let err = format!("failed to parse URL from https://api.tomorrow.io/v4/timelines?apikey={key}");
  let out = redact(&err, &[key, "tgtoken9999999"]);
  assert!(!out.contains(key), "key leaked: {out}");
  assert!(out.contains("***"));
  assert!(out.contains("api.tomorrow.io"), "non-secret context should survive");
}

#[test]
fn redact_ignores_short_or_empty_secrets() {
  // An unset secret is "" — redacting it would destroy every message.
  assert_eq!(redact("hello world", &[""]), "hello world");
  assert_eq!(redact("hello world", &["o"]), "hello world");
}

#[test]
fn redact_removes_every_occurrence() {
  let k = "supersecretvalue";
  let out = redact(&format!("{k} then {k}"), &[k]);
  assert_eq!(out, "*** then ***");
}

#[test]
fn urlencode_handles_multibyte() {
  assert_eq!(urlencode("é"), "%C3%A9");
}
