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
    /// `onset_min` is when it starts, `peak_*` is how bad it gets. Both are
    /// reported: the onset alone understates a storm behind drizzle.
    Open { peak_tier: Tier, onset_min: u32, peak_min: u32 },
    Escalate { from: Tier, to: Tier, onset_min: u32, peak_min: u32 },
    Close { duration_min: i64 },
}

/// `write_needed` is the whole point: KV free tier allows 1000 writes/day and
/// the rain cron ticks 280 times, so we only persist on a transition.
pub struct Step { pub action: Action, pub write_needed: bool }

pub fn step(
    st: &mut EventState,
    matched: Option<(Tier, u32, u32)>,
    now: i64,
    close_after_min: i64,
    all_clear: bool,
) -> Step {
    match matched {
        Some((peak_tier, onset_min, peak_min)) => {
            if !st.open {
                *st = EventState {
                    open: true, tier: peak_tier as u8, opened_at: now,
                    last_match_at: now, rule_ids: st.rule_ids.clone(),
                };
                return Step {
                    action: Action::Open { peak_tier, onset_min, peak_min },
                    write_needed: true,
                };
            }
            st.last_match_at = now;
            let prev = st.tier;
            // The ratchet tracks the PEAK, so a forecast that worsens anywhere in
            // the window escalates even if onset time has not moved.
            if (peak_tier as u8) > prev {
                st.tier = peak_tier as u8;
                let from = tier_from_u8(prev);
                return Step {
                    action: Action::Escalate { from, to: peak_tier, onset_min, peak_min },
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
