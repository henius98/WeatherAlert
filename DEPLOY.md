# Deployment runbook

Target account: **ing.rd98@gmail.com** (`68dc3bdab34873a4bf2625e3c8a98401`), via the
local wrangler OAuth login.

## Current state

| Item | State |
|---|---|
| Worker script `weather-alert` | **deployed**, `https://weather-alert.ing-rd98.workers.dev` |
| KV namespace `WX` | **created**, `f56d0baa452e41e6a514fdce44805da6`, bound |
| `cfg:rules` | **seeded** from `rules.seed.json`, read back OK |
| `API_TOKEN`, `TG_WEBHOOK_SECRET` | **set** (generated locally, rotate if you like) |
| `TOMORROW_KEY`, `TG_TOKEN`, `OWNER_CHAT_ID` | **placeholders** — hold `REPLACE_ME…` |
| Cron triggers | **registered**, all three |
| HTTP routes | **live** on the workers.dev subdomain |
| workers.dev subdomain | **registered**, `ing-rd98` |
| Telegram webhook | **not registered** — needs `TG_TOKEN` first |

The Worker is deployed and the schedules fire, but every handler short-circuits
on the first line: three secrets still hold `REPLACE_ME…`. Steps 2, 3 and 5
below are what remains. `Settings::unconfigured()` returns before any upstream
call, so nothing is fetched, nothing is sent, and no quota is consumed. That is
the intended unconfigured state, not a failure.

## Step 1 — workers.dev subdomain — DONE

The subdomain `ing-rd98` is registered, which unblocked both the HTTP routes and
the cron schedules. Kept here because the failure it caused is opaque:

    /accounts/.../workers/scripts/weather-alert/schedules failed
    You need a workers.dev subdomain in order to proceed. [code: 10063]

Cron registration is gated on the subdomain, not just HTTP, and `workers_dev =
false` does not work around it. If you ever move the Worker to a fresh account,
open the Workers dashboard once before the first `deploy`.

Alternative if you own a domain on this account: add a route to `wrangler.toml`.

    [[routes]]
    pattern = "weather.example.com/*"
    custom_domain = true

## Step 2 — the three real secrets

    npx wrangler secret put TOMORROW_KEY     # app.tomorrow.io -> Development -> API key
    npx wrangler secret put TG_TOKEN         # @BotFather -> /newbot
    npx wrangler secret put OWNER_CHAT_ID    # see below

To find your chat id: message your new bot once, then

    curl -s "https://api.telegram.org/bot<TG_TOKEN>/getUpdates" \
      | python3 -c "import sys,json;print(json.load(sys.stdin)['result'][0]['message']['chat']['id'])"

`OWNER_CHAT_ID` must match exactly — it is the allowlist that stops anyone else
who finds your bot from rewriting the rule table and draining your quota.

## Step 3 — your location

Edit `wrangler.toml`. It currently ships Sepang/Selangor as a placeholder.

    LAT, LON               decimal degrees, the Tomorrow.io query point
    MET_DISTRICT_ID        data.gov.my id, e.g. Ds064
    MET_DISTRICT_NAME      the NAME, e.g. Sepang — matched against warning prose
    MET_STATE_NAME         the NAME, e.g. Selangor

Names, not ids: `St002` appears nowhere in MET warning text. Find your district:

    curl -sL 'https://api.data.gov.my/weather/forecast/?limit=2000' \
      | python3 -c "import sys,json;[print(l) for l in sorted({(r['location']['location_id'],r['location']['location_name']) for r in json.load(sys.stdin)})]"

## Step 4 — seed the rules

Already done during setup — repeat only if you change `rules.seed.json`.

    npx wrangler kv key put --remote --binding WX cfg:rules --path rules.seed.json

Verify:

    npx wrangler kv key get --remote --binding WX cfg:rules

## Step 5 — register the Telegram webhook

    curl "https://api.telegram.org/bot$TG_TOKEN/setWebhook" \
      -d "url=https://weather-alert.ing-rd98.workers.dev/webhook/telegram" \
      -d "secret_token=$TG_WEBHOOK_SECRET"

Confirm with `getWebhookInfo`; `pending_update_count` should settle at 0.

## Verification

    curl -s https://weather-alert.ing-rd98.workers.dev/health
    # -> ok   (answers before config loads, so it works even when unconfigured)

    curl -s -o /dev/null -w '%{http_code}\n' \
      https://weather-alert.ing-rd98.workers.dev/api/report
    # -> 401  (no bearer token)

    curl -s -H "Authorization: Bearer $API_TOKEN" \
      https://weather-alert.ing-rd98.workers.dev/api/report
    # -> {"report":"..."} — "Not configured yet: ..." until step 2 is done

    npx wrangler tail --format pretty
    # a rain tick every 3 min inside 09:00-23:00 MYT, a warning poll every 10 min,
    # and a morning brief at 00:00 UTC (08:00 MYT)

All three cron triggers are registered and firing. Until the secrets are real,
each tick logs `<name>_tick skipped: TOMORROW_KEY is not configured` and returns
without an upstream call — that line in `wrangler tail` is proof the schedule
works, not a fault.

In Telegram: `/help`, then `/now`, then `/rules`.

## Operating notes

**Observe mode is on.** The disaster path forwards every live MET warning tagged
`[OBSERVE]`, unfiltered — expect 2-3/day. That is deliberate: MET publishes no
severity field, so the filter must be written against harvested strings rather
than guessed. Collect a monsoon's worth, then set `MET_SEVERITY_KEYWORDS` and
flip `MET_MODE=filter`.

**Quota.** 20 Tomorrow.io calls/hour from cron against a 25/hour ceiling; manual
`/now` draws on the remaining 5, capped at 4/hour, then serves the last snapshot
marked `(cached)`. Do not widen `WINDOW_START`/`WINDOW_END` or shorten the cron
without re-reading the budget table in AGENTS.md.

**Rollback.**

    npx wrangler deployments list
    npx wrangler rollback <version-id>

**Cost.** Everything sits inside free tiers: ~424 Worker requests/day of 100k,
~330 KV writes/day of 1,000, 280 Tomorrow.io calls/day of 500. data.gov.my is
unmetered.

## Build note

`wrangler.toml`'s build command sources `$HOME/.cargo/env` first. rustup was
installed with `--no-modify-path`, so wrangler's `/bin/sh` does not otherwise see
`cargo` and the build fails with `exit code 127: cargo: command not found`.
Add `~/.cargo/bin` to your shell `PATH` if you want to run `cargo` directly.
