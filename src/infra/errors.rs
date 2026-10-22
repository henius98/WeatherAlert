//! Deep error recorder: classifies errors, throttles user alerts, redacts secrets,
//! and persists dirty error state to KV atomically.

use crate::domain::errors::{classify, Class, ErrState};
use crate::infra::{settings::Settings, store::Store};
use worker::console_error;

pub struct ErrorRecorder<'a> {
  settings: &'a Settings,
  store: &'a Store,
  errs: ErrState,
  now: i64,
}

impl<'a> ErrorRecorder<'a> {
  pub async fn new(settings: &'a Settings, store: &'a Store, now: i64) -> Self {
    let errs = store.errs().await.unwrap_or_default();
    Self { settings, store, errs, now }
  }

  /// Sends a secret-redacted notification to Telegram via the single notify chokepoint.
  pub async fn notify(&self, text: &str) {
    crate::infra::tasks::notify(self.settings, text).await;
  }

  /// Records a successful upstream fetch. If the feed was previously failing, sends recovery notice.
  pub async fn report_success(&mut self) {
    if self.errs.note_success() {
      self.notify("\u{2705} Weather feed recovered.").await;
    }
    self.flush_dirty().await;
  }

  /// Records an HTTP status failure from Tomorrow.io. Classifies, throttles, and alerts.
  pub async fn report_http(&mut self, status: u16) {
    self.errs.note_failure();
    let c = classify(Some(status), self.errs.consecutive - 1);
    if self.errs.should_send(c, self.now) {
      self.notify(&format!("\u{26A0}\u{FE0F} Tomorrow.io HTTP {status} ({})", c.key())).await;
    }
    self.flush_dirty().await;
  }

  /// Records a transport fetch failure.
  pub async fn report_fetch_error(&mut self, err: &impl std::fmt::Display) {
    self.errs.note_failure();
    let c = classify(None, self.errs.consecutive - 1);
    if self.errs.should_send(c, self.now) {
      self.notify(&format!("\u{26A0}\u{FE0F} Weather fetch failed: {err}")).await;
    }
    self.flush_dirty().await;
  }

  /// Records an infrastructure error (Storage or Config) with custom message.
  /// Does NOT increment feed failure count — storage/config errors do not pollute the feed streak.
  pub async fn report_class(&mut self, class: Class, msg: &str) {
    if self.errs.should_send(class, self.now) {
      self.notify(msg).await;
    }
    self.flush_dirty().await;
  }

  /// Flushes `ErrState` to KV if dirty.
  pub async fn flush_dirty(&mut self) {
    if self.errs.dirty {
      if let Err(e) = self.store.put_errs(&self.errs).await {
        console_error!("kv write failed (st:err): {e}");
      }
      self.errs.dirty = false;
    }
  }
}
