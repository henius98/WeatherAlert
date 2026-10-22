use crate::domain::render::TG_LIMIT;
use worker::{wasm_bindgen::JsValue, Fetch, Headers, Method, Request, RequestInit, Result};

pub async fn send(token: &str, chat_id: &str, text: &str) -> Result<u16> {
  let text = crate::domain::render::truncate(text.to_string(), TG_LIMIT);
  let payload = serde_json::json!({
      "chat_id": chat_id,
      "text": text,
      "disable_web_page_preview": true,
  })
  .to_string();

  let headers = Headers::new();
  headers.set("content-type", "application/json")?;
  let mut init = RequestInit::new();
  init.with_method(Method::Post).with_headers(headers).with_body(Some(JsValue::from_str(&payload)));

  let url = format!("https://api.telegram.org/bot{token}/sendMessage");
  let req = Request::new_with_init(&url, &init)?;
  let resp = Fetch::Request(req).send().await?;
  Ok(resp.status_code())
}
