//! Rain intensity ladder. Escalation re-alerts fire only when the tier rises.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
  None = 0,
  Light = 1,
  Moderate = 2,
  Heavy = 3,
  Violent = 4,
}

impl Tier {
  /// mm/hr thresholds (standard meteorological intensity classes).
  pub fn from_rain_intensity(mm_per_hr: f64) -> Tier {
    match mm_per_hr {
      v if v >= 50.0 => Tier::Violent,
      v if v >= 7.6 => Tier::Heavy,
      v if v >= 2.5 => Tier::Moderate,
      v if v >= 0.25 => Tier::Light,
      _ => Tier::None,
    }
  }
  pub fn label(self) -> &'static str {
    match self {
      Tier::None => "none",
      Tier::Light => "light",
      Tier::Moderate => "moderate",
      Tier::Heavy => "heavy",
      Tier::Violent => "violent",
    }
  }
}
