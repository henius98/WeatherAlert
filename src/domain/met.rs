//! MET Malaysia warning handling, written against the live payload shape.
//!
//! Traps this module exists to absorb, all observed on api.data.gov.my:
//!   1. `valid_from`/`valid_to` are NULLABLE (the "No Advisory" sentinel record).
//!   2. Timestamps are naive and are MYT, with no offset suffix.
//!   3. Three distinct records can share `issued` AND `title_en`, differing only
//!      by validity window -> the dedup key must include the window.
//!   4. `text_en` runs to ~1.5 KB and mixes land warnings with a shipping block;
//!      Telegram caps a message at 4096 chars.

use crate::domain::timeutil::parse_naive_myt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct WarningIssue {
    pub issued: Option<String>,
    #[serde(default)]
    pub title_en: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Warning {
    pub warning_issue: WarningIssue,
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_to: Option<String>,
    #[serde(default)]
    pub heading_en: Option<String>,
    #[serde(default)]
    pub text_en: Option<String>,
    #[serde(default)]
    pub instruction_en: Option<String>,
}

pub const SHIPPING_MARKER: &str = "SECTION B: FOR SHIPPING";
const NO_ADVISORY: &str = "no advisory";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetState { pub seen: Vec<String> }

impl Warning {
    pub fn title(&self) -> &str { self.warning_issue.title_en.as_deref().unwrap_or("") }
    pub fn heading(&self) -> &str { self.heading_en.as_deref().unwrap_or("") }

    /// A sentinel meaning "nothing active" — never forward it.
    pub fn is_no_advisory(&self) -> bool {
        self.title().trim().eq_ignore_ascii_case(NO_ADVISORY)
            || self.heading().trim().eq_ignore_ascii_case(NO_ADVISORY)
    }

    /// Includes the validity window: three live records shared issued+title and
    /// differed only here. Keying on issued+title alone loses two of them.
    pub fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.warning_issue.issued.as_deref().unwrap_or("-"),
            self.title(),
            self.valid_from.as_deref().unwrap_or("-"),
            self.valid_to.as_deref().unwrap_or("-"),
        )
    }

    /// Null `valid_to` means open-ended, not expired. Unparseable is treated as
    /// live too: better a spurious forward than a dropped flood warning.
    pub fn is_expired(&self, now: i64) -> bool {
        match self.valid_to.as_deref().and_then(parse_naive_myt) {
            Some(end) => end < now,
            None => false,
        }
    }

    /// The land-facing half of `text_en`, with the shipping block removed.
    pub fn land_text(&self) -> &str {
        let t = self.text_en.as_deref().unwrap_or("");
        match t.find(SHIPPING_MARKER) {
            Some(i) => t[..i].trim_end(),
            None => t,
        }
    }

    /// Case-insensitive substring test against the land text and heading.
    pub fn mentions_any(&self, needles: &[&str]) -> bool {
        let hay = format!("{} {}", self.heading(), self.land_text()).to_lowercase();
        needles.iter().any(|n| !n.is_empty() && hay.contains(&n.to_lowercase()))
    }
}

/// Phase-2 gate. `severity` and `places` are keyword lists; an empty list means
/// "do not gate on this axis", so a half-configured filter degrades to
/// forwarding rather than to silence. Both are matched against the heading and
/// the land text only — never the shipping block.
///
/// Keywords must be place NAMES ("Selangor", "Sepang"), not data.gov.my ids:
/// "St002" appears nowhere in MET prose.
pub fn passes_filter(w: &Warning, severity: &[&str], places: &[&str]) -> bool {
    let sev_ok = severity.is_empty() || w.mentions_any(severity);
    let place_ok = places.is_empty() || w.mentions_any(places);
    sev_ok && place_ok
}

/// Observe mode forwards every live, non-sentinel, unseen warning with no
/// severity or relevance filter, so the real vocabulary can be harvested before
/// the filter is written. Returns what to send plus the updated seen-list.
pub fn select_observe(warnings: &[Warning], st: &MetState, now: i64) -> (Vec<usize>, MetState) {
    let mut out = Vec::new();
    let mut seen = st.seen.clone();
    for (i, w) in warnings.iter().enumerate() {
        if w.is_no_advisory() || w.is_expired(now) { continue; }
        let key = w.dedup_key();
        if seen.iter().any(|k| *k == key) { continue; }
        seen.push(key);
        out.push(i);
    }
    // Warnings stay valid for up to a week; a bounded ring is plenty and keeps
    // the KV value small.
    let overflow = seen.len().saturating_sub(200);
    seen.drain(..overflow);
    (out, MetState { seen })
}

/// Telegram hard-caps a message at 4096 chars; leave room for our own framing.
pub fn format_observe(w: &Warning, budget: usize) -> String {
    let mut s = format!("\u{26A0}\u{FE0F} [OBSERVE] {}\n{}\n", w.title(), w.heading());
    if let (Some(f), Some(t)) = (&w.valid_from, &w.valid_to) {
        s.push_str(&format!("valid: {f} -> {t} (MYT)\n"));
    }
    if let Some(i) = &w.instruction_en { s.push_str(&format!("\n{i}\n")); }
    s.push('\n');
    let body = w.land_text();
    let room = budget.saturating_sub(s.chars().count() + 16);
    if body.chars().count() > room {
        s.extend(body.chars().take(room));
        s.push_str("\n[truncated]");
    } else {
        s.push_str(body);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(issued: &str, title: &str, from: Option<&str>, to: Option<&str>) -> Warning {
        Warning {
            warning_issue: WarningIssue {
                issued: Some(issued.into()), title_en: Some(title.into()),
            },
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
        let old = w("2026-08-22T19:30:00", "Thunderstorms Warning",
                    Some("2026-08-22T19:00:00"), Some("2026-08-22T22:00:00"));
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
        m.text_en = Some(format!(
            "SECTION A: land warning for Sepang. {SHIPPING_MARKER} waters of Phuket and Layang-Layang."
        ));
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
        m.text_en = Some(format!(
            "SECTION A: winds over Terengganu. {SHIPPING_MARKER} KATEGORI KETIGA over Phuket."
        ));
        assert!(!passes_filter(&m, &["kategori ketiga"], &[]),
            "a severity word inside the shipping block must not qualify the warning");
    }

    #[test]
    fn formatting_respects_the_telegram_cap() {
        let mut m = w("2026-08-22T20:25:00", "Strong Winds", None, None);
        m.text_en = Some("x".repeat(9000));
        let out = format_observe(&m, 4096);
        assert!(out.chars().count() <= 4096, "got {}", out.chars().count());
        assert!(out.ends_with("[truncated]"));
    }
}
