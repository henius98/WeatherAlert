//! Telegram command parsing. Pure, so the grammar is testable without a bot.

use crate::domain::rules::Rule;

#[cfg_attr(test, derive(PartialEq))]
#[derive(Debug)]
pub enum Command {
    Now,
    ListRules,
    AddRule(Box<Rule>),
    DelRule(String),
    SetEnabled(String, bool),
    Help,
    Unknown(String),
    ParseError(String),
}

/// Test-only, and deliberately scoped that way: this compares identity fields
/// only, ignoring op/value/lead_min/enabled. Exposing it in production would
/// make `rule_a == rule_b` quietly true for rules with different thresholds,
/// which is exactly the kind of thing a future dedup would get wrong.
#[cfg(test)]
impl PartialEq for Rule {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.field == o.field && self.group == o.group
    }
}

/// Accepts an optional `@botname` suffix, which Telegram appends in groups.
pub fn parse(text: &str) -> Command {
    let t = text.trim();
    let (head, rest) = match t.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (t, ""),
    };
    let head = head.split('@').next().unwrap_or(head).to_lowercase();

    match head.as_str() {
        "/now" | "/weather" => Command::Now,
        "/help" | "/start" => Command::Help,
        "/rules" => match rest.split_once(char::is_whitespace) {
            None if rest.is_empty() => Command::ListRules,
            None => Command::Unknown(format!("/rules {rest}")),
            Some((verb, arg)) => {
                let arg = arg.trim();
                match verb.to_lowercase().as_str() {
                    "add" => match serde_json::from_str::<Rule>(arg) {
                        Ok(r) if r.id.is_empty() => {
                            Command::ParseError("rule id must not be empty".into())
                        }
                        Ok(r) => Command::AddRule(Box::new(r)),
                        Err(e) => Command::ParseError(e.to_string()),
                    },
                    "del" | "rm" => Command::DelRule(arg.to_string()),
                    "on" => Command::SetEnabled(arg.to_string(), true),
                    "off" => Command::SetEnabled(arg.to_string(), false),
                    other => Command::Unknown(format!("/rules {other}")),
                }
            }
        },
        _ => Command::Unknown(head),
    }
}

pub const HELP: &str = "\
/now - current nowcast, MET forecast and active warnings
/rules - list rules
/rules add {json} - add a rule
/rules del <id> - remove a rule
/rules on|off <id> - enable or disable a rule

Rule shape:
{\"id\":\"rain\",\"group\":null,\"field\":\"rainIntensity\",\"op\":\"gte\",\"value\":2.5,\"lead_min\":20}
group null fires alone; rules sharing a group must all hold in the same minute.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_simple_commands() {
        assert_eq!(parse("/now"), Command::Now);
        assert_eq!(parse("  /NOW  "), Command::Now);
        assert_eq!(parse("/help"), Command::Help);
        assert_eq!(parse("/rules"), Command::ListRules);
    }

    #[test]
    fn tolerates_the_botname_suffix_telegram_adds() {
        assert_eq!(parse("/now@my_weather_bot"), Command::Now);
        assert_eq!(parse("/rules@my_weather_bot"), Command::ListRules);
    }

    #[test]
    fn parses_add_with_a_full_rule() {
        let c = parse(r#"/rules add {"id":"r1","field":"rainIntensity","op":"gte","value":2.5}"#);
        match c {
            Command::AddRule(r) => {
                assert_eq!(r.id, "r1");
                assert_eq!(r.lead_min, 20, "lead_min should default");
                assert!(r.enabled, "enabled should default true");
            }
            other => panic!("expected AddRule, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_reports_an_error_rather_than_dropping_silently() {
        match parse("/rules add {not json}") {
            Command::ParseError(_) => {}
            other => panic!("expected ParseError, got {other:?}"),
        }
        match parse(r#"/rules add {"id":"","field":"x","op":"gte","value":1}"#) {
            Command::ParseError(m) => assert!(m.contains("id")),
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn parses_del_and_toggle() {
        assert_eq!(parse("/rules del r1"), Command::DelRule("r1".into()));
        assert_eq!(parse("/rules rm r1"), Command::DelRule("r1".into()));
        assert_eq!(parse("/rules on r1"), Command::SetEnabled("r1".into(), true));
        assert_eq!(parse("/rules off r1"), Command::SetEnabled("r1".into(), false));
    }

    #[test]
    fn unknown_commands_are_reported_not_ignored() {
        assert_eq!(parse("/launch"), Command::Unknown("/launch".into()));
        assert_eq!(parse("/rules frobnicate x"), Command::Unknown("/rules frobnicate".into()));
    }

    #[test]
    fn plain_text_is_unknown_rather_than_an_error() {
        match parse("hello there") {
            Command::Unknown(_) => {}
            other => panic!("got {other:?}"),
        }
    }
}
