//! Unified HTTP gateway for outbound requests across external endpoints:
//! Tomorrow.io, data.gov.my, and Telegram.

use crate::domain::render::TG_LIMIT;
use crate::domain::util::urlencode;
use worker::{wasm_bindgen::JsValue, Fetch, Headers, Method, Request, RequestInit, Result};

pub struct HttpResponse {
  pub status: u16,
  pub body: String,
}

pub struct HttpGateway;

impl HttpGateway {
  pub async fn get(url: &str) -> Result<HttpResponse> {
    let req = Request::new(url, Method::Get)?;
    let mut resp = Fetch::Request(req).send().await?;
    let status = resp.status_code();
    let body = resp.text().await.unwrap_or_default();
    Ok(HttpResponse { status, body })
  }

  pub async fn get_with_header(url: &str, header_name: &str, header_val: &str) -> Result<HttpResponse> {
    let headers = Headers::new();
    headers.set(header_name, header_val)?;
    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(headers);
    let req = Request::new_with_init(url, &init)?;
    let mut resp = Fetch::Request(req).send().await?;
    let status = resp.status_code();
    let body = resp.text().await.unwrap_or_default();
    Ok(HttpResponse { status, body })
  }

  pub async fn post_json(url: &str, json_body: &str) -> Result<HttpResponse> {
    let headers = Headers::new();
    headers.set("content-type", "application/json")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_headers(headers).with_body(Some(JsValue::from_str(json_body)));
    let req = Request::new_with_init(url, &init)?;
    let mut resp = Fetch::Request(req).send().await?;
    let status = resp.status_code();
    let body = resp.text().await.unwrap_or_default();
    Ok(HttpResponse { status, body })
  }

  pub async fn fetch_timeline(lat: &str, lon: &str, fields: &[String], key: &str, window_min: u32) -> Result<HttpResponse> {
    let url = format!(
      "https://api.tomorrow.io/v4/timelines?location={}&fields={}&timesteps=1m&startTime=now&endTime=nowPlus{}m&units=metric",
      urlencode(&format!("{lat},{lon}")),
      urlencode(&fields.join(",")),
      window_min,
    );
    Self::get_with_header(&url, "apikey", key).await
  }

  pub async fn fetch_warnings() -> Result<HttpResponse> {
    Self::get("https://api.data.gov.my/weather/warning/").await
  }

  pub async fn fetch_forecast(district: &str, date_start: &str, date_end: &str) -> Result<HttpResponse> {
    let q = crate::domain::forecast::forecast_query(district, date_start, date_end);
    Self::get(&format!("https://api.data.gov.my/weather/forecast/?{q}")).await
  }

  pub async fn send_telegram(token: &str, chat_id: &str, text: &str) -> Result<u16> {
    let text = crate::domain::render::truncate(text.to_string(), TG_LIMIT);
    let payload = serde_json::json!({
      "chat_id": chat_id,
      "text": text,
      "disable_web_page_preview": true,
    })
    .to_string();
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let resp = Self::post_json(&url, &payload).await?;
    Ok(resp.status)
  }
}
