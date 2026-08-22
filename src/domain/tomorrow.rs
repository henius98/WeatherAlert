//! Parses the Tomorrow.io Timelines response into the minute rows the rule
//! evaluator consumes. Pure: no network, no worker dependency.

use crate::domain::rules::Minute;
use crate::domain::timeutil::parse_naive_utc;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Interval {
    #[serde(rename = "startTime")]
    start_time: String,
    #[serde(default)]
    values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Timeline {
    #[serde(default)]
    intervals: Vec<Interval>,
}

#[derive(Debug, Deserialize)]
struct Data {
    #[serde(default)]
    timelines: Vec<Timeline>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    data: Data,
}

/// Minute rows at or after `now`, offsets rounded down, sorted ascending.
///
/// Non-numeric and null values are dropped rather than defaulted to zero: a
/// missing field must never satisfy a `<` rule by accident.
pub fn parse_timeline(body: &str, now: i64) -> Result<Vec<Minute>, serde_json::Error> {
    let env: Envelope = serde_json::from_str(body)?;
    let mut out = Vec::new();
    for tl in &env.data.timelines {
        for iv in &tl.intervals {
            let Some(t) = parse_naive_utc(&iv.start_time) else { continue };
            if t < now {
                continue;
            }
            let mut fields = BTreeMap::new();
            for (k, v) in &iv.values {
                if let Some(n) = v.as_f64() {
                    fields.insert(k.clone(), n);
                }
            }
            if fields.is_empty() {
                continue;
            }
            out.push(Minute { offset_min: ((t - now) / 60) as u32, fields });
        }
    }
    out.sort_by_key(|m| m.offset_min);
    Ok(out)
}

/// Distinct fields referenced by the rule table, for the `fields=` query param.
/// `rainIntensity` is always included because the tier ladder needs it even
/// when no rule mentions it.
pub fn required_fields(rules: &[crate::domain::rules::Rule]) -> Vec<String> {
    let mut f: Vec<String> = rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.field.clone())
        .collect();
    f.push("rainIntensity".to_string());
    f.sort();
    f.dedup();
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rules::{Op, Rule, Value};

    const NOW: i64 = 1787403600; // 2026-08-22T13:00:00Z

    fn body() -> String {
        r#"{"data":{"timelines":[{"timestep":"1m","intervals":[
          {"startTime":"2026-08-22T13:00:00Z","values":{"rainIntensity":0,"windGust":5.1,"weatherCode":1000}},
          {"startTime":"2026-08-22T13:07:00Z","values":{"rainIntensity":3.4,"windGust":12.0,"weatherCode":4001}},
          {"startTime":"2026-08-22T13:19:00Z","values":{"rainIntensity":11.2,"windGust":48.0,"weatherCode":8000}}
        ]}]}}"#
            .to_string()
    }

    #[test]
    fn maps_intervals_to_minute_offsets() {
        let tl = parse_timeline(&body(), NOW).unwrap();
        assert_eq!(tl.len(), 3);
        assert_eq!(tl.iter().map(|m| m.offset_min).collect::<Vec<_>>(), vec![0, 7, 19]);
        assert_eq!(tl[2].fields["rainIntensity"], 11.2);
        assert_eq!(tl[2].fields["weatherCode"], 8000.0);
    }

    #[test]
    fn drops_intervals_already_in_the_past() {
        let tl = parse_timeline(&body(), NOW + 600).unwrap();
        assert_eq!(tl.iter().map(|m| m.offset_min).collect::<Vec<_>>(), vec![9]);
    }

    #[test]
    fn null_and_non_numeric_values_are_dropped_not_zeroed() {
        let b = r#"{"data":{"timelines":[{"intervals":[
          {"startTime":"2026-08-22T13:00:00Z","values":{"rainIntensity":null,"cloudBase":"n/a","windGust":9.0}}
        ]}]}}"#;
        let tl = parse_timeline(b, NOW).unwrap();
        assert_eq!(tl.len(), 1);
        assert!(!tl[0].fields.contains_key("rainIntensity"), "null must not become 0.0");
        assert!(!tl[0].fields.contains_key("cloudBase"));
        assert_eq!(tl[0].fields["windGust"], 9.0);
    }

    #[test]
    fn empty_and_malformed_payloads_are_handled() {
        assert!(parse_timeline(r#"{"data":{"timelines":[]}}"#, NOW).unwrap().is_empty());
        assert!(parse_timeline(r#"{"data":{}}"#, NOW).unwrap().is_empty());
        assert!(parse_timeline("not json", NOW).is_err());
    }

    #[test]
    fn required_fields_always_include_rain_intensity() {
        let rules = vec![Rule {
            id: "g".into(), group: None, field: "windGust".into(),
            op: Op::Gte, value: Value::Num(40.0), lead_min: 20, enabled: true,
        }];
        assert_eq!(required_fields(&rules), vec!["rainIntensity", "windGust"]);
        assert_eq!(required_fields(&[]), vec!["rainIntensity"]);
    }

    #[test]
    fn end_to_end_rule_match_against_a_real_payload() {
        use crate::domain::rules::first_match;
        use crate::domain::tiers::Tier;
        let rules = vec![
            Rule { id: "a".into(), group: Some("g1".into()), field: "weatherCode".into(),
                   op: Op::In, value: Value::Set(vec![8000.0]), lead_min: 20, enabled: true },
            Rule { id: "b".into(), group: Some("g1".into()), field: "windGust".into(),
                   op: Op::Gte, value: Value::Num(40.0), lead_min: 20, enabled: true },
        ];
        let tl = parse_timeline(&body(), NOW).unwrap();
        let m = first_match(&rules, &tl).unwrap();
        assert_eq!(m.offset_min, 19);
        assert_eq!(Tier::from_rain_intensity(tl[2].fields["rainIntensity"]), Tier::Heavy);
    }
}
