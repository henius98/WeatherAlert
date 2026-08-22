# WeatherAlert — locked plan

Single-user weather alerting on Cloudflare Workers free tier, Rust/WASM.
Two independent alert paths, one location, Telegram delivery.

## Budget (binding constraint: Tomorrow.io 25 calls/hour)

| Constraint | Free limit | Usage | Headroom |
|---|---|---|---|
| Tomorrow.io calls/hour | 25 | 20 | 5 (reserved for manual reports) |
| Tomorrow.io calls/day | 500 | 280 | 220 |
| Worker requests/day | 100,000 | ~424 | — |
| Worker CPU per tick | 10 ms | ~30 float comparisons | large |
| KV writes/day | 1,000 | ~315 | 685 |
| Cron triggers | 3–5 | 2 | — |
| Bundle (gzipped) | 3 MB | 257 KiB measured | 92% |

`cadence_minutes >= 3 x location_count`. One location holds 3-minute cadence
with a 17-minute worst-case lead time on a 20-minute rule.

## Paths

| | Rain alert | Disaster alert |
|---|---|---|
| Cron (UTC) | `*/3 1-14 * * *` | `*/10 * * * *` |
| Local window | 09:00–22:57 MYT | 24/7, never silenced |
| Source | Tomorrow.io Timelines | data.gov.my `/weather/warning/` |
| Trigger | `cfg:rules` (DNF) | observe mode: everything live + unseen |
| Quota cost | 280/day of 500 | unmetered |

## Rain path, per tick

1. Read `cfg:rules` + `st:event` from KV.
2. One Tomorrow.io call: `timesteps=1m`, `startTime=now`, `endTime=now+30m`,
   `fields` = union of rule fields. ~30 points, a few KB.
3. `rules::first_match` walks minutes ascending, returns the earliest satisfied
   disjunct. `group: None` fires alone; a shared `group` requires all members
   true **at the same forecast minute**.
4. Tier from `rainIntensity` at that minute.
5. `state::step` decides: Open / Escalate / Nothing / Close.
6. Write KV only when `write_needed` (`st:event`) or `dirty` (`st:err`) —
   transitions only. `st:snapshot` is the one deliberate per-tick write, for the
   `/now` fallback; ~315 writes/day total.

### Tier ladder (mm/hr)
`none <0.25 | light 0.25–2.5 | moderate 2.5–7.6 | heavy 7.6–50 | violent >50`

Alert on open; re-alert only when the tier rises; never on de-escalation.
Close after 30 min with no match, then send an all-clear.

## Disaster path — OBSERVE MODE (phase 1)

Severity is not a structured field in the MET payload. It appears only as prose
("FIRST CATEGORY WARNING", "KATEGORI PERTAMA"), and the vocabulary differs per
warning type. The live sample contained no heavy-rain or flood warning, so the
severity keyword list cannot yet be written honestly.

Phase 1 therefore forwards **every** live, non-sentinel, unseen warning with no
severity or relevance filter, tagged `[OBSERVE]`. Run through the monsoon,
harvest the real vocabulary, then write the filter against observed strings.

Expected volume: 5 active records at time of writing, ~2–3 new issuances/day.

Phase 2 (`MET_MODE=filter`) adds: severity keyword match, relevance match on
`MET_STATE_ID`/`MET_DISTRICT_ID` names, and drops the shipping block.

### Payload traps, all observed live and covered by tests
1. `valid_from`/`valid_to` are **nullable** — the "No Advisory" sentinel has both
   null. Null `valid_to` means open-ended, not expired.
2. Timestamps are naive and are **MYT**; no offset suffix. Parsing as UTC puts
   every warning 8 hours out.
3. Three records shared `issued` **and** `title_en`, differing only by validity
   window. Dedup key must be `issued|title|valid_from|valid_to`.
4. `text_en` reaches ~1.5 KB and mixes a land section with
   `SECTION B: FOR SHIPPING` (Phuket, Layang-Layang). Telegram caps at 4096.
5. Endpoints **301 to a trailing slash** — follow redirects or hardcode it.
6. Warnings stay valid for up to a week, so a 10-min poll without dedup would
   re-alert ~864 times.

## Manual report — `GET /api/report` and Telegram `/now`

Same builder, two transports. Live Tomorrow.io fetch capped at 4/hour via
`st:quota`, falling back to the last cron snapshot when exhausted, so a 429 is
never surfaced. Includes the data.gov.my district forecast, filtered
server-side: `?contains=<MET_DISTRICT_ID>@location__location_id&date_start=..&date_end=..`
returns 2 records instead of 2,000 — the difference between fitting the 10 ms
CPU budget and not.

`summary_forecast` is a closed Malay enum (`Tiada hujan`, `Hujan`, `Ribut petir`,
`Jerebu`, `Berangin`, ...) mapped to English for display.

## Error policy

Send: 401/403, 429 sustained past 2 ticks, KV failure, config parse failure,
>=5 consecutive tick failures. Silent: single transient 5xx/timeout, one
malformed record, empty forecast array. One message per class per hour via
`st:err`, plus one "recovered". Without the throttle a 6-hour outage sends 120
identical messages.

## Security

- `POST /webhook/telegram` — constant-time compare of
  `X-Telegram-Bot-Api-Secret-Token`, then assert `chat_id == OWNER_CHAT_ID`,
  else 200 and drop. The chat_id allowlist matters more than the token: without
  it anyone who finds the bot can rewrite the rule table and drain the quota.
- `GET /api/report` — `Authorization: Bearer <32 bytes>`, constant-time.

## Config surface

vars: `LAT LON LOC_LABEL MET_DISTRICT_ID MET_STATE_ID WINDOW_START WINDOW_END
TZ_OFFSET MET_MODE LEAD_MIN_DEFAULT EVENT_CLOSE_MIN ALL_CLEAR
MANUAL_CALLS_PER_HOUR`

secrets: `TOMORROW_KEY TG_TOKEN TG_WEBHOOK_SECRET API_TOKEN OWNER_CHAT_ID`

KV `WX`: `cfg:rules st:event st:met st:err st:quota`

Location is deploy-time, not runtime: no Share Location handler, no district
keyboard, no centroid table (data.gov.my exposes no coordinates for its 286
locations). Moving house needs a redeploy.

## Layout

    src/domain/   pure, no worker dependency -> `cargo test` runs on the host
      timeutil.rs epoch<->civil, MYT offset, active window
      tiers.rs    intensity ladder
      rules.rs    DNF evaluator
      state.rs    event state machine
      met.rs      warning parse, dedup, observe selection, formatting
    src/infra/    wasm-only: tomorrow, metapi, telegram, kv

## Status

Complete and deployed. 76 unit tests pass; `worker-build --release` yields 257 KiB gzipped
(3 MB ceiling); `wrangler deploy --dry-run` validates.

Build-flag finding: `strip = true` and `panic = "abort"` both break the build —
wasm-bindgen needs the externref table for its catch wrappers. `opt-level = "z"`,
`lto` and `codegen-units = 1` are all fine and are what we ship.

KV namespace created and seeded; Worker deployed; crons registered. Remaining:
register a workers.dev subdomain, set the three third-party secrets, and set the
real location. See DEPLOY.md.

Post-audit fixes: secret redaction at the notify chokepoint (the Tomorrow.io key
rides in a query string and upstream errors quote the URL); `have_data` so an
empty snapshot cannot render as "no rain expected"; `/health` answered before
config loads; KV write failures logged rather than dropped; `observability`
enabled; dead `TZ_OFFSET` and `LEAD_MIN_DEFAULT` vars removed.
