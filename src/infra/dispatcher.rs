//! Dispatcher for Cloudflare Worker entry points: routing HTTP requests and cron events.

use crate::domain::util::ct_eq;
use crate::infra::{http::HttpGateway, settings::Settings, store::Store, tasks};
use worker::{console_error, Context, Env, Method, Request, Response, Result, ScheduleContext, ScheduledEvent};

pub struct AppContext {
  pub settings: Settings,
  pub store: Store,
}

impl AppContext {
  pub fn load(env: &Env) -> Result<Self> {
    let settings = Settings::load(env)?;
    let store = Store::new(env)?;
    Ok(Self { settings, store })
  }
}

pub struct Dispatcher;

impl Dispatcher {
  /// Cron trigger dispatch.
  pub async fn dispatch_scheduled(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let cron = event.cron();
    let ctx = match AppContext::load(&env) {
      Ok(c) => c,
      Err(e) => {
        console_error!("cron {cron} failed loading settings: {e}");
        return;
      }
    };
    let result = match cron.as_str() {
      c if c.starts_with("*/3 ") => tasks::rain_tick(&ctx).await,
      c if c.starts_with("*/10 ") => tasks::warning_tick(&ctx).await,
      "0 0 * * *" => tasks::daily_tick(&ctx).await,
      other => {
        console_error!("unrecognised cron {other}; no handler ran");
        Ok(())
      }
    };
    if let Err(e) = result {
      console_error!("cron {cron} failed: {e}");
    }
  }

  /// HTTP fetch dispatch.
  pub async fn dispatch_fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // Answered before config loads: a liveness probe that fails on an
    // incomplete config cannot tell you the Worker is running.
    if req.method() == Method::Get && req.path() == "/health" {
      return Response::ok("ok");
    }

    let ctx = match AppContext::load(&env) {
      Ok(c) => c,
      Err(e) => {
        console_error!("settings: {e}");
        return Response::error("misconfigured", 500);
      }
    };
    let s = &ctx.settings;

    match (req.method(), req.path().as_str()) {
      (Method::Get, "/api/report") => {
        if s.api_token.is_empty() || s.api_token.starts_with(crate::infra::settings::SENTINEL) {
          return Response::error("unauthorized", 401);
        }
        let ok = req.headers().get("authorization").ok().flatten().map(|h| ct_eq(&h, &format!("Bearer {}", s.api_token))).unwrap_or(false);
        if !ok {
          return Response::error("unauthorized", 401);
        }
        let text = tasks::build_report(&ctx).await?;
        Response::from_json(&serde_json::json!({ "report": text }))
      }

      (Method::Post, "/webhook/telegram") => {
        // Secret first, then chat_id. The chat_id allowlist is the load
        // bearing half: without it anyone who finds the bot can rewrite
        // the rule table and drain the daily quota.
        if s.tg_webhook_secret.is_empty() || s.tg_webhook_secret.starts_with(crate::infra::settings::SENTINEL) {
          return Response::empty();
        }
        let secret_ok = req.headers().get("x-telegram-bot-api-secret-token").ok().flatten().map(|h| ct_eq(&h, &s.tg_webhook_secret)).unwrap_or(false);
        if !secret_ok {
          return Response::empty();
        }

        let update: serde_json::Value = match req.json().await {
          Ok(v) => v,
          Err(_) => return Response::empty(),
        };
        let msg = update.get("message").or_else(|| update.get("edited_message"));
        let chat_id = msg.and_then(|m| m.get("chat")).and_then(|c| c.get("id")).map(|v| v.to_string()).unwrap_or_default();
        if s.owner_chat_id.is_empty() || !ct_eq(&chat_id, &s.owner_chat_id) {
          return Response::empty();
        }

        let text = msg.and_then(|m| m.get("text")).and_then(|t| t.as_str()).unwrap_or("");
        // Redacted: a propagated fetch error can quote the request URL,
        // which carries the Tomorrow.io key as a query parameter.
        let reply = tasks::handle_command(&ctx, text).await.unwrap_or_else(|e| crate::domain::util::redact(&format!("error: {e}"), &[&s.tomorrow_key, &s.tg_token, &s.api_token]));
        let _ = HttpGateway::send_telegram(&s.tg_token, &s.owner_chat_id, &reply).await;
        Response::empty()
      }

      _ => Response::error("not found", 404),
    }
  }
}
