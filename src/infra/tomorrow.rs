use crate::infra::http::HttpGateway;
use worker::Result;

pub struct Fetched {
  pub body: String,
  pub status: u16,
}

/// One Timelines call for a 30-minute window at 1-minute resolution.
pub async fn timeline(lat: &str, lon: &str, fields: &[String], key: &str, window_min: u32) -> Result<Fetched> {
  let resp = HttpGateway::fetch_timeline(lat, lon, fields, key, window_min).await?;
  Ok(Fetched { body: resp.body, status: resp.status })
}
