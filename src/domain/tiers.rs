//! Rain intensity ladder. Escalation re-alerts fire only when the tier rises.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier { None = 0, Light = 1, Moderate = 2, Heavy = 3, Violent = 4 }

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
            Tier::None => "none", Tier::Light => "light", Tier::Moderate => "moderate",
            Tier::Heavy => "heavy", Tier::Violent => "violent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_are_inclusive_lower() {
        assert_eq!(Tier::from_rain_intensity(0.0), Tier::None);
        assert_eq!(Tier::from_rain_intensity(0.25), Tier::Light);
        assert_eq!(Tier::from_rain_intensity(2.5), Tier::Moderate);
        assert_eq!(Tier::from_rain_intensity(7.6), Tier::Heavy);
        assert_eq!(Tier::from_rain_intensity(50.0), Tier::Violent);
    }

    #[test]
    fn tiers_order_for_escalation_checks() {
        assert!(Tier::Heavy > Tier::Moderate);
        assert!(Tier::from_rain_intensity(9.0) > Tier::from_rain_intensity(3.0));
    }
}
