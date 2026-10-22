use crate::domain::commands::{self, Command, HELP};
use crate::domain::daily::{morning_summary, DailyLog};
use crate::domain::errors::Class;
use crate::domain::forecast;
use crate::domain::met::{self, format_observe, Warning};
use crate::domain::render;
use crate::domain::rules::Rule;
use crate::domain::state::evaluate_tick;
use crate::domain::tiers::Tier;
use crate::domain::timeutil::{in_active_window, ymd_myt};
use crate::domain::tomorrow::{parse_timeline, required_fields, window_minutes};
use crate::infra::{dispatcher::AppContext, errors::ErrorRecorder, http::HttpGateway, settings::Settings, store::Store};
use worker::{console_error, console_log, Date, Result};

pub fn now_secs() -> i64 {
  (Date::now().as_millis() / 1000) as i64
}

/// The single chokepoint for user-visible text. Redaction lives here rather
/// than at each call site so a future error path cannot forget it.
pub(crate) async fn notify(s: &Settings, text: &str) {
  let safe = crate::domain::util::redact(text, &[&s.tomorrow_key, &s.tg_token, &s.api_token, &s.tg_webhook_secret]);
  if let Err(e) = HttpGateway::send_telegram(&s.tg_token, &s.owner_chat_id, &safe).await {
    console_error!("telegram send failed: {e}");
  }
}

/// KV writes are best-effort on non-critical paths, but a silent drop
/// contradicts the documented error policy, so every failure is logged.
fn log_kv(what: &str, r: Result<()>) {
  if let Err(e) = r {
    console_error!("kv write failed ({what}): {e}");
  }
}

/// Rain path. Runs only inside the active window; one Tomorrow.io call per tick.
pub async fn rain_tick(ctx: &AppContext) -> Result<()> {
  let s = &ctx.settings;
  if let Some(missing) = s.unconfigured() {
    console_error!("rain_tick skipped: {missing} is not configured");
    return Ok(());
  }
  let now = now_secs();
  if !in_active_window(now, s.window_start, s.window_end) {
    return Ok(());
  }

  let mut recorder = ErrorRecorder::new(s, &ctx.store, now).await;

  // A corrupt cfg:rules must never degrade to an empty table. Doing so would
  // leave the Worker ticking, spending quota, and never alerting — a silent
  // total failure with nothing in the logs.
  let rules = match ctx.store.rules().await {
    Ok(r) => r,
    Err(e) => {
      recorder.report_class(Class::Config, &format!("\u{26A0}\u{FE0F} Rule table unreadable: {e}\nNo rain alerts will fire until this is fixed.")).await;
      return Ok(());
    }
  };
  if rules.is_empty() {
    // No rules means nothing can fire, so skip the fetch and keep the quota.
    console_error!("cfg:rules is empty; no alerts possible, skipping fetch");
    return Ok(());
  }

  let fields = required_fields(&rules);
  let window = window_minutes(&rules);
  let fetched = HttpGateway::fetch_timeline(&s.lat, &s.lon, &fields, &s.tomorrow_key, window).await;

  let body = match fetched {
    Ok(f) if f.status == 200 => {
      recorder.report_success().await;
      f.body
    }
    Ok(f) => {
      recorder.report_http(f.status).await;
      return Ok(());
    }
    Err(e) => {
      recorder.report_fetch_error(&e).await;
      return Ok(());
    }
  };

  let timeline = match parse_timeline(&body, now) {
    Ok(t) => t,
    Err(e) => {
      recorder.report_class(Class::Config, &format!("\u{26A0}\u{FE0F} Timeline parse failed: {e}")).await;
      return Ok(());
    }
  };

  // Cache for /now fallback. Cheap: expires in an hour, one write per tick,
  // still far inside the 1000/day KV budget alongside ~20 state writes.
  log_kv("st:snapshot", ctx.store.put_snapshot(&body).await);

  let mut ev = ctx.store.event().await.unwrap_or_default();
  let plan = evaluate_tick(&rules, &timeline, &mut ev, now, s.close_min, s.all_clear, &s.label);

  if let Some(msg) = plan.alert_message {
    recorder.notify(&msg).await;
  }
  // Tally for the morning summary, read and written only upon Open or Escalate.
  if let Some((peak_tier, escalation)) = plan.daily_record {
    let mut daily = ctx.store.daily().await.unwrap_or_default();
    daily.record(&ymd_myt(now), peak_tier, escalation);
    log_kv("st:daily", ctx.store.put_daily(&daily).await);
  }
  if plan.write_event {
    if let Err(e) = ctx.store.put_event(&ev).await {
      console_error!("kv write failed (st:event): {e}");
      recorder.report_class(Class::Storage, &format!("\u{26A0}\u{FE0F} State storage write failed: {e}")).await;
      return Err(e);
    }
  }
  recorder.flush_dirty().await;
  Ok(())
}

/// Disaster path. Runs 24/7 and is never silenced by the active window.
pub async fn warning_tick(ctx: &AppContext) -> Result<()> {
  let s = &ctx.settings;
  if let Some(missing) = s.unconfigured() {
    console_error!("warning_tick skipped: {missing} is not configured");
    return Ok(());
  }
  let now = now_secs();

  let resp = match HttpGateway::fetch_warnings().await {
    Ok(v) => v,
    Err(e) => {
      console_log!("met warning fetch failed: {e}");
      return Ok(());
    }
  };
  if resp.status != 200 {
    console_log!("met warning HTTP {}", resp.status);
    return Ok(());
  }

  let warnings: Vec<Warning> = match serde_json::from_str(&resp.body) {
    Ok(v) => v,
    Err(e) => {
      console_log!("met warning parse failed: {e}");
      return Ok(());
    }
  };

  let st = ctx.store.met().await.unwrap_or_default();
  let (selected, next) = met::select_observe(&warnings, &st, now);
  if selected.is_empty() {
    return Ok(());
  }

  let sev: Vec<&str> = if !s.met_observe { s.severity_keywords.split(',').map(str::trim).filter(|x| !x.is_empty()).collect() } else { Vec::new() };
  let places: Vec<&str> = if !s.met_observe { [s.state_name.as_str(), s.district_name.as_str()].into_iter().filter(|x| !x.is_empty()).collect() } else { Vec::new() };

  for i in selected {
    let w = &warnings[i];
    if s.met_observe {
      // Observe mode sends the compact form: the severity vocabulary lives
      // in the title and heading, not in 1.5 KB of prose.
      notify(s, &met::format_summary(w)).await;
    } else if met::passes_filter(w, &sev, &places) {
      // Filter mode gates on severity keywords harvested from real payloads
      // plus place NAMES (ids like "St002" never appear in MET prose).
      notify(s, &format_observe(w, render::TG_LIMIT)).await;
    }
  }
  ctx.store.put_met(&next).await?;
  Ok(())
}

/// 08:00 MYT summary. Doubles as the liveness signal: if this stops arriving,
/// the monitor is down. Costs one Telegram send and two unmetered MET calls.
pub async fn daily_tick(ctx: &AppContext) -> Result<()> {
  let s = &ctx.settings;
  if let Some(missing) = s.unconfigured() {
    console_error!("daily_tick skipped: {missing} is not configured");
    return Ok(());
  }
  let now = now_secs();
  let today = ymd_myt(now);

  // The forecast is context, not a trigger, so a failure degrades to an empty
  // brief rather than aborting — but it is logged. An unlogged fallback hid a
  // malformed query behind a brief that merely looked sparse.
  let days = match HttpGateway::fetch_forecast(&s.district, &today, &today).await {
    Ok(resp) if resp.status == 200 => forecast::parse(&resp.body, &s.district, 1).unwrap_or_default(),
    Ok(resp) => {
      console_error!("daily forecast HTTP {}: {}", resp.status, resp.body.chars().take(200).collect::<String>());
      Vec::new()
    }
    Err(e) => {
      console_error!("daily forecast fetch failed: {e}");
      Vec::new()
    }
  };
  let warn_lines: Vec<String> = match HttpGateway::fetch_warnings().await {
    Ok(resp) if resp.status == 200 => met::active_titles(&resp.body, now),
    _ => Vec::new(),
  };

  let prev = ctx.store.daily().await.unwrap_or_default();
  notify(s, &morning_summary(&s.label, &today, &days, &warn_lines, &prev, (s.window_start, s.window_end))).await;

  // Start today's tally fresh so the count always covers one civil day.
  log_kv("st:daily", ctx.store.put_daily(&DailyLog { date: today, ..Default::default() }).await);
  Ok(())
}

/// Shared body for `/now` and `GET /api/report`.
pub async fn build_report(ctx: &AppContext) -> Result<String> {
  let s = &ctx.settings;
  if let Some(missing) = s.unconfigured() {
    return Ok(format!("Not configured yet: {missing} is unset. See DEPLOY.md."));
  }
  let now = now_secs();
  // Same invariant as the rain path: a corrupt rule table is reported, not
  // silently defaulted. Here it only selects query fields, but a report that
  // looks normal while the rules are unreadable is still a lie.
  let rules = match ctx.store.rules().await {
    Ok(r) => r,
    Err(e) => return Ok(format!("Rule table unreadable: {e}\nNo rain alerts are firing.")),
  };

  let mut quota = ctx.store.quota().await.unwrap_or_default();
  let allow_live = quota.try_spend(now, s.manual_per_hour);
  let mut stale = false;

  let body = if allow_live {
    log_kv("st:quota", ctx.store.put_quota(&quota).await);
    let w = window_minutes(&rules);
    match HttpGateway::fetch_timeline(&s.lat, &s.lon, &required_fields(&rules), &s.tomorrow_key, w).await {
      Ok(resp) if resp.status == 200 => resp.body,
      _ => {
        stale = true;
        ctx.store.snapshot().await.unwrap_or_default().unwrap_or_default()
      }
    }
  } else {
    stale = true;
    ctx.store.snapshot().await.unwrap_or_default().unwrap_or_default()
  };

  let timeline = parse_timeline(&body, now).unwrap_or_default();
  // An empty snapshot (KV miss, expired TTL, first run) must report NO DATA
  // rather than an all-clear.
  let have_data = !timeline.is_empty();
  // One lookup carrying its value: indexing the map separately would panic if
  // the field were absent, and would be safe only by coupling to this predicate.
  let next = timeline.iter().find_map(|m| {
    let v = m.fields.get("rainIntensity").copied().unwrap_or(0.0);
    (v >= 0.25).then_some((m.offset_min, v))
  });
  let (tier, eta) = match next {
    Some((off, v)) => (Tier::from_rain_intensity(v), Some(off)),
    None => (Tier::None, None),
  };

  let days = match HttpGateway::fetch_forecast(&s.district, &ymd_myt(now), &ymd_myt(now + 86_400)).await {
    Ok(resp) if resp.status == 200 => forecast::parse(&resp.body, &s.district, 2).unwrap_or_default(),
    Ok(resp) => {
      console_error!("report forecast HTTP {}: {}", resp.status, resp.body.chars().take(200).collect::<String>());
      Vec::new()
    }
    Err(e) => {
      console_error!("report forecast fetch failed: {e}");
      Vec::new()
    }
  };

  let warn_lines = match HttpGateway::fetch_warnings().await {
    Ok(resp) if resp.status == 200 => met::active_titles(&resp.body, now),
    _ => Vec::new(),
  };

  Ok(render::report(&s.label, tier, eta, &days, &warn_lines, stale, have_data))
}

/// Loads the rule table for a command that may write it back.
///
/// Defaulting here would be destructive, not merely lossy: `/rules add` against
/// an unreadable table would default to empty, push one rule, and persist that —
/// silently deleting every existing rule.
async fn rules_for_edit(store: &Store) -> std::result::Result<Vec<Rule>, String> {
  store.rules().await.map_err(|e| format!("Rule table unreadable: {e}\nRefusing to edit it — that would overwrite your rules."))
}

/// Executes a parsed Telegram command and returns the reply text.
pub async fn handle_command(ctx: &AppContext, text: &str) -> Result<String> {
  match commands::parse(text) {
    Command::Now => build_report(ctx).await,
    Command::Help => Ok(HELP.to_string()),
    Command::ListRules => {
      let rules = match rules_for_edit(&ctx.store).await {
        Ok(r) => r,
        Err(m) => return Ok(m),
      };
      if rules.is_empty() {
        return Ok("No rules configured. /help shows the rule shape.".into());
      }
      let mut s = String::from("Rules:\n");
      for r in &rules {
        s.push_str(&format!("  {} [{}] {} {:?} {:?} lead={}m{}\n", r.id, r.group.clone().unwrap_or_else(|| "solo".into()), r.field, r.op, r.value, r.lead_min, if r.enabled { "" } else { " (off)" }));
      }
      Ok(render::truncate(s, render::TG_LIMIT))
    }
    Command::AddRule(r) => {
      let mut rules = match rules_for_edit(&ctx.store).await {
        Ok(v) => v,
        Err(m) => return Ok(m),
      };
      rules.retain(|x| x.id != r.id);
      let id = r.id.clone();
      rules.push(r);
      ctx.store.put_rules(&rules).await?;
      Ok(format!("Added rule {id}."))
    }
    Command::DelRule(id) => {
      let mut rules: Vec<Rule> = match rules_for_edit(&ctx.store).await {
        Ok(v) => v,
        Err(m) => return Ok(m),
      };
      let before = rules.len();
      rules.retain(|x| x.id != id);
      if rules.len() == before {
        return Ok(format!("No rule with id {id}."));
      }
      ctx.store.put_rules(&rules).await?;
      Ok(format!("Removed rule {id}."))
    }
    Command::SetEnabled(id, on) => {
      let mut rules = match rules_for_edit(&ctx.store).await {
        Ok(v) => v,
        Err(m) => return Ok(m),
      };
      match rules.iter_mut().find(|x| x.id == id) {
        Some(r) => {
          r.enabled = on;
          ctx.store.put_rules(&rules).await?;
          Ok(format!("Rule {id} {}.", if on { "enabled" } else { "disabled" }))
        }
        None => Ok(format!("No rule with id {id}.")),
      }
    }
    Command::ParseError(e) => Ok(format!("Could not parse that rule: {e}")),
    Command::Unknown(c) => Ok(format!("Unknown command {c}. Try /help.")),
  }
}
