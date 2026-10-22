//! data.gov.my client.
//!
//! Both endpoints 301 to a trailing slash, so the slash is hardcoded rather
//! than relying on redirect following. The forecast is filtered server-side:
//! `contains=` plus a date range returns 2 records instead of 2000, which is
//! the difference between fitting the 10ms CPU budget and not.

use worker::{Fetch, Method, Request, Result};

const BASE: &str = "https://api.data.gov.my/weather";

async fn get(url: &str) -> Result<(String, u16)> {
  let req = Request::new(url, Method::Get)?;
  let mut resp = Fetch::Request(req).send().await?;
  let status = resp.status_code();
  let body = resp.text().await.unwrap_or_default();
  Ok((body, status))
}

pub async fn warnings() -> Result<(String, u16)> {
  get(&format!("{BASE}/warning/")).await
}

pub async fn forecast(district: &str, date_start: &str, date_end: &str) -> Result<(String, u16)> {
  let q = crate::domain::forecast::forecast_query(district, date_start, date_end);
  get(&format!("{BASE}/forecast/?{q}")).await
}
