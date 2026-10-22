//! Daily alert tally and the morning summary.
//!
//! The summary is also the liveness signal. Without it, a Worker that has
//! stopped running is indistinguishable from a quiet day — silence would mean
//! both "no rain" and "no system", which is the one failure an alerting tool
//! must not have.

use crate::domain::forecast::DayForecast;
use crate::domain::tiers::Tier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyLog {
  #[serde(default)]
  pub date: String,
  /// Tier label -> count of events opened at that tier.
  #[serde(default)]
  pub opens: BTreeMap<String, u32>,
  #[serde(default)]
  pub escalations: u32,
}

impl DailyLog {
  /// Rolls the tally when the civil date changes, then records.
  pub fn record(&mut self, today: &str, tier: Tier, escalation: bool) {
    if self.date != today {
      *self = DailyLog { date: today.to_string(), ..Default::default() };
    }
    if escalation {
      self.escalations += 1;
    } else {
      *self.opens.entry(tier.label().to_string()).or_insert(0) += 1;
    }
  }

  pub fn total_opens(&self) -> u32 {
    self.opens.values().sum()
  }

  /// One line for the morning summary, phrased for threshold tuning.
  pub fn tuning_line(&self) -> String {
    let total = self.total_opens();
    if total == 0 && self.escalations == 0 {
      return "No rain alerts fired.".to_string();
    }
    let breakdown: Vec<String> = self.opens.iter().map(|(t, n)| format!("{t} {n}")).collect();
    format!("{total} rain alert{} ({}), {} escalation{}.", if total == 1 { "" } else { "s" }, breakdown.join(", "), self.escalations, if self.escalations == 1 { "" } else { "s" },)
  }
}

/// The 08:00 MYT message: today's outlook, live warnings, yesterday's tally,
/// and an explicit statement that the monitor is running.
pub fn morning_summary(label: &str, today: &str, days: &[DayForecast], warnings: &[String], yesterday: &DailyLog, window: (i64, i64)) -> String {
  let mut s = format!("\u{2600}\u{FE0F} Good morning \u{2014} {label}\n{today}\n");

  if days.is_empty() {
    s.push_str("\nMET forecast unavailable this morning.\n");
  } else {
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

  s.push_str(&format!("\nYesterday: {}\n", yesterday.tuning_line()));
  s.push_str(&format!("Monitoring {:02}:00-{:02}:00 MYT every 3 min. If this message stops arriving, the monitor is down.", window.0, window.1));
  s
}
