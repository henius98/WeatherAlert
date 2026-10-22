//! Deep error recorder: classifies errors, throttles user alerts, redacts secrets,
//! and persists dirty error state to KV atomically.

use crate::domain::errors::{classify, Class, ErrState};
use crate::infra::{http::HttpGateway, settings::Settings, store::Store};
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

  /// Sends a secret-redacted notification to Telegram.
  pub async fn notify(&self, text: &str) {
    let safe = crate::domain::util::redact(
      text,
      &[&self.settings.tomorrow_key, &self.settings.tg_token, &self.settings.api_token, &self.settings.tg_webhook_secret],
    );
    if let Err(e) = HttpGateway::send_telegram(&self.settings.tg_token, &self.settings.owner_chat_id, &safe).await {
      console_error!("telegram send failed: {e}");
    }
  }

  /// Records a successful upstream fetch. If the feed was previously failing, sends recovery notice.
  pub async fn report_success(&mut self) {
    if self.errs.note_success() {
      self.notify("\u{2705} Weather feed recovered.").await;
    }
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

  /// Records a specific error class with custom message.
  pub async fn report_class(&mut self, class: Class, msg: &str) {
    self.errs.note_failure();
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
