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
pub struct MetState {
  pub seen: Vec<String>,
}

impl Warning {
  pub fn title(&self) -> &str {
    self.warning_issue.title_en.as_deref().unwrap_or("")
  }
  pub fn heading(&self) -> &str {
    self.heading_en.as_deref().unwrap_or("")
  }

  /// A sentinel meaning "nothing active" — never forward it.
  pub fn is_no_advisory(&self) -> bool {
    self.title().trim().eq_ignore_ascii_case(NO_ADVISORY) || self.heading().trim().eq_ignore_ascii_case(NO_ADVISORY)
  }

  /// Includes the validity window: three live records shared issued+title and
  /// differed only here. Keying on issued+title alone loses two of them.
  pub fn dedup_key(&self) -> String {
    format!("{}|{}|{}|{}", self.warning_issue.issued.as_deref().unwrap_or("-"), self.title(), self.valid_from.as_deref().unwrap_or("-"), self.valid_to.as_deref().unwrap_or("-"),)
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
    if w.is_no_advisory() || w.is_expired(now) {
      continue;
    }
    let key = w.dedup_key();
    if seen.contains(&key) {
      continue;
    }
    seen.push(key);
    out.push(i);
  }
  // Warnings stay valid for up to a week; a bounded ring is plenty and keeps
  // the KV value small.
  let overflow = seen.len().saturating_sub(200);
  seen.drain(..overflow);
  (out, MetState { seen })
}

/// Observe-mode form: title, heading and validity only.
///
/// The point of observe mode is harvesting the severity vocabulary, and that
/// vocabulary lives entirely in `title_en` / `heading_en`. Forwarding 1.5 KB of
/// prose per warning adds noise without adding signal.
pub fn format_summary(w: &Warning) -> String {
  let mut s = format!("\u{26A0}\u{FE0F} [OBSERVE] {}\n{}", w.title(), w.heading());
  if let (Some(f), Some(t)) = (&w.valid_from, &w.valid_to) {
    s.push_str(&format!("\nvalid: {f} -> {t} (MYT)"));
  }
  s
}

/// Filter-mode form, for a warning that actually concerns you. Telegram hard-caps
/// a message at 4096 chars; leave room for our own framing.
pub fn format_observe(w: &Warning, budget: usize) -> String {
  let mut s = format!("\u{26A0}\u{FE0F} {}\n{}\n", w.title(), w.heading());
  if let (Some(f), Some(t)) = (&w.valid_from, &w.valid_to) {
    s.push_str(&format!("valid: {f} -> {t} (MYT)\n"));
  }
  if let Some(i) = &w.instruction_en {
    s.push_str(&format!("\n{i}\n"));
  }
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

/// Active non-sentinel warning titles for summaries and reports.
pub fn active_titles(body: &str, now: i64) -> Vec<String> {
  let mut titles: Vec<String> = serde_json::from_str::<Vec<Warning>>(body)
    .unwrap_or_default()
    .into_iter()
    .filter(|w| !w.is_no_advisory() && !w.is_expired(now))
    .map(|w| w.title().to_string())
    .collect();
  titles.sort();
  titles.dedup();
  titles
}
