//! data.gov.my client.
//!
//! Both endpoints 301 to a trailing slash, so the slash is hardcoded rather
//! than relying on redirect following. The forecast is filtered server-side:
//! `contains=` plus a date range returns 2 records instead of 2000, which is
//! the difference between fitting the 10ms CPU budget and not.

use crate::infra::http::HttpGateway;
use worker::Result;

pub async fn warnings() -> Result<(String, u16)> {
  let resp = HttpGateway::fetch_warnings().await?;
  Ok((resp.body, resp.status))
}

pub async fn forecast(district: &str, date_start: &str, date_end: &str) -> Result<(String, u16)> {
  let resp = HttpGateway::fetch_forecast(district, date_start, date_end).await?;
  Ok((resp.body, resp.status))
}
