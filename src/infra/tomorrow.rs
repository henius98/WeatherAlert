use crate::domain::util::urlencode;
use worker::{Fetch, Headers, Method, Request, RequestInit, Result};

pub struct Fetched {
  pub body: String,
  pub status: u16,
}

/// One Timelines call for a 30-minute window at 1-minute resolution.
///
/// The window is deliberately narrow: the 1m timestep permits a 24-hour range,
/// which is ~1440 points, and deserializing thatevery tick threatens the 10ms CPU
/// limit on the free plan. 30 points is the design.
pub async fn timeline(lat: &str, lon: &str, fields: &[String], key: &str, window_min: u32) -> Result<Fetched> {
  let url = format!(
    "https://api.tomorrow.io/v4/timelines?location={}&fields={}&timesteps=1m&startTime=now&endTime=nowPlus{}m&units=metric",
    urlencode(&format!("{lat},{lon}")),
    urlencode(&fields.join(",")),
    window_min,
  );
  let headers = Headers::new();
  headers.set("apikey", key)?;
  let mut init = RequestInit::new();
  init.with_method(Method::Get).with_headers(headers);
  let req = Request::new_with_init(&url, &init)?;
  let mut resp = Fetch::Request(req).send().await?;
  let status = resp.status_code();
  let body = resp.text().await.unwrap_or_default();
  Ok(Fetched { body, status })
}
