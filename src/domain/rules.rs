//! Rule evaluation in disjunctive normal form.
//!
//! `group: None` -> the rule fires on its own (the OR case).
//! `group: Some("g1")` -> every enabled rule in "g1" must hold AT THE SAME
//! forecast minute (the AND case). Requiring the same minute is what makes
//! "thunderstorm arriving" distinct from "gusts now, rain in 19 minutes".

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op { Gt, Gte, Lt, Lte, Eq, In }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value { Num(f64), Set(Vec<f64>) }

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
fn default_lead() -> u32 { 20 }
fn yes() -> bool { true }

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
            (Op::Eq, Value::Num(t)) => (v - *t).abs() < f64::EPSILON,
            (Op::In, Value::Set(s)) => s.iter().any(|t| (v - *t).abs() < f64::EPSILON),
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
        let mut hits: Vec<String> = singles
            .iter()
            .filter(|r| m.offset_min <= r.lead_min && r.holds(m))
            .map(|r| r.id.clone())
            .collect();

        // AND groups: every member must hold at this same minute.
        for members in groups.values() {
            let all = members
                .iter()
                .all(|r| m.offset_min <= r.lead_min && r.holds(m));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minute(off: u32, kv: &[(&str, f64)]) -> Minute {
        Minute {
            offset_min: off,
            fields: kv.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }
    fn rule(id: &str, group: Option<&str>, field: &str, op: Op, v: Value) -> Rule {
        Rule {
            id: id.into(), group: group.map(String::from), field: field.into(),
            op, value: v, lead_min: 20, enabled: true,
        }
    }

    #[test]
    fn shipped_seed_file_parses_and_means_what_the_docs_say() {
        // rules.seed.json is what gets written to KV; a typo there would deploy
        // a Worker with no rules and no alerts, silently.
        let seed = include_str!("../../rules.seed.json");
        let rules: Vec<Rule> = serde_json::from_str(seed).expect("rules.seed.json must parse");
        assert_eq!(rules.len(), 3);

        let solo: Vec<_> = rules.iter().filter(|r| r.group.is_none()).collect();
        assert_eq!(solo.len(), 1, "one independent rain rule");
        assert_eq!(solo[0].field, "rainIntensity");

        // The thunderstorm pair must be AND-ed: both, same minute, or nothing.
        let tl = vec![minute(5, &[("weatherCode", 8000.0), ("windGust", 10.0)])];
        assert!(first_match(&rules, &tl).is_none(), "code alone must not fire");
        let tl2 = vec![minute(5, &[("weatherCode", 8000.0), ("windGust", 55.0)])];
        assert!(first_match(&rules, &tl2).is_some(), "code + gust must fire");
    }

    #[test]
    fn independent_rule_fires_alone() {
        let rules = vec![rule("rain", None, "rainIntensity", Op::Gte, Value::Num(2.5))];
        let tl = vec![minute(0, &[("rainIntensity", 0.1)]), minute(7, &[("rainIntensity", 3.0)])];
        assert_eq!(first_match(&rules, &tl).unwrap().offset_min, 7);
    }

    #[test]
    fn and_group_requires_same_minute() {
        let rules = vec![
            rule("storm_a", Some("g1"), "weatherCode", Op::In, Value::Set(vec![8000.0])),
            rule("storm_b", Some("g1"), "windGust", Op::Gte, Value::Num(40.0)),
        ];
        // Conditions present but on DIFFERENT minutes -> must not fire.
        let split = vec![
            minute(4, &[("weatherCode", 8000.0), ("windGust", 10.0)]),
            minute(9, &[("weatherCode", 1000.0), ("windGust", 55.0)]),
        ];
        assert!(first_match(&rules, &split).is_none());

        let together = vec![minute(6, &[("weatherCode", 8000.0), ("windGust", 55.0)])];
        let m = first_match(&rules, &together).unwrap();
        assert_eq!(m.offset_min, 6);
        assert_eq!(m.rule_ids, vec!["storm_a", "storm_b"]);
    }

    #[test]
    fn lead_min_bounds_the_window() {
        let mut r = rule("rain", None, "rainIntensity", Op::Gte, Value::Num(2.5));
        r.lead_min = 20;
        let tl = vec![minute(27, &[("rainIntensity", 9.0)])];
        assert!(first_match(&[r], &tl).is_none(), "27 min out exceeds a 20 min lead");
    }

    #[test]
    fn disabled_rules_and_missing_fields_never_fire() {
        let mut off = rule("rain", None, "rainIntensity", Op::Gte, Value::Num(2.5));
        off.enabled = false;
        let tl = vec![minute(2, &[("rainIntensity", 99.0)])];
        assert!(first_match(&[off], &tl).is_none());

        let r = rule("gust", None, "windGust", Op::Gte, Value::Num(40.0));
        assert!(first_match(&[r], &tl).is_none(), "windGust absent from timeline");
    }

    #[test]
    fn mismatched_op_and_value_shape_does_not_fire() {
        let bad = rule("bad", None, "rainIntensity", Op::Gte, Value::Set(vec![1.0]));
        let tl = vec![minute(1, &[("rainIntensity", 99.0)])];
        assert!(first_match(&[bad], &tl).is_none());
    }

    #[test]
    fn two_and_groups_or_against_each_other() {
        let rules = vec![
            rule("a1", Some("g1"), "windGust", Op::Gte, Value::Num(90.0)),
            rule("a2", Some("g1"), "rainIntensity", Op::Gte, Value::Num(90.0)),
            rule("b1", Some("g2"), "rainIntensity", Op::Gte, Value::Num(2.5)),
            rule("b2", Some("g2"), "precipitationProbability", Op::Gte, Value::Num(60.0)),
        ];
        let tl = vec![minute(5, &[("rainIntensity", 4.0), ("precipitationProbability", 80.0), ("windGust", 5.0)])];
        let m = first_match(&rules, &tl).unwrap();
        assert_eq!(m.rule_ids, vec!["b1", "b2"], "g2 satisfied, g1 not");
    }
}
