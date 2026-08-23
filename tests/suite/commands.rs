//! Tests for `domain::commands`.
//! Moved out of the source file so the whole suite lives in one place.

use weather_alert::domain::commands::*;

#[test]
fn parses_the_simple_commands() {
    assert!(matches!(parse("/now"), Command::Now));
    assert!(matches!(parse("  /NOW  "), Command::Now));
    assert!(matches!(parse("/help"), Command::Help));
    assert!(matches!(parse("/rules"), Command::ListRules));
}

#[test]
fn tolerates_the_botname_suffix_telegram_adds() {
    assert!(matches!(parse("/now@my_weather_bot"), Command::Now));
    assert!(matches!(parse("/rules@my_weather_bot"), Command::ListRules));
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
    match parse(r#"/rules add {"id":"","field":"rainIntensity","op":"gte","value":1}"#) {
        Command::ParseError(m) => assert!(m.contains("id")),
        other => panic!("expected ParseError, got {other:?}"),
    }
    // A typo'd field would 400 every subsequent fetch, so reject it here.
    match parse(r#"/rules add {"id":"r","field":"rainIntesity","op":"gte","value":1}"#) {
        Command::ParseError(m) => assert!(m.contains("unknown field"), "got {m}"),
        other => panic!("expected ParseError, got {other:?}"),
    }
}

#[test]
fn parses_del_and_toggle() {
    assert!(matches!(parse("/rules del r1"), Command::DelRule(ref s) if s == "r1"));
    assert!(matches!(parse("/rules rm r1"), Command::DelRule(ref s) if s == "r1"));
    assert!(matches!(parse("/rules on r1"), Command::SetEnabled(ref s, true) if s == "r1"));
    assert!(matches!(parse("/rules off r1"), Command::SetEnabled(ref s, false) if s == "r1"));
}

#[test]
fn unknown_commands_are_reported_not_ignored() {
    assert!(matches!(parse("/launch"), Command::Unknown(ref s) if s == "/launch"));
    assert!(matches!(parse("/rules frobnicate x"),
                     Command::Unknown(ref s) if s == "/rules frobnicate"));
}

#[test]
fn plain_text_is_unknown_rather_than_an_error() {
    match parse("hello there") {
        Command::Unknown(_) => {}
        other => panic!("got {other:?}"),
    }
}
