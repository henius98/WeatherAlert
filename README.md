# WeatherAlert

Minutely-resolution rain alerts and MET Malaysia disaster warnings, delivered to
Telegram. Rust/WASM on Cloudflare Workers, sized to run entirely inside the free
tier of every service it touches.

Single user, single location. See [PLAN.md](PLAN.md) for the full design rationale.

## Status

Deployed and building. 76 unit tests pass, `worker-build --release` produces a
~258 KiB gzipped bundle, and the Worker is live on Cloudflare with its KV
namespace bound and cron triggers registered.

Three secrets still hold placeholders, so both cron paths short-circuit before any
upstream call, and HTTP routes await a workers.dev subdomain. See
[DEPLOY.md](DEPLOY.md) for the remaining steps.

## How it works

Two independent paths on two cron triggers:

**Rain alerts** fire every 3 minutes from 09:00–23:00 MYT. Each tick makes one
Tomorrow.io Timelines call for a 30-minute window at 1-minute resolution, then
evaluates your rule table against every minute in that window. First match wins.

It does *not* poll minutely. Tomorrow.io's free tier allows 25 calls/hour, so
minutely polling would be throttled 25 minutes into the first hour. Instead one
call returns a minutely array and the rules are evaluated locally — you keep
minute-level alert resolution at 1/20th the quota.

**Disaster warnings** poll MET Malaysia every 10 minutes, 24/7, and are never
silenced by quiet hours. This endpoint is unmetered, which is why it can run at
higher frequency than the paid path.

### Alert behaviour

One alert when an event opens. A second alert only if intensity climbs a tier.
Silence on de-escalation. An all-clear once 30 minutes pass with no match.

| Tier | mm/hr |
|---|---|
| light | 0.25 – 2.5 |
| moderate | 2.5 – 7.6 |
| heavy | 7.6 – 50 |
| violent | > 50 |

A 20-minute shower matches roughly seven consecutive ticks. You get one message,
not seven.

## Observe mode

The disaster path ships in **observe mode** and forwards every live warning
tagged `[OBSERVE]`, with no severity filter.

This is deliberate. MET's payload has no severity field — severity exists only as
prose (`FIRST CATEGORY WARNING`, `KATEGORI PERTAMA`), and the vocabulary differs
per warning type. Writing a keyword filter today means guessing, and the failure
mode of a wrong guess is a real flood warning silently dropped at 3am.

So: run through a monsoon, collect the actual strings, then set `MET_MODE=filter`
and write the filter against observed data. Expect ~2–3 messages/day meanwhile.

## Setup

```sh
# toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown
cargo install worker-build

# state store — put the returned id into wrangler.toml
npx wrangler kv namespace create WX

# secrets
npx wrangler secret put TOMORROW_KEY        # tomorrow.io API key
npx wrangler secret put TG_TOKEN            # from @BotFather
npx wrangler secret put TG_WEBHOOK_SECRET   # openssl rand -hex 32
npx wrangler secret put API_TOKEN           # openssl rand -hex 32
npx wrangler secret put OWNER_CHAT_ID       # your Telegram chat id

# set LAT / LON / MET_DISTRICT_ID in wrangler.toml, then
npx wrangler deploy
```

Register the Telegram webhook, including the secret so forged calls are rejected:

```sh
curl "https://api.telegram.org/bot$TG_TOKEN/setWebhook" \
  -d "url=https://<worker>.workers.dev/webhook/telegram" \
  -d "secret_token=$TG_WEBHOOK_SECRET"
```

Find your `MET_DISTRICT_ID` (286 locations, `Ds`/`Tn`/`St`/`Rc`/`Dv` tiers):

```sh
curl -sL 'https://api.data.gov.my/weather/forecast/?limit=2000' \
  | python3 -c "import sys,json;[print(l) for l in sorted({(r['location']['location_id'],r['location']['location_name']) for r in json.load(sys.stdin)})]"
```

## Configuration

Location is deploy-time, not runtime — there is no location command. Only the
rule table is editable at runtime.

| Var | Meaning |
|---|---|
| `LAT` / `LON` / `LOC_LABEL` | the one monitored location |
| `MET_DISTRICT_ID` / `MET_STATE_ID` | data.gov.my ids for forecast + relevance |
| `WINDOW_START` / `WINDOW_END` | active hours in MYT (rain path only) |
| `MET_MODE` | `observe` or `filter` |
| `EVENT_CLOSE_MIN` | quiet minutes before an event closes (30) |
| `ALL_CLEAR` | send a "rain has passed" message |
| `MANUAL_CALLS_PER_HOUR` | cap on live manual reports (4, from 5 spare) |

### Rules

Stored in KV at `cfg:rules`. `group: null` fires on its own. Rules sharing a
`group` must **all** hold at the same forecast minute.

```json
[
  {"id":"rain_mod","group":null, "field":"rainIntensity","op":"gte","value":2.5,"lead_min":20},
  {"id":"storm_a", "group":"g1", "field":"weatherCode",  "op":"in", "value":[8000],"lead_min":20},
  {"id":"storm_b", "group":"g1", "field":"windGust",     "op":"gte","value":40,  "lead_min":20}
]
```

Fires if `rain_mod` holds, **or** if `storm_a` and `storm_b` both hold in the
same minute. Multiple groups OR against each other.

Same-minute matters: without it, gusts at minute 4 plus rain at minute 19 would
fire as one "thunderstorm" — a false positive.

## Interfaces

| | |
|---|---|
| `GET /api/report` | full current report as JSON, `Authorization: Bearer $API_TOKEN` |
| `POST /webhook/telegram` | bot commands, requires webhook secret **and** matching `chat_id` |
| `/now` | same report, formatted |
| `/rules` | show, add, toggle, delete rules |

Manual reports use a live fetch from the 5 calls/hour reserve, capped at 4/hour,
falling back to the last cron snapshot. You should never see a 429.

## Budget

Every limit below is a free tier ceiling. The binding one is 25 calls/hour.

| Constraint | Limit | Usage |
|---|---|---|
| Tomorrow.io / hour | 25 | 20 |
| Tomorrow.io / day | 500 | 280 |
| KV writes / day | 1,000 | ~315 |
| Worker CPU / tick | 10 ms | ~30 float compares |
| Cron triggers | 3–5 | 2 |

`cadence_minutes >= 3 x location_count`. A second location would break both the
hourly and daily caps at 3-minute cadence.

## Development

```sh
cargo test              # domain layer, runs natively — no wasm needed
cargo test -- --nocapture
npx wrangler dev
npx wrangler tail
```

`src/domain/` carries no `worker` dependency, which is what lets the logic be
tested on the host instead of only in WASM.

```
src/domain/   timeutil tiers rules state met forecast tomorrow
              render commands quota errors util        (pure, tested)
src/infra/    settings store tomorrow metapi telegram tasks   (wasm-only)
```

`[profile.release]` deliberately omits `strip` and `panic = "abort"` — both break
wasm-bindgen's catch wrappers. See AGENTS.md before changing it.

## Data sources

- [Tomorrow.io Timelines](https://docs.tomorrow.io/reference/post-timelines) — nowcast trigger. 500/day, 25/hr, 3/sec free.
- [data.gov.my Weather](https://developer.data.gov.my/realtime-api/weather) — warnings (authoritative) and 7-day district forecast (daily, context only). Unauthenticated.

The daily forecast cannot corroborate a minutely nowcast; it appears in reports
as context and never triggers an alert.
