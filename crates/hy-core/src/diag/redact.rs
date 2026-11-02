//! The second privacy pass for the diagnostics zip (V1-5 contract 11: "diagnostics zips
//! from the companion run the same redaction before the user reviews them").
//!
//! The log is written from typed values only (see [`super::V`]), so nothing personal
//! should be in it. Before a line enters a zip it is still checked: a line that is not
//! one of the log's JSON objects is dropped, and in every string that remains
//! (including the `detail` JSON inside it) Windows and home-folder paths, e-mail
//! addresses, long hexadecimal ids and GUIDs, and the PC's own names given by the caller
//! (user name, PC name, domain, profile folder) are replaced by a marker.

use serde_json::Value;

/// What the redaction did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Redaction {
    /// Lines kept.
    pub kept: usize,
    /// Lines dropped (not one of the log's JSON objects).
    pub dropped: usize,
    /// Values changed by a replacement.
    pub scrubbed: usize,
}

const TERMINATORS: &[char] = &[
    '"', '\'', ' ', '\t', '\r', '\n', ',', ';', ')', ']', '}', '|', '<', '>', '`',
];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Replace every case-insensitive (ASCII) occurrence of `needle` in `s` with `marker`.
fn replace_ci(s: &str, needle: &str, marker: &str) -> String {
    if needle.is_empty() {
        return s.to_owned();
    }
    let lower = s.to_ascii_lowercase();
    let nl = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&nl) {
        out.push_str(&s[i..i + pos]);
        out.push_str(marker);
        i += pos + nl.len();
    }
    out.push_str(&s[i..]);
    out
}

/// Replace paths: `X:\...`, `X:/...`, `\\server\...` (and `\\?\`), and `/home/...`,
/// `/Users/...`, `/root/...` up to the next terminator.
fn replace_paths(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let prev_ok = i == 0 || !is_word(chars[i - 1]);
        let drive = prev_ok
            && chars[i].is_ascii_alphabetic()
            && chars.get(i + 1) == Some(&':')
            && matches!(chars.get(i + 2), Some('\\' | '/'));
        let unc = chars[i] == '\\'
            && chars.get(i + 1) == Some(&'\\')
            && chars
                .get(i + 2)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '?' || *c == '.');
        let home = prev_ok
            && chars[i] == '/'
            && ["home/", "Users/", "users/", "root/"].iter().any(|p| {
                let p: Vec<char> = p.chars().collect();
                chars.get(i + 1..i + 1 + p.len()) == Some(&p[..])
            });
        if drive || unc || home {
            while i < chars.len() && !TERMINATORS.contains(&chars[i]) {
                i += 1;
            }
            out.push_str("[path]");
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Replace e-mail addresses (`local@domain.tld`).
fn replace_emails(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut start = out.len();
            while start > 0 && local(out[start - 1]) {
                start -= 1;
            }
            let mut end = i + 1;
            while end < chars.len() && domain(chars[end]) {
                end += 1;
            }
            let dom: String = chars[i + 1..end].iter().collect();
            let dom = dom.trim_end_matches('.');
            if start < out.len() && dom.contains('.') && !dom.starts_with('.') {
                out.truncate(start);
                out.extend("[email]".chars());
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.into_iter().collect()
}

/// Replace runs of 16 or more hexadecimal digits (dashes allowed between them, as in a
/// GUID): device and user ids, keys, tokens.
fn replace_ids(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() && (i == 0 || !is_word(chars[i - 1])) {
            let mut j = i;
            let mut digits = 0;
            while j < chars.len() && (chars[j].is_ascii_hexdigit() || chars[j] == '-') {
                if chars[j] != '-' {
                    digits += 1;
                }
                j += 1;
            }
            // Do not end on a dash, and the run must end at a word boundary.
            while j > i && chars[j - 1] == '-' {
                j -= 1;
            }
            let ends = j == chars.len() || !is_word(chars[j]);
            if digits >= 16 && ends {
                out.push_str("[id]");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// One string with every replacement applied. `planted` are the PC's own names; values
/// shorter than three characters are ignored (they would match everywhere).
#[must_use]
pub fn scrub(s: &str, planted: &[&str]) -> String {
    // Paths first, whole (a name replaced inside one would end it early).
    let mut out = replace_ids(&replace_emails(&replace_paths(s)));
    // Longest first, so a longer name goes before a shorter one inside it.
    let mut names: Vec<&str> = planted
        .iter()
        .map(|p| p.trim())
        .filter(|p| p.chars().count() >= 3)
        .collect();
    names.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for p in names {
        out = replace_ci(&out, p, "[redacted]");
    }
    out
}

fn scrub_value(v: &mut Value, planted: &[&str], changed: &mut usize) {
    match v {
        Value::String(s) => {
            let new = scrub(s, planted);
            if new != *s {
                *changed += 1;
                *s = new;
            }
        }
        Value::Array(a) => {
            for x in a {
                scrub_value(x, planted, changed);
            }
        }
        Value::Object(m) => {
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                let nk = scrub(&k, planted);
                if let Some(mut x) = m.remove(&k) {
                    scrub_value(&mut x, planted, changed);
                    if nk != k {
                        *changed += 1;
                    }
                    m.insert(nk, x);
                }
            }
        }
        _ => {}
    }
}

/// Whether `v` has the log line's shape: an object with a string `timestamp`, a string
/// `level` and a `fields` object holding a string `event`.
fn is_log_line(v: &Value) -> bool {
    v.get("timestamp").is_some_and(Value::is_string)
        && v.get("level").is_some_and(Value::is_string)
        && v.get("fields")
            .and_then(|f| f.get("event"))
            .is_some_and(Value::is_string)
}

/// Redact one log line: `None` if it is dropped, otherwise the line (re-serialised) and
/// how many values changed.
#[must_use]
pub fn redact_line(line: &str, planted: &[&str]) -> Option<(String, usize)> {
    let mut v: Value = serde_json::from_str(line.trim()).ok()?;
    if !is_log_line(&v) {
        return None;
    }
    let mut changed = 0;
    scrub_value(&mut v, planted, &mut changed);
    Some((v.to_string(), changed))
}

/// Redact a whole log text, line by line.
#[must_use]
pub fn redact_log(text: &str, planted: &[&str]) -> (Vec<String>, Redaction) {
    let mut r = Redaction::default();
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match redact_line(line, planted) {
            Some((l, n)) => {
                r.kept += 1;
                r.scrubbed += n;
                out.push(l);
            }
            None => r.dropped += 1,
        }
    }
    (out, r)
}
