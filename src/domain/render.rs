//! Telegram message bodies. Pure so the wording is testable without a network.

use crate::domain::forecast::DayForecast;
use crate::domain::state::Action;
use crate::domain::tiers::Tier;

pub const TG_LIMIT: usize = 4096;

/// Bare form for the second half of a range ("... by ~15 min").
fn mins(min: u32) -> String {
  match min {
    0 => "now".to_string(),
    n => format!("~{n} min"),
  }
}

fn eta(min: u32) -> String {
  match min {
    0 => "now".to_string(),
    1 => "in ~1 min".to_string(),
    n => format!("in ~{n} min"),
  }
}

/// Onset and peak in one clause. When the worst minute is the first minute this
/// collapses to the simple form; otherwise it says both, because "light rain in
/// 2 min" is a dangerous way to describe a downpour arriving at minute 15.
fn window(onset: u32, peak_tier: Tier, peak: u32) -> String {
  if peak == onset {
    format!("{} rain {}", cap(peak_tier.label()), eta(onset))
  } else {
    format!("Rain {}, {} by {}", eta(onset), peak_tier.label(), mins(peak))
  }
}

pub fn alert(action: &Action, label: &str) -> Option<String> {
  match action {
    Action::Open { peak_tier, onset_min, peak_min, rule_ids } => {
      let rules = if rule_ids.is_empty() { String::new() } else { format!("\nrules: {}", rule_ids.join(", ")) };
      Some(format!("\u{1F327} {} \u{2014} {}{}", window(*onset_min, *peak_tier, *peak_min), label, rules))
    }
    Action::Escalate { from, to, onset_min, peak_min, rule_ids } => {
      let rules = if rule_ids.is_empty() { String::new() } else { format!("\nrules: {}", rule_ids.join(", ")) };
      Some(format!("\u{26A1} Intensifying: {} \u{2192} {} \u{2014} {} \u{2014} {}{}", from.label(), to.label(), window(*onset_min, *to, *peak_min), label, rules))
    }
    Action::Close { duration_min } => Some(format!("\u{2705} Rain has passed \u{2014} {label} (lasted ~{duration_min} min)")),
    Action::Nothing => None,
  }
}

fn cap(s: &str) -> String {
  let mut c = s.chars();
  match c.next() {
    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    None => String::new(),
  }
}

/// The `/now` and `GET /api/report` body.
pub fn report(label: &str, tier: Tier, next_rain_min: Option<u32>, days: &[DayForecast], warnings: &[String], stale: bool, have_data: bool) -> String {
  let mut s = format!("\u{1F4CD} {label}\n");
  // "no data" and "no rain" must never render the same. Reporting "no rain
  // expected" when the nowcast is simply missing is a false all-clear.
  s.push_str(&match next_rain_min {
    _ if !have_data => "Next 30 min: NO DATA \u{2014} nowcast unavailable\n".to_string(),
    Some(m) if tier != Tier::None => format!("Next 30 min: {} rain {}\n", tier.label(), eta(m)),
    Some(m) => format!("Next 30 min: precipitation {}\n", eta(m)),
    None => "Next 30 min: no rain expected\n".to_string(),
  });
  if stale {
    s.push_str("(cached \u{2014} hourly manual-refresh cap reached)\n");
  }
  if !days.is_empty() {
    s.push_str("\nMET forecast\n");
    for d in days {
      let t = d.temp_range().map(|r| format!(" {r}")).unwrap_or_default();
      s.push_str(&format!("  {} {}{}\n", d.date, d.summary_en(), t));
    }
  }
  if !warnings.is_empty() {
    s.push_str("\n\u{26A0}\u{FE0F} Active warnings\n");
    for w in warnings {
      s.push_str(&format!("  {w}\n"));
    }
  }
  truncate(s, TG_LIMIT)
}

pub fn truncate(mut s: String, limit: usize) -> String {
  if s.chars().count() <= limit {
    return s;
  }
  s = s.chars().take(limit.saturating_sub(12)).collect();
  s.push_str("\n[truncated]");
  s
}
