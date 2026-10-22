//! Tests for `domain::tiers`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::tiers::*;

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
