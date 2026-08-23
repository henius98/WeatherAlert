# AGENTS.md

Guidance for AI agents working in this repo. Read this before changing anything
under `src/`. [PLAN.md](PLAN.md) has the reasoning; this file has the rules.

The whole project is shaped by free-tier ceilings that are not obvious from the
code. Several natural-looking "improvements" break it. Those are listed first.

## Keep FLOW.md current — required

[FLOW.md](FLOW.md) is the normative control-flow reference: five mermaid diagrams
covering every entry point, guard, and early return. **Any change to control flow
must update it in the same commit.** A stale flow diagram is worse than none — it
is read as authoritative and quietly teaches the next reader the wrong model.

Which diagram to touch:

| You changed | Update |
|---|---|
| `tasks::rain_tick` | §1 Rain path |
| `domain::state::step`, tier ladder | §2 Event decision tree |
| `tasks::warning_tick`, `domain::met` selection | §3 Disaster path |
| `tasks::build_report`, quota, snapshot fallback | §4 Report |
| `lib.rs` routing, `commands`, `handle_command` | §5 Bot commands |

It counts as a control-flow change if you add or remove any of: a guard or early
return, a KV read or write, an upstream call, a branch in an `alt`/`opt`, an entry
point or route, or a reordering of send-versus-persist. Renaming a helper or
editing a threshold does not.

Removing a branch means deleting it from the diagram — do not only ever add.

Validate before committing; do not eyeball mermaid:

    # extract and render every block — all five must succeed
    npx -y @mermaid-js/mermaid-cli -i block.mmd -o block.svg

Then **look at the rendered image**, not just the exit code. Two failure modes
have already bitten this file: a `Note over X:` spanning one participant clips its
text, and `stateDiagram-v2` silently merges repeated transitions between the same
two states — the original event diagram lost its entire escalation path that way,
which is why §2 is a `flowchart` decision tree rather than a state diagram.

## Do not do these

**Do not increase Tomorrow.io call frequency.** Free tier is 25 calls/hour, 500/day.
The rain cron runs every 3 minutes over a 14-hour window = 20/hour, 280/day. The
5 remaining hourly calls are reserved for manual reports. Tightening the cadence,
widening the active window, or adding a second location breaks a hard cap and the
alerts silently stop working for the rest of the hour.

The governing rule is `cadence_minutes >= 3 x location_count`. Multi-location
support therefore requires *lengthening* the cadence, not parallel fetches.

**Do not poll minutely.** Tempting, given "minutely alerts". One call with
`timesteps=1m` returns a whole minutely array; the rules are evaluated locally
against it. That is where the resolution comes from. Minutely polling would be
throttled 25 minutes into the first hour.

**Do not request a wider Tomorrow.io window.** Ask for `now` to `now+30m` only.
The 1m timestep permits a 24-hour range — that is ~1440 points, and deserializing
them every tick threatens the 10ms CPU limit. 30 points is the design.

**Do not add an unconditional KV write to a cron path.** Free tier is 1,000
writes/day and the rain cron ticks 280 times, so one careless per-tick write costs
28% of the budget. Two gates exist and both must be respected:

- `state::step` returns `write_needed` — persist `st:event` only when true. A
  60-minute shower must cost one write, not twenty.
- `ErrState.dirty` (not serialized) — persist `st:err` only when something
  actually changed. A healthy tick must not write at all.

`alerts_once_then_stays_silent_across_ticks` and `a_clean_tick_costs_no_kv_write`
assert these; if either fails, the fix is the caller, not the test.

`st:snapshot` is the one deliberate per-tick write (~280/day), because the `/now`
fallback needs recent data. `st:daily` is written only on Open/Escalate plus once
each morning. Actual total is ~330/day of 1,000. If you add another per-tick
write, re-do this arithmetic first.

**Do not add `chrono`, `time`, or a TLS/HTTP crate as a direct dependency.** The
free-tier bundle ceiling is 3 MB gzipped. `src/domain/timeutil.rs` implements the
civil-date maths needed; use `worker::Fetch` for HTTP. (`chrono` does arrive
transitively via `worker` 0.8 — that is not a licence to depend on it directly.)

**Do not add `strip = true` or `panic = "abort"` to `[profile.release]`.** Both
break the build, and both look like obvious size wins, so this will tempt you.
wasm-bindgen needs the externref table for the catch wrappers that worker-build
requests via `--force-enable-abort-handler`; stripping removes it and the build
dies with `externref table required for catch wrappers`. This was verified by
bisection: `opt-level="z"` + `lto` + `codegen-units=1` all build fine, and adding
`strip = true` alone is what fails. Current size is 257 KiB gzipped against a
3 MB ceiling, so there is nothing to win here anyway.

**Do not make `src/domain/` depend on the `worker` crate.** `worker` is declared
only under `[target.'cfg(target_arch = "wasm32")'.dependencies]` so `cargo test`
runs the logic natively on the host. Domain modules take plain data; all I/O
lives in `src/infra/`. Breaking this makes the test suite unrunnable without a
WASM harness.

**Do not add a severity filter to the disaster path.** `MET_MODE=observe` is
intentional, not unfinished work. MET's payload has no severity field — severity
appears only inside prose, with different vocabularies per warning type
(`FIRST CATEGORY` for wind/sea, colour words for rain). No heavy-rain or flood
example has been observed yet, so any keyword list today is a guess whose failure
mode is a real flood warning dropped at 3am. The filter gets written against
harvested strings, after a monsoon of observation. Leave it alone.

**Do not interpolate a raw error into user-visible text.** The Tomorrow.io key
travels as a query parameter, and an upstream error can quote the request URL, so
an unredacted message is a credential leak. `tasks::notify` is the single
chokepoint and redacts every known secret; route new user-facing paths through it
rather than calling `telegram::send` directly. `domain::util::redact` covers the
one place that cannot (the command-error reply in `lib.rs`).

**Do not report only the onset minute.** `rules::evaluate` returns onset *and*
peak tier, and both appear in the alert. Reporting the earliest matching minute
alone describes the drizzle in front of a downpour — "light rain in 2 min" while
60 mm/hr arrives at minute 15. The escalation ratchet compares peaks for the same
reason. `reports_onset_and_peak_not_just_the_drizzle_in_front` guards this.

**Do not remove the morning brief.** `daily_tick` is the only liveness signal.
Without it a dead Worker and a quiet day both produce silence, which is the one
failure mode an alerting system cannot have. It is sent even when both MET calls
fail — weather is secondary to proving the monitor ran.

**Do not accept an unvalidated rule field.** `is_valid_field` gates `/rules add`.
Field names go straight into the `fields=` query parameter, so one typo makes
Tomorrow.io 400 *every* subsequent fetch. `classify` treats 400 as transient, so
alerting would die silently for ~15 minutes before reporting a cause that never
names the typo.

**Do not let "no data" render as "no rain".** `render::report` takes `have_data`
precisely so an empty timeline — KV miss, expired snapshot, quota exhausted —
reports NO DATA. Collapsing the two produces a false all-clear during the exact
conditions where someone is checking. `missing_data_never_renders_as_no_rain`
guards this.

**Do not make `/health` depend on `Settings::load`.** It is answered before config
loads, on purpose: a liveness probe that fails on an incomplete config cannot tell
you whether the Worker is running.

**Do not let quiet hours suppress the disaster path.** The rain path is windowed
to 09:00–23:00 MYT. The warning path runs 24/7 by design and must stay that way.

**Do not de-escalate alerts.** Tier only ratchets upward within an open event.
Falling intensity is silent; the peak tier is retained until the event closes.

**Do not let a config read degrade to a default.** `store.rules()` returning an
error means `cfg:rules` is corrupt; treating that as an empty rule table leaves the
Worker ticking, spending quota, and never alerting — a silent total failure. The
rain path notifies and returns instead. The same reasoning applies to any future
config key: `unwrap_or_default()` on a parse error hides exactly the failure you
most need to see. (`st:event`, `st:err` and `st:met` may default — losing state
degrades gracefully; losing rules does not.)

## Config traps

**Three cron triggers are in use, at or near the free-plan ceiling.** Adding a
fourth may be rejected. Fold new periodic work into an existing schedule instead.

**`WINDOW_START`/`WINDOW_END` can only narrow, never widen.** The cron expression
`*/3 1-14 * * *` is the outer bound; the vars are a second gate inside it. Setting
`WINDOW_END = "24"` does nothing, because the Worker is never invoked after 14:59
UTC. To genuinely widen the window you must change the cron **and** re-check the
quota table — more hours means more calls against the 25/hour ceiling.

**Warning delivery is at-least-once, deliberately.** `warning_tick` sends before
persisting `st:met`. If the KV write fails you get a duplicate on the next poll;
the reverse order would drop the alert entirely. Do not "fix" the ordering.

## Payload traps

All six were observed on the live `api.data.gov.my` feed and each has a
regression test in `src/domain/met.rs`. Do not "simplify" past them.

1. **Timestamps are naive and are MYT.** `"issued":"2026-08-22T20:25:00"` carries
   no offset. Parsing as UTC puts every warning 8 hours out. Always go through
   `timeutil::parse_naive_myt`, which subtracts the +08:00.

2. **`valid_from` / `valid_to` are nullable.** The `No Advisory` sentinel record
   has both null. They are `Option<String>`. Null `valid_to` means open-ended,
   **not** expired.

3. **Dedup must include the validity window.** Three live records shared `issued`
   *and* `title_en`, differing only by `valid_from`/`valid_to`. Keying on
   issued+title collapses three real warnings into one. Key is
   `issued|title|valid_from|valid_to`.

4. **Warnings stay valid for days** (one ran 08-21 → 08-27). Without dedup, a
   10-minute poll re-alerts ~864 times over six days.

5. **Endpoints 301 to a trailing slash.** `/weather/warning` redirects to
   `/weather/warning/`. Follow redirects or hardcode the slash.

6. **`text_en` mixes land and marine content.** ~1.5 KB, split by
   `SECTION B: FOR SHIPPING`, and the marine half covers Phuket, Palawan,
   Layang-Layang — irrelevant to a land user. Use `Warning::land_text()`.
   Telegram caps a message at 4096 chars; `format_observe` respects a budget.

Also: filter data.gov.my server-side. `?contains=<id>@location__location_id`
plus `date_start`/`date_end` returns 2 records instead of 2,000. Both params are
confirmed working. Pulling the full feed and filtering in Rust wastes the CPU
budget.

## Rule semantics

`src/domain/rules.rs` evaluates disjunctive normal form.

- `group: None` — fires independently (OR).
- `group: Some("g1")` — every enabled member must hold **at the same forecast
  minute** (AND). Multiple groups OR against each other.
- A rule whose `op` and `value` shapes disagree (e.g. `gte` against a set) is a
  config error and never matches. Never fire on an uninterpretable rule.
- A field absent from the timeline never matches.
- Minutes are walked ascending; the first satisfied disjunct wins, so alerts
  always carry the earliest ETA.

Same-minute AND is load-bearing. Relaxing it to anywhere-in-window makes "gusts
at minute 4, rain at minute 19" fire as a thunderstorm — a false positive.

## Security invariants

- `POST /webhook/telegram` — constant-time compare of
  `X-Telegram-Bot-Api-Secret-Token`, **then** assert `chat_id == OWNER_CHAT_ID`.
  Reject with 200 and no body so the endpoint stays quiet to probes. The chat_id
  check matters more than the token: without it anyone who finds the bot can
  rewrite the rule table and drain the daily quota.
- `GET /api/report` — `Authorization: Bearer`, constant-time compare.
- Never log or echo secrets. `TOMORROW_KEY` goes in a header, never a query
  string that might reach an error message sent to Telegram.
- Never add an unauthenticated route that triggers an upstream fetch. That is a
  free faucet on a 500-call/day quota.

## Errors

Send to Telegram: 401/403, 429 sustained past 2 ticks, KV failure, config parse
failure, ≥5 consecutive tick failures. Stay silent: single transient 5xx or
timeout, one malformed record in a batch, an empty forecast array.

Every send goes through the `st:err` throttle — one message per class per hour,
plus one "recovered". Without it a 6-hour upstream outage sends 120 identical
messages. Do not bypass the throttle for a "just this once" error path.

## Testing

Every test lives in `tests/suite/`, one file per domain module, compiled into a
single binary rooted at `tests/suite/main.rs`. `src/` contains no test code.

    cargo test                       # whole suite
    cargo test --test suite met      # one module
    cargo test -- --nocapture

Two things about this layout will trip you up:

**Adding a file is not enough — register it.** `Cargo.toml` sets
`autotests = false` and declares one explicit `[[test]]` target, because a test
crate root resolves `mod` against its own directory (`tests/suite.rs` would look
for `tests/<name>.rs`). A new `tests/suite/foo.rs` is dead code until you add
`mod foo;` to `tests/suite/main.rs`. It will not fail — it simply will not run.

**These are integration tests: they see only the public API.** A test cannot reach
a private function or const. That is a deliberate forcing function — it keeps the
domain layer's surface honest — but it means a genuinely internal helper needs
either a public wrapper worth having, or coverage through the public function that
calls it. Do not make something `pub` solely to test it; test it through its caller.

The public-API-only constraint is also what keeps `src/domain/` free of the
`worker` crate, which is why the suite runs natively with no wasm target.

Add a regression test for every upstream payload surprise, with the observed
values in the test — the MET tests encode real records for exactly this reason.
Prefer asserting on quota-relevant behaviour (`write_needed`, `dirty`, call
counts) over message text; the quota is what breaks in production.

Do not weaken a failing budget test to make a change pass. Those tests encode
free-tier ceilings, not preferences.

## Verification status

Verified on Rust 1.98, `worker` 0.8.5, wrangler 4.125:

    cargo test                                   89 passed (tests/suite/)
    worker-build --release                       ok, 257 KiB gzipped
    npx wrangler deploy --dry-run --outdir ...   ok

An unconfigured deploy is a supported state: `Settings::unconfigured()`
short-circuits both cron paths before any upstream call while the third-party
secrets still hold the `REPLACE_ME` sentinel. Do not "fix" this by removing the
guard — it is what stops a half-configured Worker from making authenticated calls
with a credential already known to be wrong.

`src/infra/` is wasm-only, so `cargo test` does not cover it — it is checked by
`cargo check --target wasm32-unknown-unknown`. Anything worth asserting on should
therefore live in `src/domain/`, which is why parsing, rendering, quota, command
grammar and date maths are all there rather than in the client modules.
