//! data.gov.my district forecast: daily granularity, Malay summary strings.
//! Context only — it never triggers an alert.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Location {
    pub location_id: String,
    pub location_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DayForecast {
    pub location: Location,
    pub date: String,
    #[serde(default)]
    pub morning_forecast: Option<String>,
    #[serde(default)]
    pub afternoon_forecast: Option<String>,
    #[serde(default)]
    pub night_forecast: Option<String>,
    #[serde(default)]
    pub summary_forecast: Option<String>,
    #[serde(default)]
    pub summary_when: Option<String>,
    #[serde(default)]
    pub min_temp: Option<f64>,
    #[serde(default)]
    pub max_temp: Option<f64>,
}

/// Longest-first so compound phrases win over their own substrings.
const PHRASES: &[(&str, &str)] = &[
    ("Ribut petir di beberapa tempat di kawasan pedalaman", "Thunderstorms in some inland areas"),
    ("Ribut petir di beberapa tempat di kawasan perairan", "Thunderstorms over some coastal waters"),
    ("Hujan di kebanyakan tempat di kawasan perairan", "Rain over most coastal waters"),
    ("Hujan di beberapa tempat di kawasan pedalaman", "Rain in some inland areas"),
    ("Hujan di beberapa tempat di kawasan perairan", "Rain over some coastal waters"),
    ("Ribut petir di beberapa tempat", "Thunderstorms in some areas"),
    ("Hujan di kebanyakan tempat", "Rain in most areas"),
    ("Hujan di beberapa tempat", "Rain in some areas"),
    ("Tiada hujan", "No rain"),
    ("Ribut petir", "Thunderstorms"),
    ("Sepanjang Hari", "all day"),
    ("Berangin", "Windy"),
    ("Jerebu", "Haze"),
    ("Hujan", "Rain"),
    ("Pagi", "morning"),
    ("Petang", "afternoon"),
    ("Malam", "night"),
];

/// Best-effort translation of the closed Malay vocabulary. Unknown strings pass
/// through untouched rather than being dropped — a summary we cannot translate
/// is still more useful than a blank.
pub fn translate(s: &str) -> String {
    let mut out = s.trim().to_string();
    for (my, en) in PHRASES {
        if out.eq_ignore_ascii_case(my) {
            return (*en).to_string();
        }
    }
    for (my, en) in PHRASES {
        if let Some(i) = out.to_lowercase().find(&my.to_lowercase()) {
            out.replace_range(i..i + my.len(), en);
        }
    }
    out
}

impl DayForecast {
    pub fn summary_en(&self) -> String {
        self.summary_forecast.as_deref().map(translate).unwrap_or_default()
    }
    pub fn temp_range(&self) -> Option<String> {
        Some(format!("{:.0}-{:.0}C", self.min_temp?, self.max_temp?))
    }
}

/// Query string for `/weather/forecast/`, built here rather than in `infra` so
/// the native test suite can hold it to the server's syntax.
///
/// Every filter is `<value>@<column>`: the column name is part of the query
/// language, not of the value, so it stays outside `urlencode`. Omitting `@date`
/// on the bounds is not a lenient default — the server answers HTTP 400.
pub fn forecast_query(district: &str, date_start: &str, date_end: &str) -> String {
    format!(
        "contains={}@location__location_id&date_start={}@date&date_end={}@date",
        crate::domain::util::urlencode(district),
        crate::domain::util::urlencode(date_start),
        crate::domain::util::urlencode(date_end),
    )
}

/// Records for `district`, sorted by date ascending, limited to `days`.
pub fn parse(body: &str, district: &str, days: usize) -> Result<Vec<DayForecast>, serde_json::Error> {
    let mut v: Vec<DayForecast> = serde_json::from_str(body)?;
    v.retain(|f| f.location.location_id == district);
    v.sort_by(|a, b| a.date.cmp(&b.date));
    v.truncate(days);
    Ok(v)
}
