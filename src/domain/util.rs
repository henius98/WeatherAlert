//! Small pure helpers that the Worker SDK does not provide.

/// Length-independent-ish constant-time comparison for secrets. Returns false
/// on length mismatch (lengths are not secret here) and otherwise accumulates
/// XOR across every byte so timing does not leak the matching prefix.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Percent-encodes for a query-string value. Avoids pulling in a url crate for
/// the sake of the 3 MB bundle ceiling.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Removes secret values from text bound for a user-visible channel.
///
/// Upstream errors can quote the request URL, and the Tomorrow.io key travels
/// as a query parameter, so an unredacted error message is a credential leak.
/// Empty secrets are skipped — otherwise every message would be shredded.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for sec in secrets {
        if sec.len() >= 8 {
            out = out.replace(sec, "***");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_only_identical_strings() {
        assert!(ct_eq("abc123", "abc123"));
        assert!(!ct_eq("abc123", "abc124"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn ct_eq_rejects_prefixes() {
        // A naive starts_with check would accept this.
        assert!(!ct_eq("secret", "secretlonger"));
        assert!(!ct_eq("secretlonger", "secret"));
    }

    #[test]
    fn urlencode_escapes_reserved_characters() {
        assert_eq!(urlencode("2.8333,101.7"), "2.8333%2C101.7");
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("safe-_.~"), "safe-_.~");
    }

    #[test]
    fn redact_strips_an_api_key_quoted_back_in_an_error() {
        let key = "abcd1234efgh5678";
        let err = format!("failed to parse URL from https://api.tomorrow.io/v4/timelines?apikey={key}");
        let out = redact(&err, &[key, "tgtoken9999999"]);
        assert!(!out.contains(key), "key leaked: {out}");
        assert!(out.contains("***"));
        assert!(out.contains("api.tomorrow.io"), "non-secret context should survive");
    }

    #[test]
    fn redact_ignores_short_or_empty_secrets() {
        // An unset secret is "" — redacting it would destroy every message.
        assert_eq!(redact("hello world", &[""]), "hello world");
        assert_eq!(redact("hello world", &["o"]), "hello world");
    }

    #[test]
    fn redact_removes_every_occurrence() {
        let k = "supersecretvalue";
        let out = redact(&format!("{k} then {k}"), &[k]);
        assert_eq!(out, "*** then ***");
    }

    #[test]
    fn urlencode_handles_multibyte() {
        assert_eq!(urlencode("é"), "%C3%A9");
    }
}
