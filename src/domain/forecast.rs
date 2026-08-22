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

/// Records for `district`, sorted by date ascending, limited to `days`.
pub fn parse(body: &str, district: &str, days: usize) -> Result<Vec<DayForecast>, serde_json::Error> {
    let mut v: Vec<DayForecast> = serde_json::from_str(body)?;
    v.retain(|f| f.location.location_id == district);
    v.sort_by(|a, b| a.date.cmp(&b.date));
    v.truncate(days);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verbatim from api.data.gov.my.
    const BODY: &str = r#"[
      {"location":{"location_id":"Ds001","location_name":"Langkawi"},"date":"2026-08-28",
       "morning_forecast":"Tiada Hujan","afternoon_forecast":"Tiada Hujan","night_forecast":"Tiada Hujan",
       "summary_forecast":"Tiada Hujan","summary_when":"Sepanjang Hari","min_temp":25,"max_temp":32},
      {"location":{"location_id":"Ds001","location_name":"Langkawi"},"date":"2026-08-27",
       "summary_forecast":"Ribut petir di beberapa tempat","summary_when":"Petang","min_temp":24,"max_temp":33},
      {"location":{"location_id":"Ds064","location_name":"Sepang"},"date":"2026-08-22",
       "summary_forecast":"Hujan di beberapa tempat","summary_when":"Petang","min_temp":24,"max_temp":33}
    ]"#;

    #[test]
    fn filters_by_district_and_sorts_ascending() {
        let f = parse(BODY, "Ds001", 7).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].date, "2026-08-27", "must be sorted, the feed is not");
        assert_eq!(f[1].date, "2026-08-28");
    }

    #[test]
    fn truncates_to_requested_days() {
        assert_eq!(parse(BODY, "Ds001", 1).unwrap().len(), 1);
        assert!(parse(BODY, "Ds999", 7).unwrap().is_empty());
    }

    #[test]
    fn translates_the_observed_malay_vocabulary() {
        assert_eq!(translate("Tiada Hujan"), "No rain");
        assert_eq!(translate("Ribut petir di beberapa tempat"), "Thunderstorms in some areas");
        assert_eq!(translate("Hujan di kebanyakan tempat"), "Rain in most areas");
        assert_eq!(translate("Jerebu"), "Haze");
        assert_eq!(translate("Sepanjang Hari"), "all day");
    }

    #[test]
    fn compound_phrases_beat_their_substrings() {
        // "Hujan" alone must not win over the longer phrase.
        assert_eq!(
            translate("Hujan di beberapa tempat di kawasan pedalaman"),
            "Rain in some inland areas"
        );
        assert_eq!(
            translate("Ribut petir di beberapa tempat di kawasan perairan"),
            "Thunderstorms over some coastal waters"
        );
    }

    #[test]
    fn unknown_strings_pass_through_rather_than_vanishing() {
        assert_eq!(translate("Cuaca Aneh"), "Cuaca Aneh");
    }

    #[test]
    fn optional_fields_absent_do_not_panic() {
        let f = parse(BODY, "Ds001", 7).unwrap();
        assert_eq!(f[0].summary_en(), "Thunderstorms in some areas");
        assert_eq!(f[0].temp_range().unwrap(), "24-33C");
        assert!(f[0].morning_forecast.is_none());
    }
}
