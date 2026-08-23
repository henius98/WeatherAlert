# Control flow

Four entry points. Every guard and early return below is real — the branches are
where this program's behaviour actually lives.

**This file is normative.** Any change to control flow must update it in the same
commit. See the "Keeping this file current" rule in [AGENTS.md](AGENTS.md).

| Entry point | Trigger | Handler |
|---|---|---|
| Rain path | cron `*/3 1-14 * * *` | `tasks::rain_tick` |
| Disaster path | cron `*/10 * * * *` | `tasks::warning_tick` |
| Report | `GET /api/report` | `tasks::build_report` |
| Bot commands | `POST /webhook/telegram` | `tasks::handle_command` |
| Morning brief | cron `0 0 * * *` (08:00 MYT) | `tasks::daily_tick` |

---

## 1. Rain path — `tasks::rain_tick`

```mermaid
sequenceDiagram
    autonumber
    participant C as Cron */3
    participant W as lib.rs scheduled
    participant T as tasks::rain_tick
    participant KV as KV (WX)
    participant API as Tomorrow.io
    participant D as domain
    participant TG as Telegram

    C->>W: ScheduledEvent(cron)
    W->>T: cron starts with "*/3 "
    T->>T: Settings::load

    alt secret still REPLACE_ME
        T-->>W: log + return (no upstream call)
    end
    alt outside 09:00-23:00 MYT
        T-->>W: return
    end

    T->>KV: get st:err
    T->>KV: get cfg:rules

    alt cfg:rules unreadable
        Note over T,API: never degrade to an empty table —<br/>that is a silent total failure
        T->>TG: "Rule table unreadable"
        T-->>W: return
    end
    alt rules empty
        T-->>W: log + return (saves a call)
    end

    T->>D: required_fields + window_minutes(rules)
    T->>API: GET /v4/timelines (now .. now+N min, 1m)

    alt HTTP != 200 or transport error
        T->>D: note_failure + classify
        opt not throttled
            T->>TG: error message (secrets redacted)
        end
        opt errs.dirty
            T->>KV: put st:err
        end
        T-->>W: return
    end

    T->>D: note_success
    opt was failing
        T->>TG: "feed recovered"
    end
    T->>D: parse_timeline
    alt parse error
        T->>D: note_failure
        T->>TG: "Timeline parse failed"
        T-->>W: return
    end

    T->>KV: put st:snapshot (only deliberate per-tick write)
    T->>D: evaluate(rules, timeline)
    Note over T,API: returns onset AND peak tier —<br/>onset alone understates a storm behind drizzle
    T->>KV: get st:event
    T->>D: state::step -> Action

    opt Action != Nothing
        T->>TG: alert / escalation / all-clear
    end
    opt write_needed
        T->>KV: put st:event
    end
    opt Open or Escalate
        T->>KV: put st:daily (tally for the morning brief)
    end
    opt errs.dirty
        T->>KV: put st:err
    end
```

Two gates keep this inside the 1,000 writes/day budget: `write_needed` for
`st:event` and `dirty` for `st:err`. A healthy tick writes only `st:snapshot`.

The fetch window is `window_minutes(rules)` — the widest enabled `lead_min` plus a
5-minute buffer, clamped to 30..=120. A longer window costs no extra API call,
only a longer response, so a rule asking for 45 minutes of warning gets it.

---

## 2. Event state machine — `domain::state::step`

This is what turns a condition that matches on ~7 consecutive ticks into one
message.

```mermaid
flowchart TD
    S([step]) --> M{condition<br/>matched?}

    M -->|no| O1{event open?}
    O1 -->|no| N1[Nothing<br/>no write]
    O1 -->|yes| Q{quiet >= 30 min?}
    Q -->|no| N2[Nothing<br/>no write]
    Q -->|yes| CL[Close<br/>ALL-CLEAR + write]

    M -->|yes| O2{event open?}
    O2 -->|no| OP[Open<br/>ALERT + write]
    O2 -->|yes| T{peak tier ><br/>stored peak?}
    T -->|yes| ES[Escalate<br/>ALERT + write]
    T -->|no| N3[Nothing<br/>no write<br/>same or lower tier]
```

The ratchet compares **peak** tiers, so a forecast that worsens anywhere inside
the lead window escalates even when the onset time has not moved.

Tier only ratchets upward inside an open event; the peak is retained until close,
so falling intensity is silent. Three of the six outcomes write nothing — that is
what keeps 280 daily ticks inside the KV budget.

(Drawn as a decision tree rather than a state diagram on purpose: the three
distinct `Open -> Open` transitions collapse into one edge in `stateDiagram-v2`,
which silently hides the escalation path.)

---

## 3. Disaster path — `tasks::warning_tick`

Runs 24/7 and is never silenced by the active window.

```mermaid
sequenceDiagram
    autonumber
    participant C as Cron */10
    participant T as tasks::warning_tick
    participant MET as data.gov.my
    participant D as domain::met
    participant KV as KV (WX)
    participant TG as Telegram

    C->>T: ScheduledEvent (any non-*/3 cron)
    T->>T: Settings::load
    alt unconfigured
        T-->>C: log + return
    end

    T->>MET: GET /weather/warning/ (trailing slash)
    alt error or HTTP != 200
        T-->>C: log + return
    end
    T->>D: parse [Warning]
    T->>KV: get st:met (seen keys)

    T->>D: select_observe
    Note over D,KV: drop No Advisory sentinel<br/>drop expired (null valid_to = open-ended)<br/>dedup on issued|title|valid_from|valid_to

    alt nothing new
        T-->>C: return (no write)
    end

    loop each new warning
        alt MET_MODE=observe
            T->>TG: [OBSERVE] compact summary<br/>(title + heading + validity only)
        else MET_MODE=filter
            T->>D: passes_filter(severity, place names)
            opt passes
                T->>TG: warning (land text only, 4096 cap)
            end
        end
    end
    T->>KV: put st:met
```

Observe mode sends the compact form deliberately: the severity vocabulary being
harvested lives in `title_en` and `heading_en`, so forwarding 1.5 KB of prose per
warning would add noise without adding signal. Filter mode sends the full land text.

Sending precedes the `st:met` write on purpose: delivery is at-least-once, so a
failed write costs a duplicate rather than a dropped warning.

---

## 4. Report — `GET /api/report` and `/now`

```mermaid
sequenceDiagram
    autonumber
    participant U as Caller
    participant W as lib.rs fetch
    participant T as tasks::build_report
    participant KV as KV (WX)
    participant API as Tomorrow.io
    participant MET as data.gov.my

    U->>W: GET /api/report
    W->>W: ct_eq(Bearer, API_TOKEN)
    alt mismatch
        W-->>U: 401
    end

    W->>T: build_report
    alt unconfigured
        T-->>U: "Not configured yet: X"
    end
    T->>KV: get cfg:rules
    alt unreadable
        T-->>U: "Rule table unreadable"
    end

    T->>KV: get st:quota
    T->>T: try_spend(cap 4/hour)
    alt budget left
        T->>KV: put st:quota
        T->>API: live timeline
        alt fetch fails
            T->>KV: get st:snapshot (stale=true)
        end
    else exhausted
        T->>KV: get st:snapshot (stale=true)
    end

    T->>T: have_data = !timeline.is_empty()
    Note over T,API: empty snapshot renders NO DATA,<br/>never "no rain expected"
    T->>MET: district forecast (server-side filtered)
    T->>MET: active warnings
    T-->>U: rendered report
```

The 4/hour manual cap sits under the 5 calls/hour the cron leaves unused, so a
manual report can never push the cron path into a 429.

---

## 5. Bot commands — `POST /webhook/telegram`

```mermaid
sequenceDiagram
    autonumber
    participant TG as Telegram
    participant W as lib.rs fetch
    participant T as tasks::handle_command
    participant KV as KV (WX)

    TG->>W: POST /webhook/telegram
    W->>W: ct_eq(X-Telegram-Bot-Api-Secret-Token)
    alt mismatch
        W-->>TG: 200 empty (quiet to probes)
    end
    W->>W: ct_eq(chat_id, OWNER_CHAT_ID)
    alt mismatch
        Note over W,T: the allowlist matters more than the token
        W-->>TG: 200 empty
    end

    W->>T: commands::parse(text)
    alt /now
        T->>T: build_report
    else /rules (read or write)
        T->>KV: rules_for_edit
        Note over T,KV: on unreadable, refuse — defaulting<br/>would overwrite every rule
        opt mutation
            T->>KV: put cfg:rules
        end
    else /help or unknown
        T->>T: static reply
    end
    W->>TG: sendMessage (redacted)
    W-->>TG: 200 empty
```

`/health` is answered before `Settings::load`, so it works even when the Worker
is unconfigured.

---

## 6. Morning brief — `tasks::daily_tick`

Runs at 08:00 MYT, before the rain window opens. Carries the day's outlook and,
more importantly, proves the monitor is alive.

```mermaid
sequenceDiagram
    autonumber
    participant C as Cron 0 0 * * *
    participant T as tasks::daily_tick
    participant MET as data.gov.my
    participant KV as KV (WX)
    participant TG as Telegram

    C->>T: ScheduledEvent
    T->>T: Settings::load
    alt unconfigured
        T-->>C: log + return
    end

    T->>MET: district forecast (today)
    T->>MET: active warnings
    Note over T,MET: both unmetered, and failure degrades<br/>to "forecast unavailable" rather than to silence
    T->>KV: get st:daily (yesterday's tally)
    T->>TG: outlook + warnings + tally + liveness line
    T->>KV: put st:daily (reset for today)
```

The final line of the message states the contract explicitly: *"If this message
stops arriving, the monitor is down."* Without it, a dead Worker and a quiet day
produce identical output — silence — which is the one failure mode an alerting
system cannot have. The brief is sent even when both MET calls fail, because its
primary job is liveness, not weather.

The tally (`st:daily`, counts of opens by tier plus escalations) exists so
thresholds can be tuned from what actually fired rather than from guesswork.
