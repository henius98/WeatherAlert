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
