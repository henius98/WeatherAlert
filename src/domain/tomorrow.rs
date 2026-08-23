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

/// Tomorrow.io core layer fields usable on the free plan.
///
/// An unvalidated field name is not a harmless typo: it goes straight into the
/// `fields=` query parameter, Tomorrow.io answers 400, and *every* subsequent
/// fetch fails. One mistyped rule takes down all alerting.
pub const VALID_FIELDS: &[&str] = &[
    "cloudBase", "cloudCeiling", "cloudCover", "dewPoint", "evapotranspiration",
    "freezingRainIntensity", "humidity", "precipitationProbability",
    "pressureSeaLevel", "pressureSurfaceLevel", "rainAccumulation",
    "rainIntensity", "sleetIntensity", "snowAccumulation", "snowIntensity",
    "temperature", "temperatureApparent", "uvHealthConcern", "uvIndex",
    "visibility", "weatherCode", "windDirection", "windGust", "windSpeed",
];

pub fn is_valid_field(f: &str) -> bool {
    VALID_FIELDS.contains(&f)
}

/// Forecast window to request, sized to the widest rule so a long `lead_min`
/// is not silently truncated. Costs no extra API call — only a longer response.
pub fn window_minutes(rules: &[crate::domain::rules::Rule]) -> u32 {
    let want = crate::domain::rules::max_lead_min(rules).saturating_add(5);
    want.clamp(30, 120)
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
