use crate::infra::http::HttpGateway;
use worker::Result;

pub async fn send(token: &str, chat_id: &str, text: &str) -> Result<u16> {
  HttpGateway::send_telegram(token, chat_id, text).await
}
