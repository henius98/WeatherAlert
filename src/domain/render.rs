//! Telegram message bodies. Pure so the wording is testable without a network.

use crate::domain::forecast::DayForecast;
use crate::domain::state::Action;
use crate::domain::tiers::Tier;

pub const TG_LIMIT: usize = 4096;

fn eta(min: u32) -> String {
    match min {
        0 => "now".to_string(),
        1 => "in ~1 min".to_string(),
        n => format!("in ~{n} min"),
    }
}

pub fn alert(action: &Action, label: &str, rule_ids: &[String]) -> Option<String> {
    let rules = if rule_ids.is_empty() {
        String::new()
    } else {
        format!("\nrules: {}", rule_ids.join(", "))
    };
    match action {
        Action::Open { tier, eta_min } => Some(format!(
            "\u{1F327} {} rain {} \u{2014} {}{}",
            cap(tier.label()),
            eta(*eta_min),
            label,
            rules
        )),
        Action::Escalate { from, to, eta_min } => Some(format!(
            "\u{26A1} Intensifying: {} \u{2192} {} {} \u{2014} {}{}",
            from.label(),
            to.label(),
            eta(*eta_min),
            label,
            rules
        )),
        Action::Close { duration_min } => Some(format!(
            "\u{2705} Rain has passed \u{2014} {label} (lasted ~{duration_min} min)"
        )),
        Action::Nothing => None,
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The `/now` and `GET /api/report` body.
pub fn report(
    label: &str,
    tier: Tier,
    next_rain_min: Option<u32>,
    days: &[DayForecast],
    warnings: &[String],
    stale: bool,
    have_data: bool,
) -> String {
    let mut s = format!("\u{1F4CD} {label}\n");
    // "no data" and "no rain" must never render the same. Reporting "no rain
    // expected" when the nowcast is simply missing is a false all-clear.
    s.push_str(&match next_rain_min {
        _ if !have_data => "Next 30 min: NO DATA \u{2014} nowcast unavailable\n".to_string(),
        Some(m) if tier != Tier::None => format!("Next 30 min: {} rain {}\n", tier.label(), eta(m)),
        Some(m) => format!("Next 30 min: precipitation {}\n", eta(m)),
        None => "Next 30 min: no rain expected\n".to_string(),
    });
    if stale {
        s.push_str("(cached \u{2014} hourly manual-refresh cap reached)\n");
    }
    if !days.is_empty() {
        s.push_str("\nMET forecast\n");
        for d in days {
            let t = d.temp_range().map(|r| format!(" {r}")).unwrap_or_default();
            s.push_str(&format!("  {} {}{}\n", d.date, d.summary_en(), t));
        }
    }
    if !warnings.is_empty() {
        s.push_str("\n\u{26A0}\u{FE0F} Active warnings\n");
        for w in warnings {
            s.push_str(&format!("  {w}\n"));
        }
    }
    truncate(s, TG_LIMIT)
}

pub fn truncate(mut s: String, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s;
    }
    s = s.chars().take(limit.saturating_sub(12)).collect();
    s.push_str("\n[truncated]");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_escalate_read_differently() {
        let ids = vec!["rain_mod".to_string()];
        let o = alert(&Action::Open { tier: Tier::Moderate, eta_min: 18 }, "Sepang", &ids).unwrap();
        assert!(o.contains("Moderate rain in ~18 min"));
        assert!(o.contains("Sepang") && o.contains("rain_mod"));

        let e = alert(
            &Action::Escalate { from: Tier::Moderate, to: Tier::Heavy, eta_min: 6 },
            "Sepang",
            &ids,
        )
        .unwrap();
        assert!(e.contains("moderate \u{2192} heavy"), "got {e}");
    }

    #[test]
    fn nothing_renders_no_message() {
        assert!(alert(&Action::Nothing, "Sepang", &[]).is_none());
    }

    #[test]
    fn eta_zero_reads_as_now() {
        let o = alert(&Action::Open { tier: Tier::Heavy, eta_min: 0 }, "Sepang", &[]).unwrap();
        assert!(o.contains("rain now"), "got {o}");
    }

    #[test]
    fn report_states_the_quiet_case_plainly() {
        let r = report("Sepang", Tier::None, None, &[], &[], false, true);
        assert!(r.contains("no rain expected"));
        assert!(!r.contains("MET forecast"));
        assert!(!r.contains("cached"));
    }

    #[test]
    fn report_marks_a_cached_result() {
        let r = report("Sepang", Tier::None, None, &[], &[], true, true);
        assert!(r.contains("cached"));
    }

    #[test]
    fn missing_data_never_renders_as_no_rain() {
        let none = report("Sepang", Tier::None, None, &[], &[], true, false);
        assert!(none.contains("NO DATA"), "got {none}");
        assert!(!none.contains("no rain expected"), "a false all-clear: {none}");
    }

    #[test]
    fn every_message_fits_the_telegram_cap() {
        let warns: Vec<String> = (0..400).map(|i| format!("warning number {i} with padding text")).collect();
        let r = report("Sepang", Tier::Heavy, Some(3), &[], &warns, false, true);
        assert!(r.chars().count() <= TG_LIMIT, "got {}", r.chars().count());
        assert!(r.ends_with("[truncated]"));
    }

    #[test]
    fn truncate_is_a_noop_below_the_limit() {
        assert_eq!(truncate("short".to_string(), 4096), "short");
    }
}
