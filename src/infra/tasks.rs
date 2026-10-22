use crate::domain::commands::{self, Command, HELP};
use crate::domain::daily::{morning_summary, DailyLog};
use crate::domain::errors::{classify, Class};
use crate::domain::forecast;
use crate::domain::met::{self, format_observe, Warning};
use crate::domain::render;
use crate::domain::rules::{evaluate, Rule};
use crate::domain::state::step;
use crate::domain::state::Action;
use crate::domain::tiers::Tier;
use crate::domain::timeutil::{in_active_window, ymd_myt};
use crate::domain::tomorrow::{parse_timeline, required_fields, window_minutes};
use crate::infra::{metapi, settings::Settings, store::Store, telegram, tomorrow};
use worker::{console_error, console_log, Date, Env, Result};

pub fn now_secs() -> i64 {
  (Date::now().as_millis() / 1000) as i64
}

/// The single chokepoint for user-visible text. Redaction lives here rather
/// than at each call site so a future error path cannot forget it.
async fn notify(s: &Settings, text: &str) {
  let safe = crate::domain::util::redact(text, &[&s.tomorrow_key, &s.tg_token, &s.api_token, &s.tg_webhook_secret]);
  if let Err(e) = telegram::send(&s.tg_token, &s.owner_chat_id, &safe).await {
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
pub async fn rain_tick(env: &Env) -> Result<()> {
  let s = Settings::load(env)?;
  if let Some(missing) = s.unconfigured() {
    console_error!("rain_tick skipped: {missing} is not configured");
    return Ok(());
  }
  let now = now_secs();
  if !in_active_window(now, s.window_start, s.window_end) {
    return Ok(());
  }

  let store = Store::new(env)?;
  let mut errs = store.errs().await.unwrap_or_default();

  // A corrupt cfg:rules must never degrade to an empty table. Doing so would
  // leave the Worker ticking, spending quota, and never alerting — a silent
  // total failure with nothing in the logs.
  let rules = match store.rules().await {
    Ok(r) => r,
    Err(e) => {
      if errs.should_send(Class::Config, now) {
        notify(&s, &format!("\u{26A0}\u{FE0F} Rule table unreadable: {e}\nNo rain alerts will fire until this is fixed.")).await;
      }
      if errs.dirty {
        log_kv("st:err", store.put_errs(&errs).await);
      }
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
  let fetched = tomorrow::timeline(&s.lat, &s.lon, &fields, &s.tomorrow_key, window).await;

  let body = match fetched {
    Ok(f) if f.status == 200 => {
      if errs.note_success() {
        notify(&s, "\u{2705} Weather feed recovered.").await;
      }
      f.body
    }
    Ok(f) => {
      errs.note_failure();
      let c = classify(Some(f.status), errs.consecutive - 1);
      if errs.should_send(c, now) {
        notify(&s, &format!("\u{26A0}\u{FE0F} Tomorrow.io HTTP {} ({})", f.status, c.key())).await;
      }
      if errs.dirty {
        log_kv("st:err", store.put_errs(&errs).await);
      }
      return Ok(());
    }
    Err(e) => {
      errs.note_failure();
      let c = classify(None, errs.consecutive - 1);
      if errs.should_send(c, now) {
        notify(&s, &format!("\u{26A0}\u{FE0F} Weather fetch failed: {e}")).await;
      }
      if errs.dirty {
        log_kv("st:err", store.put_errs(&errs).await);
      }
      return Ok(());
    }
  };

  let timeline = match parse_timeline(&body, now) {
    Ok(t) => t,
    Err(e) => {
      errs.note_failure();
      if errs.should_send(Class::Config, now) {
        notify(&s, &format!("\u{26A0}\u{FE0F} Timeline parse failed: {e}")).await;
      }
      if errs.dirty {
        log_kv("st:err", store.put_errs(&errs).await);
      }
      return Ok(());
    }
  };

  // Cache for /now fallback. Cheap: expires in an hour, one write per tick,
  // still far inside the 1000/day KV budget alongside ~20 state writes.
  log_kv("st:snapshot", store.put_snapshot(&body).await);

  // evaluate() reports onset AND peak, so the first alert states how bad it
  // gets rather than describing only the drizzle in front of the storm.
  let hit = evaluate(&rules, &timeline);

  let mut ev = store.event().await.unwrap_or_default();
  let out = step(&mut ev, hit.as_ref(), now, s.close_min, s.all_clear);

  if let Some(msg) = render::alert(&out.action, &s.label) {
    notify(&s, &msg).await;
  }
  // Tally for the morning summary, which is how thresholds get tuned.
  match &out.action {
    Action::Open { peak_tier, .. } | Action::Escalate { to: peak_tier, .. } => {
      let escalation = matches!(out.action, Action::Escalate { .. });
      let mut log = store.daily().await.unwrap_or_default();
      log.record(&ymd_myt(now), *peak_tier, escalation);
      log_kv("st:daily", store.put_daily(&log).await);
    }
    _ => {}
  }
  if out.write_needed {
    if let Err(e) = store.put_event(&ev).await {
      console_error!("kv write failed (st:event): {e}");
      if errs.should_send(Class::Storage, now) {
        notify(&s, &format!("\u{26A0}\u{FE0F} State storage write failed: {e}")).await;
      }
      if errs.dirty {
        log_kv("st:err", store.put_errs(&errs).await);
      }
      return Err(e);
    }
  }
  if errs.dirty {
    log_kv("st:err", store.put_errs(&errs).await);
  }
  Ok(())
}

/// Disaster path. Runs 24/7 and is never silenced by the active window.
pub async fn warning_tick(env: &Env) -> Result<()> {
  let s = Settings::load(env)?;
  if let Some(missing) = s.unconfigured() {
    console_error!("warning_tick skipped: {missing} is not configured");
    return Ok(());
  }
  let now = now_secs();
  let store = Store::new(env)?;

  let (body, status) = match metapi::warnings().await {
    Ok(v) => v,
    Err(e) => {
      console_log!("met warning fetch failed: {e}");
      return Ok(());
    }
  };
  if status != 200 {
    console_log!("met warning HTTP {status}");
    return Ok(());
  }

  let warnings: Vec<Warning> = match serde_json::from_str(&body) {
    Ok(v) => v,
    Err(e) => {
      console_log!("met warning parse failed: {e}");
      return Ok(());
    }
  };

  let st = store.met().await.unwrap_or_default();
  let (selected, next) = met::select_observe(&warnings, &st, now);
  if selected.is_empty() {
    return Ok(());
  }

  let sev: Vec<&str> = if !s.met_observe {
    s.severity_keywords.split(',').map(str::trim).filter(|x| !x.is_empty()).collect()
  } else {
    Vec::new()
  };
  let places: Vec<&str> = if !s.met_observe {
    [s.state_name.as_str(), s.district_name.as_str()].into_iter().filter(|x| !x.is_empty()).collect()
  } else {
    Vec::new()
  };

  for i in selected {
    let w = &warnings[i];
    if s.met_observe {
      // Observe mode sends the compact form: the severity vocabulary lives
      // in the title and heading, not in 1.5 KB of prose.
      notify(&s, &met::format_summary(w)).await;
    } else if met::passes_filter(w, &sev, &places) {
      // Filter mode gates on severity keywords harvested from real payloads
      // plus place NAMES (ids like "St002" never appear in MET prose).
      notify(&s, &format_observe(w, render::TG_LIMIT)).await;
    }
  }
  store.put_met(&next).await?;
  Ok(())
}

/// 08:00 MYT summary. Doubles as the liveness signal: if this stops arriving,
/// the monitor is down. Costs one Telegram send and two unmetered MET calls.
pub async fn daily_tick(env: &Env) -> Result<()> {
  let s = Settings::load(env)?;
  if let Some(missing) = s.unconfigured() {
    console_error!("daily_tick skipped: {missing} is not configured");
    return Ok(());
  }
  let now = now_secs();
  let today = ymd_myt(now);
  let store = Store::new(env)?;

  // The forecast is context, not a trigger, so a failure degrades to an empty
  // brief rather than aborting — but it is logged. An unlogged fallback hid a
  // malformed query behind a brief that merely looked sparse.
  let days = match metapi::forecast(&s.district, &today, &today).await {
    Ok((b, 200)) => forecast::parse(&b, &s.district, 1).unwrap_or_default(),
    Ok((b, st)) => {
      console_error!("daily forecast HTTP {st}: {}", b.chars().take(200).collect::<String>());
      Vec::new()
    }
    Err(e) => {
      console_error!("daily forecast fetch failed: {e}");
      Vec::new()
    }
  };
  let warn_lines: Vec<String> = match metapi::warnings().await {
    Ok((b, 200)) => met::active_titles(&b, now),
    _ => Vec::new(),
  };

  let prev = store.daily().await.unwrap_or_default();
  notify(&s, &morning_summary(&s.label, &today, &days, &warn_lines, &prev, (s.window_start, s.window_end))).await;

  // Start today's tally fresh so the count always covers one civil day.
  log_kv("st:daily", store.put_daily(&DailyLog { date: today, ..Default::default() }).await);
  Ok(())
}

/// Shared body for `/now` and `GET /api/report`.
pub async fn build_report(env: &Env) -> Result<String> {
  let s = Settings::load(env)?;
  if let Some(missing) = s.unconfigured() {
    return Ok(format!("Not configured yet: {missing} is unset. See DEPLOY.md."));
  }
  let now = now_secs();
  let store = Store::new(env)?;
  // Same invariant as the rain path: a corrupt rule table is reported, not
  // silently defaulted. Here it only selects query fields, but a report that
  // looks normal while the rules are unreadable is still a lie.
  let rules = match store.rules().await {
    Ok(r) => r,
    Err(e) => return Ok(format!("Rule table unreadable: {e}\nNo rain alerts are firing.")),
  };

  let mut quota = store.quota().await.unwrap_or_default();
  let allow_live = quota.try_spend(now, s.manual_per_hour);
  let mut stale = false;

  let body = if allow_live {
    log_kv("st:quota", store.put_quota(&quota).await);
    let w = window_minutes(&rules);
    match tomorrow::timeline(&s.lat, &s.lon, &required_fields(&rules), &s.tomorrow_key, w).await {
      Ok(f) if f.status == 200 => f.body,
      _ => {
        stale = true;
        store.snapshot().await.unwrap_or_default().unwrap_or_default()
      }
    }
  } else {
    stale = true;
    store.snapshot().await.unwrap_or_default().unwrap_or_default()
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

  let days = match metapi::forecast(&s.district, &ymd_myt(now), &ymd_myt(now + 86_400)).await {
    Ok((b, 200)) => forecast::parse(&b, &s.district, 2).unwrap_or_default(),
    Ok((b, st)) => {
      console_error!("report forecast HTTP {st}: {}", b.chars().take(200).collect::<String>());
      Vec::new()
    }
    Err(e) => {
      console_error!("report forecast fetch failed: {e}");
      Vec::new()
    }
  };

  let warn_lines = match metapi::warnings().await {
    Ok((b, 200)) => met::active_titles(&b, now),
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
pub async fn handle_command(env: &Env, text: &str) -> Result<String> {
  let store = Store::new(env)?;
  match commands::parse(text) {
    Command::Now => build_report(env).await,
    Command::Help => Ok(HELP.to_string()),
    Command::ListRules => {
      let rules = match rules_for_edit(&store).await {
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
      let mut rules = match rules_for_edit(&store).await {
        Ok(v) => v,
        Err(m) => return Ok(m),
      };
      rules.retain(|x| x.id != r.id);
      let id = r.id.clone();
      rules.push(r);
      store.put_rules(&rules).await?;
      Ok(format!("Added rule {id}."))
    }
    Command::DelRule(id) => {
      let mut rules: Vec<Rule> = match rules_for_edit(&store).await {
        Ok(v) => v,
        Err(m) => return Ok(m),
      };
      let before = rules.len();
      rules.retain(|x| x.id != id);
      if rules.len() == before {
        return Ok(format!("No rule with id {id}."));
      }
      store.put_rules(&rules).await?;
      Ok(format!("Removed rule {id}."))
    }
    Command::SetEnabled(id, on) => {
      let mut rules = match rules_for_edit(&store).await {
        Ok(v) => v,
        Err(m) => return Ok(m),
      };
      match rules.iter_mut().find(|x| x.id == id) {
        Some(r) => {
          r.enabled = on;
          store.put_rules(&rules).await?;
          Ok(format!("Rule {id} {}.", if on { "enabled" } else { "disabled" }))
        }
        None => Ok(format!("No rule with id {id}.")),
      }
    }
    Command::ParseError(e) => Ok(format!("Could not parse that rule: {e}")),
    Command::Unknown(c) => Ok(format!("Unknown command {c}. Try /help.")),
  }
}
