//! Rule evaluation in disjunctive normal form.
//!
//! `group: None` -> the rule fires on its own (the OR case).
//! `group: Some("g1")` -> every enabled rule in "g1" must hold AT THE SAME
//! forecast minute (the AND case). Requiring the same minute is what makes
//! "thunderstorm arriving" distinct from "gusts now, rain in 19 minutes".

use crate::domain::tiers::Tier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
  Gt,
  Gte,
  Lt,
  Lte,
  Eq,
  In,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
  Num(f64),
  Set(Vec<f64>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
  pub id: String,
  #[serde(default)]
  pub group: Option<String>,
  pub field: String,
  pub op: Op,
  pub value: Value,
  #[serde(default = "default_lead")]
  pub lead_min: u32,
  #[serde(default = "yes")]
  pub enabled: bool,
}
fn default_lead() -> u32 {
  20
}
fn yes() -> bool {
  true
}

/// One minute of the Tomorrow.io 1m timeline.
#[derive(Debug, Clone, Default)]
pub struct Minute {
  pub offset_min: u32,
  pub fields: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
  pub offset_min: u32,
  pub rule_ids: Vec<String>,
}

impl Rule {
  fn holds(&self, m: &Minute) -> bool {
    let Some(&v) = m.fields.get(&self.field) else { return false };
    match (&self.op, &self.value) {
      (Op::Gt, Value::Num(t)) => v > *t,
      (Op::Gte, Value::Num(t)) => v >= *t,
      (Op::Lt, Value::Num(t)) => v < *t,
      (Op::Lte, Value::Num(t)) => v <= *t,
      (Op::Eq, Value::Num(t)) => (v - *t).abs() < 1e-4,
      (Op::In, Value::Set(s)) => s.iter().any(|t| (v - *t).abs() < 1e-4),
      // A set threshold with a scalar operator (or vice versa) is a config
      // error, not a match. Never fire on a rule we cannot interpret.
      _ => false,
    }
  }
}

/// Earliest minute in the window where any disjunct is satisfied.
pub fn first_match(rules: &[Rule], timeline: &[Minute]) -> Option<Match> {
  let active: Vec<&Rule> = rules.iter().filter(|r| r.enabled).collect();
  let mut groups: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
  let mut singles: Vec<&Rule> = Vec::new();
  for r in &active {
    match r.group.as_deref() {
      Some(g) => groups.entry(g).or_default().push(r),
      None => singles.push(r),
    }
  }

  for m in timeline {
    // Independent rules: any one is enough.
    let mut hits: Vec<String> = singles.iter().filter(|r| m.offset_min <= r.lead_min && r.holds(m)).map(|r| r.id.clone()).collect();

    // AND groups: every member must hold at this same minute.
    for members in groups.values() {
      let all = members.iter().all(|r| m.offset_min <= r.lead_min && r.holds(m));
      if all && !members.is_empty() {
        hits.extend(members.iter().map(|r| r.id.clone()));
      }
    }

    if !hits.is_empty() {
      hits.sort();
      hits.dedup();
      return Some(Match { offset_min: m.offset_min, rule_ids: hits });
    }
  }
  None
}

/// What the alert should say: when rain starts, and how bad it gets.
///
/// Reporting only the onset understates a downpour sitting behind drizzle —
/// "light rain in 2 min" while 60 mm/hr arrives at minute 15.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
  pub onset_min: u32,
  pub rule_ids: Vec<String>,
  pub peak_tier: Tier,
  pub peak_min: u32,
}

/// Widest lead time any enabled rule asks for; also sizes the fetch window.
pub fn max_lead_min(rules: &[Rule]) -> u32 {
  rules.iter().filter(|r| r.enabled).map(|r| r.lead_min).max().unwrap_or(20)
}

/// Earliest matching minute, plus the worst rain intensity anywhere in the
/// lead window (not merely at the onset minute).
pub fn evaluate(rules: &[Rule], timeline: &[Minute]) -> Option<Hit> {
  let m = first_match(rules, timeline)?;
  let horizon = max_lead_min(rules);
  let mut peak_tier = Tier::None;
  let mut peak_min = m.offset_min;
  for x in timeline.iter().filter(|x| x.offset_min >= m.offset_min && x.offset_min <= horizon) {
    let v = x.fields.get("rainIntensity").copied().unwrap_or(0.0);
    let t = Tier::from_rain_intensity(v);
    if t > peak_tier {
      peak_tier = t;
      peak_min = x.offset_min;
    }
  }
  // A non-rain rule (gust, weather code) can fire with no rain at all; treat
  // that as Light so the message still conveys "something is coming".
  if peak_tier == Tier::None {
    peak_tier = Tier::Light;
    peak_min = m.offset_min;
  }
  Some(Hit { onset_min: m.offset_min, rule_ids: m.rule_ids, peak_tier, peak_min })
}
