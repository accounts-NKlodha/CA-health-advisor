//! The diagnostics zip (architecture 10: "user-reviewed diagnostics zip"; nothing is ever
//! uploaded).
//!
//! Settings > App & updates > "Prepare diagnostics..." builds the bundle in memory: every
//! log file, oldest first, through [`super::redact`], the newest lines up to
//! [`MAX_LOG_BYTES`], as `companion.log`, and an `about.txt` with the version, the Windows
//! build and what the redaction did. The page shows the user each file's full text; only
//! "Save zip..." writes it, byte for byte what was shown, where the user chooses.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use super::redact::{Redaction, redact_log, scrub};
use super::zip::{self, DosTime};

/// The most log text a zip holds (the newest lines; older ones are left out and counted).
pub const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;

/// One file of the zip, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// The file's name in the zip.
    pub name: String,
    /// Its size, bytes.
    pub bytes: usize,
    /// Its lines.
    pub lines: usize,
    /// Its full text.
    pub text: String,
}

/// What the page shows before saving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    /// The files, in zip order.
    pub entries: Vec<Entry>,
    /// Log lines in the zip.
    pub kept_lines: usize,
    /// Log lines left out because they were not readable log lines.
    pub dropped_lines: usize,
    /// Log lines left out because the zip was full (the oldest).
    pub older_lines_left_out: usize,
    /// Values the redaction replaced.
    pub scrubbed: usize,
    /// How many lines of each event the log part holds.
    pub events: BTreeMap<String, u64>,
    /// The suggested file name.
    pub suggested_name: String,
    /// The zip's size, bytes.
    pub zip_bytes: usize,
}

/// A built bundle: the preview and the zip it describes.
#[derive(Debug, Clone)]
pub struct Bundle {
    /// What the page shows.
    pub preview: Preview,
    /// The zip, exactly the shown files.
    pub zip: Vec<u8>,
}

fn event_of(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(v.get("fields")?.get("event")?.as_str()?.to_owned())
}

/// A local date and time for the name and the zip's entry times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Year, month, day.
    pub date: (i32, u32, u32),
    /// Hour, minute, second.
    pub time: (u32, u32, u32),
}

/// Build the bundle from the log files in `log_dir`. `about` are extra `name: value`
/// lines for `about.txt` (they go through the same redaction); `planted` are the PC's
/// own names to remove (see [`super::redact::scrub`]).
#[must_use]
pub fn build(log_dir: &Path, about: &[(&str, String)], planted: &[&str], at: Stamp) -> Bundle {
    let mut lines: Vec<String> = Vec::new();
    let mut total = Redaction::default();
    for f in super::log::files(log_dir) {
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        let (l, r) = redact_log(&String::from_utf8_lossy(&bytes), planted);
        total.kept += r.kept;
        total.dropped += r.dropped;
        total.scrubbed += r.scrubbed;
        lines.extend(l);
    }
    // The newest lines that fit.
    let mut size = 0;
    let mut first = lines.len();
    while first > 0 && size + lines[first - 1].len() < MAX_LOG_BYTES {
        size += lines[first - 1].len() + 1;
        first -= 1;
    }
    let left_out = first;
    let kept = &lines[first..];
    let mut events: BTreeMap<String, u64> = BTreeMap::new();
    for l in kept {
        if let Some(e) = event_of(l) {
            *events.entry(e).or_default() += 1;
        }
    }
    let mut log_text = kept.join("\n");
    if !log_text.is_empty() {
        log_text.push('\n');
    }

    let (y, mo, d) = at.date;
    let (h, mi, s) = at.time;
    let mut about_text = String::from(
        "HealthierYou diagnostics\r\n\
         \r\n\
         This zip was made on this PC when you chose \"Prepare diagnostics\" and holds only\r\n\
         what the preview showed. It is never sent anywhere by HealthierYou: share it only\r\n\
         if you want to.\r\n\
         \r\n\
         companion.log holds event names, counts, sizes and times: no health values, no\r\n\
         names, no file paths and no device ids. Before it was added every line was checked\r\n\
         again and anything that looked like a path, an e-mail address, a long id or this\r\n\
         PC's user or computer name was replaced.\r\n\
         \r\n",
    );
    about_text.push_str(&format!("Made: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}\r\n"));
    for (k, v) in about {
        about_text.push_str(&format!("{}: {}\r\n", scrub(k, planted), scrub(v, planted)));
    }
    about_text.push_str(&format!(
        "Log lines: {} included, {} older lines left out (size limit), {} unreadable lines left out, {} values replaced\r\n",
        kept.len(),
        left_out,
        total.dropped,
        total.scrubbed
    ));

    let at_dos = DosTime::new(y, mo, d, h, mi, s);
    let zip = zip::write(
        &[
            ("about.txt", about_text.as_bytes()),
            ("companion.log", log_text.as_bytes()),
        ],
        at_dos,
    );
    let entry = |name: &str, text: String| Entry {
        name: name.to_owned(),
        bytes: text.len(),
        lines: text.lines().count(),
        text,
    };
    let preview = Preview {
        entries: vec![
            entry("about.txt", about_text),
            entry("companion.log", log_text),
        ],
        kept_lines: kept.len(),
        dropped_lines: total.dropped,
        older_lines_left_out: left_out,
        scrubbed: total.scrubbed,
        events,
        suggested_name: format!("HealthierYou-diagnostics-{y:04}-{mo:02}-{d:02}.zip"),
        zip_bytes: zip.len(),
    };
    Bundle { preview, zip }
}
