//! Telegram command parsing. Pure, so the grammar is testable without a bot.

use crate::domain::rules::Rule;

#[derive(Debug)]
pub enum Command {
  Now,
  ListRules,
  AddRule(Rule),
  DelRule(String),
  SetEnabled(String, bool),
  Help,
  Unknown(String),
  ParseError(String),
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
            Ok(r) if r.id.is_empty() => Command::ParseError("rule id must not be empty".into()),
            Ok(r) if !crate::domain::tomorrow::is_valid_field(&r.field) => Command::ParseError(format!("unknown field {:?}. Valid: {}", r.field, crate::domain::tomorrow::VALID_FIELDS.join(", "))),
            Ok(r) => Command::AddRule(r),
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
