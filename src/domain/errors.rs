//! Error classification and the send-throttle.
//!
//! Without a throttle a six-hour upstream outage sends 120 identical Telegram
//! messages, one per 3-minute tick.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Auth,
    QuotaSustained,
    Storage,
    Config,
    SustainedFailure,
    Transient,
}

impl Class {
    pub fn key(self) -> &'static str {
        match self {
            Class::Auth => "auth",
            Class::QuotaSustained => "quota",
            Class::Storage => "storage",
            Class::Config => "config",
            Class::SustainedFailure => "sustained",
            Class::Transient => "transient",
        }
    }
    /// Transient failures stay silent; everything else is worth a message.
    pub fn is_notifiable(self) -> bool {
        !matches!(self, Class::Transient)
    }
}

/// `consecutive_failures` is the tick-failure counter *before* this attempt.
pub fn classify(status: Option<u16>, consecutive_failures: u32) -> Class {
    match status {
        Some(401) | Some(403) => Class::Auth,
        // A single 429 is absorbed; only a sustained one is worth waking someone.
        Some(429) if consecutive_failures >= 2 => Class::QuotaSustained,
        Some(429) => Class::Transient,
        _ if consecutive_failures >= 5 => Class::SustainedFailure,
        _ => Class::Transient,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrState {
    #[serde(default)]
    pub last_sent: BTreeMap<String, i64>,
    #[serde(default)]
    pub consecutive: u32,
    /// Not persisted. The rain cron ticks 280x/day; writing this state on every
    /// tick would spend 280 of the 1000/day KV write budget for nothing.
    #[serde(skip)]
    pub dirty: bool,
}

pub const THROTTLE_SECS: i64 = 3600;

impl ErrState {
    /// One message per class per hour. Returns false when suppressed.
    pub fn should_send(&mut self, c: Class, now: i64) -> bool {
        if !c.is_notifiable() {
            return false;
        }
        let k = c.key().to_string();
        match self.last_sent.get(&k) {
            Some(&t) if now - t < THROTTLE_SECS => false,
            _ => {
                self.last_sent.insert(k, now);
                self.dirty = true;
                true
            }
        }
    }

    /// True when a recovery message is owed: we had been reporting failures.
    pub fn note_success(&mut self) -> bool {
        let was_failing = self.consecutive >= 5;
        if self.consecutive != 0 {
            self.dirty = true;
        }
        self.consecutive = 0;
        if was_failing {
            self.last_sent.clear();
        }
        was_failing
    }

    pub fn note_failure(&mut self) {
        self.consecutive = self.consecutive.saturating_add(1);
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failures_always_notify() {
        assert_eq!(classify(Some(401), 0), Class::Auth);
        assert_eq!(classify(Some(403), 0), Class::Auth);
        assert!(Class::Auth.is_notifiable());
    }

    #[test]
    fn a_single_429_is_absorbed_but_a_sustained_one_is_not() {
        assert_eq!(classify(Some(429), 0), Class::Transient);
        assert_eq!(classify(Some(429), 1), Class::Transient);
        assert_eq!(classify(Some(429), 2), Class::QuotaSustained);
    }

    #[test]
    fn transient_5xx_stays_silent_until_it_is_sustained() {
        assert_eq!(classify(Some(503), 0), Class::Transient);
        assert_eq!(classify(Some(503), 4), Class::Transient);
        assert_eq!(classify(Some(503), 5), Class::SustainedFailure);
    }

    #[test]
    fn a_six_hour_outage_sends_one_message_per_hour_not_one_per_tick() {
        let mut st = ErrState::default();
        let mut sent = 0;
        // 120 ticks at 3-minute spacing == 6 hours.
        for tick in 0..120i64 {
            let now = tick * 180;
            st.note_failure();
            let c = classify(Some(500), st.consecutive - 1);
            if st.should_send(c, now) {
                sent += 1;
            }
        }
        assert_eq!(sent, 6, "expected hourly, got {sent}");
    }

    #[test]
    fn recovery_is_reported_once_then_the_throttle_resets() {
        let mut st = ErrState::default();
        for _ in 0..5 {
            st.note_failure();
        }
        assert!(st.note_success(), "recovery owed after sustained failure");
        assert!(!st.note_success(), "not owed twice");
        assert_eq!(st.consecutive, 0);
    }

    #[test]
    fn a_clean_tick_costs_no_kv_write() {
        let mut st = ErrState::default();
        // Healthy tick: nothing failed, nothing sent, nothing to persist.
        assert!(!st.note_success());
        assert!(!st.dirty, "a healthy tick must not dirty the error state");
        // 280 healthy ticks/day must stay at zero writes.
        for _ in 0..280 {
            st.note_success();
        }
        assert!(!st.dirty);
    }

    #[test]
    fn failures_and_sends_mark_state_dirty() {
        let mut st = ErrState::default();
        st.note_failure();
        assert!(st.dirty);

        let mut st2 = ErrState::default();
        assert!(st2.should_send(Class::Auth, 0));
        assert!(st2.dirty, "a sent message updates the throttle and must persist");

        let mut st3 = ErrState::default();
        st3.note_failure();
        st3.dirty = false;
        assert!(!st3.note_success(), "recovery not owed below the threshold");
        assert!(st3.dirty, "clearing a failure streak must persist");
    }

    #[test]
    fn transient_class_never_sends_even_on_first_occurrence() {
        let mut st = ErrState::default();
        assert!(!st.should_send(Class::Transient, 0));
    }
}
