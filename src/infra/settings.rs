use worker::{Env, Result};

pub struct Settings {
  pub lat: String,
  pub lon: String,
  pub label: String,
  pub district: String,
  pub district_name: String,
  pub state_name: String,
  pub severity_keywords: String,
  pub window_start: i64,
  pub window_end: i64,
  pub met_observe: bool,
  pub close_min: i64,
  pub all_clear: bool,
  pub manual_per_hour: u32,
  pub tomorrow_key: String,
  pub tg_token: String,
  pub tg_webhook_secret: String,
  pub api_token: String,
  pub owner_chat_id: String,
}

fn var(env: &Env, k: &str, default: &str) -> String {
  env.var(k).map(|v| v.to_string()).unwrap_or_else(|_| default.to_string())
}

fn secret(env: &Env, k: &str) -> Result<String> {
  Ok(env.secret(k)?.to_string())
}

/// Matches the placeholder convention used for KV ids in wrangler.toml.
pub const SENTINEL: &str = "REPLACE_ME";

impl Settings {
  /// Names the first secret still holding a placeholder, so an unconfigured
  /// deploy reports plainly instead of making authenticated calls with a
  /// credential we already know is wrong.
  pub fn unconfigured(&self) -> Option<&'static str> {
    for (name, v) in [("TOMORROW_KEY", &self.tomorrow_key), ("TG_TOKEN", &self.tg_token), ("OWNER_CHAT_ID", &self.owner_chat_id)] {
      if v.is_empty() || v.starts_with(SENTINEL) {
        return Some(name);
      }
    }
    None
  }

  pub fn load(env: &Env) -> Result<Self> {
    Ok(Settings {
      lat: var(env, "LAT", "0"),
      lon: var(env, "LON", "0"),
      label: var(env, "LOC_LABEL", "location"),
      district: var(env, "MET_DISTRICT_ID", ""),
      district_name: var(env, "MET_DISTRICT_NAME", ""),
      state_name: var(env, "MET_STATE_NAME", ""),
      severity_keywords: var(env, "MET_SEVERITY_KEYWORDS", ""),
      window_start: var(env, "WINDOW_START", "9").parse().unwrap_or(9),
      window_end: var(env, "WINDOW_END", "23").parse().unwrap_or(23),
      met_observe: var(env, "MET_MODE", "observe") != "filter",
      close_min: var(env, "EVENT_CLOSE_MIN", "30").parse().unwrap_or(30),
      all_clear: var(env, "ALL_CLEAR", "true") == "true",
      manual_per_hour: var(env, "MANUAL_CALLS_PER_HOUR", "4").parse().unwrap_or(4),
      tomorrow_key: secret(env, "TOMORROW_KEY")?,
      tg_token: secret(env, "TG_TOKEN")?,
      tg_webhook_secret: secret(env, "TG_WEBHOOK_SECRET")?,
      api_token: secret(env, "API_TOKEN")?,
      owner_chat_id: secret(env, "OWNER_CHAT_ID")?,
    })
  }
}
