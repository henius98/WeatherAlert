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
