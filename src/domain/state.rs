//! Event state machine: alert once on open, re-alert only on tier escalation,
//! close after a quiet gap, optional all-clear.

use crate::domain::tiers::Tier;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventState {
    pub open: bool,
    pub tier: u8,
    pub opened_at: i64,
    pub last_match_at: i64,
    pub rule_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Nothing,
    Open { tier: Tier, eta_min: u32 },
    Escalate { from: Tier, to: Tier, eta_min: u32 },
    Close { duration_min: i64 },
}

/// `write_needed` is the whole point: KV free tier allows 1000 writes/day and
/// the rain cron ticks 280 times, so we only persist on a transition.
pub struct Step { pub action: Action, pub write_needed: bool }

pub fn step(
    st: &mut EventState,
    matched: Option<(Tier, u32)>,
    now: i64,
    close_after_min: i64,
    all_clear: bool,
) -> Step {
    match matched {
        Some((tier, eta_min)) => {
            if !st.open {
                *st = EventState {
                    open: true, tier: tier as u8, opened_at: now,
                    last_match_at: now, rule_ids: st.rule_ids.clone(),
                };
                return Step { action: Action::Open { tier, eta_min }, write_needed: true };
            }
            st.last_match_at = now;
            let prev = st.tier;
            if (tier as u8) > prev {
                st.tier = tier as u8;
                let from = tier_from_u8(prev);
                return Step {
                    action: Action::Escalate { from, to: tier, eta_min },
                    write_needed: true,
                };
            }
            // Same or lower tier: stay silent. We never de-escalate, and we do
            // not write, so a 60-minute shower costs one KV write, not twenty.
            Step { action: Action::Nothing, write_needed: false }
        }
        None => {
            if st.open && now - st.last_match_at >= close_after_min * 60 {
                let duration_min = (st.last_match_at - st.opened_at) / 60;
                *st = EventState::default();
                let action = if all_clear { Action::Close { duration_min } } else { Action::Nothing };
                return Step { action, write_needed: true };
            }
            Step { action: Action::Nothing, write_needed: false }
        }
    }
}

fn tier_from_u8(v: u8) -> Tier {
    match v {
        1 => Tier::Light, 2 => Tier::Moderate, 3 => Tier::Heavy, 4 => Tier::Violent,
        _ => Tier::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MIN: i64 = 60;

    #[test]
    fn alerts_once_then_stays_silent_across_ticks() {
        let mut st = EventState::default();
        let s = step(&mut st, Some((Tier::Moderate, 18)), 0, 30, true);
        assert_eq!(s.action, Action::Open { tier: Tier::Moderate, eta_min: 18 });
        assert!(s.write_needed);

        // Seven consecutive 3-minute ticks still matching -> no further alerts.
        for i in 1..=7 {
            let s = step(&mut st, Some((Tier::Moderate, 18 - i as u32)), i * 3 * MIN, 30, true);
            assert_eq!(s.action, Action::Nothing, "tick {i} must not re-alert");
            assert!(!s.write_needed, "tick {i} must not write to KV");
        }
    }

    #[test]
    fn re_alerts_on_escalation_only_upward() {
        let mut st = EventState::default();
        step(&mut st, Some((Tier::Light, 20)), 0, 30, true);
        let up = step(&mut st, Some((Tier::Heavy, 8)), 3 * MIN, 30, true);
        assert_eq!(up.action, Action::Escalate { from: Tier::Light, to: Tier::Heavy, eta_min: 8 });

        let down = step(&mut st, Some((Tier::Light, 2)), 6 * MIN, 30, true);
        assert_eq!(down.action, Action::Nothing, "de-escalation is silent");
        assert_eq!(st.tier, Tier::Heavy as u8, "peak tier is retained");
    }

    #[test]
    fn closes_only_after_the_quiet_gap() {
        let mut st = EventState::default();
        step(&mut st, Some((Tier::Moderate, 10)), 0, 30, true);
        assert_eq!(step(&mut st, None, 20 * MIN, 30, true).action, Action::Nothing);
        assert!(st.open, "20 min of quiet is inside a 30 min close window");

        let closed = step(&mut st, None, 30 * MIN, 30, true);
        assert_eq!(closed.action, Action::Close { duration_min: 0 });
        assert!(!st.open);
    }

    #[test]
    fn all_clear_disabled_closes_silently_but_still_persists() {
        let mut st = EventState::default();
        step(&mut st, Some((Tier::Moderate, 10)), 0, 30, false);
        let closed = step(&mut st, None, 31 * MIN, 30, false);
        assert_eq!(closed.action, Action::Nothing);
        assert!(closed.write_needed, "state must still be cleared in KV");
    }

    #[test]
    fn quiet_ticks_with_no_open_event_never_write() {
        let mut st = EventState::default();
        for t in 0..50 {
            let s = step(&mut st, None, t * 3 * MIN, 30, true);
            assert!(!s.write_needed);
        }
    }
}
