use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use serde_json::Value;

use super::bundle::{self, MAX_LOG_BYTES, Stamp};
use super::log::{self, LOG_NAME, Limits, RotatingLog};
use super::redact::{redact_line, redact_log, scrub};
use super::resources::{ProcessReading, ProcessUse, Summary, TreeSample};
use super::zip::{self, DosTime};
use super::{
    Level, MarkerFile, RESTART_GAP_S, RecoveryBudget, RestartGuard, RestartMarker, V, line,
    panic_line, reserve_restart, restart_allowed, short_location, webview_failure,
};

const T0: u64 = 1_793_000_000; // November 2026

fn at(s: u64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(i64::try_from(s).unwrap(), 0).unwrap()
}

fn detail(l: &str) -> Value {
    let v: Value = serde_json::from_str(l).unwrap();
    serde_json::from_str(v["fields"]["detail"].as_str().unwrap()).unwrap()
}

// ---------------------------------------------------------------- line format

#[test]
fn lines_have_the_servers_shape_with_typed_fields_only() {
    let l = line(
        at(T0),
        Level::Info,
        "resources",
        &[
            ("handles", V::N(812)),
            ("cpu_pct", V::Dec(0.123_456)),
            ("window", V::Name("pet")),
            ("ok", V::Bool(true)),
            ("bad", V::Dec(f64::NAN)),
        ],
    );
    assert!(!l.contains('\n'));
    let v: Value = serde_json::from_str(&l).unwrap();
    assert_eq!(v["level"], "INFO");
    assert_eq!(v["fields"]["event"], "resources");
    assert!(v["timestamp"].as_str().unwrap().starts_with("2026-"));
    assert_eq!(log::line_time(&l), Some(T0));
    let d = detail(&l);
    assert_eq!(d["handles"], 812);
    assert_eq!(d["cpu_pct"], 0.12);
    assert_eq!(d["window"], "pet");
    assert_eq!(d["ok"], true);
    assert!(d["bad"].is_null());
}

#[test]
fn panic_lines_hold_a_short_location_and_never_a_message() {
    assert_eq!(short_location("src\\shell.rs"), "src/shell.rs");
    assert_eq!(
        short_location(
            "C:\\Users\\someone\\.cargo\\registry\\src\\index.crates.io-1\\tauri-2.12.4\\src\\app.rs"
        ),
        "src/app.rs"
    );
    assert_eq!(short_location("/home/someone/x/y/z.rs"), "y/z.rs");
    assert_eq!(short_location("\\\\?\\C:\\a.rs"), "a.rs");
    assert_eq!(short_location("main.rs"), "main.rs");
    let l = panic_line(
        at(T0),
        "C:\\Users\\someone\\.cargo\\registry\\src\\dep\\src\\lib.rs",
        42,
        true,
    );
    assert!(!l.contains("someone") && !l.contains("Users"), "{l}");
    let v: Value = serde_json::from_str(&l).unwrap();
    assert_eq!(v["level"], "ERROR");
    assert_eq!(v["fields"]["event"], "panic");
    let d = detail(&l);
    assert_eq!(d["file"], "src/lib.rs");
    assert_eq!(d["line"], 42);
    assert_eq!(d["main_thread"], true);
}

#[test]
fn a_restart_after_a_panic_waits_ten_minutes_from_the_last() {
    assert!(restart_allowed(None, T0));
    assert!(!restart_allowed(Some(T0), T0 + 5));
    assert!(!restart_allowed(Some(T0), T0 + RESTART_GAP_S - 1));
    assert!(restart_allowed(Some(T0), T0 + RESTART_GAP_S));
    // A clock set back does not block for ever.
    assert!(restart_allowed(Some(T0 + 10_000), T0));
}

/// A restart marker that can be made to fail, standing in for the marker file.
#[derive(Default)]
struct FakeMarker {
    text: std::cell::RefCell<Option<String>>,
    read_fails: bool,
    write_fails: bool,
    /// The write reports success but nothing is kept (a write the disk lost).
    write_lost: bool,
}

impl RestartMarker for FakeMarker {
    fn read(&self) -> std::io::Result<Option<String>> {
        if self.read_fails {
            return Err(std::io::Error::other("injected read fault"));
        }
        Ok(self.text.borrow().clone())
    }

    fn write(&self, text: &str) -> std::io::Result<()> {
        if self.write_fails {
            return Err(std::io::Error::other("injected write fault (disk full)"));
        }
        if !self.write_lost {
            *self.text.borrow_mut() = Some(text.to_owned());
        }
        Ok(())
    }
}

/// Panics at start-up, one process after another, `gap` seconds apart: how many of them
/// started the app again (each panic is a fresh process reading the same marker).
fn relaunches(marker: &impl RestartMarker, panics: u64, gap: u64) -> u64 {
    (0..panics)
        .filter(|i| reserve_restart(marker, T0 + i * gap) == RestartGuard::Reserved)
        .count() as u64
}

#[test]
fn a_panic_restart_is_refused_when_its_marker_cannot_be_saved() {
    // The marker cannot be written: no restart at all, panic after panic.
    let full = FakeMarker {
        write_fails: true,
        ..FakeMarker::default()
    };
    assert_eq!(
        reserve_restart(&full, T0),
        RestartGuard::Refused("marker_not_saved")
    );
    assert_eq!(relaunches(&full, 20, 1), 0);
    // A write that reports success but is not there on reading back is refused too.
    let lost = FakeMarker {
        write_lost: true,
        ..FakeMarker::default()
    };
    assert_eq!(
        reserve_restart(&lost, T0),
        RestartGuard::Refused("marker_not_saved")
    );
    assert_eq!(relaunches(&lost, 20, 1), 0);
    // A working marker allows the first restart only, then one per 10 minutes.
    let ok = FakeMarker::default();
    assert_eq!(relaunches(&ok, 20, 1), 1);
    assert_eq!(*ok.text.borrow(), Some(T0.to_string()));
    let ok = FakeMarker::default();
    assert_eq!(relaunches(&ok, 3, RESTART_GAP_S), 3);
}

#[test]
fn a_panic_restart_is_refused_when_its_marker_is_unreadable_or_corrupt() {
    let unreadable = FakeMarker {
        read_fails: true,
        ..FakeMarker::default()
    };
    assert_eq!(
        reserve_restart(&unreadable, T0),
        RestartGuard::Refused("marker_unreadable")
    );
    assert_eq!(relaunches(&unreadable, 5, RESTART_GAP_S), 0);
    for bad in ["", "soon", "-5", "1.5", "99999999999999999999999"] {
        let corrupt = FakeMarker {
            text: std::cell::RefCell::new(Some(bad.to_owned())),
            ..FakeMarker::default()
        };
        assert_eq!(
            reserve_restart(&corrupt, T0),
            RestartGuard::Refused("marker_corrupt"),
            "{bad:?}"
        );
        // Not overwritten: it stays refused.
        assert_eq!(*corrupt.text.borrow(), Some(bad.to_owned()));
    }
    // Only an absent marker counts as never restarted.
    assert_eq!(
        reserve_restart(&FakeMarker::default(), T0),
        RestartGuard::Reserved
    );
}

#[test]
fn the_panic_restart_marker_file_guards_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("last-panic-restart");
    // Each panic is a fresh process with its own handle on the same file.
    assert_eq!(
        reserve_restart(&MarkerFile(path.clone()), T0),
        RestartGuard::Reserved
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), T0.to_string());
    assert_eq!(
        reserve_restart(&MarkerFile(path.clone()), T0 + 60),
        RestartGuard::Refused("too_soon")
    );
    assert_eq!(
        reserve_restart(&MarkerFile(path.clone()), T0 + RESTART_GAP_S),
        RestartGuard::Reserved
    );
    // The marker's folder is gone (so the marker cannot be created): the marker reads as
    // absent but the reservation cannot be saved, so no restart, however often it panics.
    let missing = MarkerFile(dir.path().join("gone").join("last-panic-restart"));
    assert_eq!(
        reserve_restart(&missing, T0),
        RestartGuard::Refused("marker_not_saved")
    );
    assert_eq!(relaunches(&missing, 20, RESTART_GAP_S), 0);
    // Something that is not a readable file where the marker should be: refused.
    let blocked = dir.path().join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    assert_eq!(
        reserve_restart(&MarkerFile(blocked), T0),
        RestartGuard::Refused("marker_unreadable")
    );
}

#[test]
fn webview_failures_that_lose_the_page_are_told_apart() {
    for (k, name, lost) in [
        (0, "browser_process_exited", true),
        (1, "render_process_exited", true),
        (2, "render_process_unresponsive", true),
        (3, "frame_render_process_exited", false),
        (6, "gpu_process_exited", false),
        (9, "unknown_process_exited", false),
        (77, "other", false),
    ] {
        let f = webview_failure(k);
        assert_eq!((f.kind, f.page_lost), (name, lost), "{k}");
    }
}

#[test]
fn the_recovery_budget_allows_five_in_ten_minutes() {
    let mut b = RecoveryBudget::pet();
    let t = Instant::now();
    for i in 0..5 {
        assert!(b.try_take(t + Duration::from_secs(i)), "{i}");
    }
    assert!(!b.try_take(t + Duration::from_secs(10)));
    assert!(!b.try_take(t + Duration::from_secs(599)));
    // The first one has aged out.
    assert!(b.try_take(t + Duration::from_secs(600)));
    assert!(!b.try_take(t + Duration::from_secs(600)));
}

// ------------------------------------------------------------------ rotation

fn small() -> Limits {
    Limits {
        rotate_bytes: 1000,
        rotate_age_s: 7 * 86_400,
        keep_s: 7 * 86_400,
        keep_files: 4,
    }
}

fn rotated(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&format!("{LOG_NAME}.")))
        .collect();
    v.sort();
    v
}

#[test]
fn the_log_rotates_at_its_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let log = RotatingLog::open_with(dir.path(), small(), T0).unwrap();
    let l = line(at(T0), Level::Info, "x", &[("n", V::N(1))]);
    let per = l.len() + 1;
    let fit = 1000 / per + 1; // the write that reaches 1000 bytes is still in the file
    for _ in 0..fit {
        log.write_line(&l, T0).unwrap();
    }
    assert!(rotated(dir.path()).is_empty());
    log.write_line(&l, T0 + 1).unwrap();
    assert_eq!(rotated(dir.path()), vec![format!("{LOG_NAME}.{}", T0 + 1)]);
    let current = std::fs::read_to_string(dir.path().join(LOG_NAME)).unwrap();
    assert_eq!(current.lines().count(), 1);
    let old = std::fs::read_to_string(dir.path().join(format!("{LOG_NAME}.{}", T0 + 1))).unwrap();
    assert_eq!(old.lines().count(), fit);
    assert!(old.len() >= 1000);
}

#[test]
fn the_log_rotates_seven_days_after_its_first_line_even_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let log = RotatingLog::open_with(dir.path(), small(), T0).unwrap();
        log.write_line(&line(at(T0), Level::Info, "start", &[]), T0)
            .unwrap();
    }
    // Reopened six days later: the age comes from the first line.
    let log = RotatingLog::open_with(dir.path(), small(), T0 + 6 * 86_400).unwrap();
    log.write_line("{}", T0 + 6 * 86_400).unwrap();
    assert!(rotated(dir.path()).is_empty());
    log.write_line("{}", T0 + 7 * 86_400).unwrap();
    assert_eq!(rotated(dir.path()).len(), 1);
    let current = std::fs::read_to_string(dir.path().join(LOG_NAME)).unwrap();
    assert_eq!(current, "{}\n");
}

#[test]
fn rotated_files_are_kept_seven_days_and_at_most_four() {
    let dir = tempfile::tempdir().unwrap();
    for stamp in [T0 - 8 * 86_400, T0 - 3, T0 - 2, T0 - 1] {
        std::fs::write(dir.path().join(format!("{LOG_NAME}.{stamp}")), "old\n").unwrap();
    }
    std::fs::write(dir.path().join("unrelated.txt"), "keep").unwrap();
    // Opening deletes the one past seven days.
    let log = RotatingLog::open_with(dir.path(), small(), T0).unwrap();
    assert_eq!(rotated(dir.path()).len(), 3);
    assert!(dir.path().join("unrelated.txt").exists());
    // Two more rotations: only the newest four rotated files stay.
    let big = "x".repeat(1200);
    log.write_line(&big, T0).unwrap();
    log.write_line(&big, T0 + 10).unwrap();
    log.write_line(&big, T0 + 20).unwrap();
    let r = rotated(dir.path());
    assert_eq!(r.len(), 4, "{r:?}");
    assert!(!r.contains(&format!("{LOG_NAME}.{}", T0 - 3)), "{r:?}");
    assert!(r.contains(&format!("{LOG_NAME}.{}", T0 + 20)), "{r:?}");
    // files(): oldest first, current last.
    let f = log::files(dir.path());
    assert_eq!(f.len(), 5);
    assert!(f[4].ends_with(LOG_NAME));
    assert!(f[0].to_string_lossy().ends_with(&format!("{}", T0 - 2)));
}

#[test]
fn a_rotation_in_the_same_second_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let log = RotatingLog::open_with(dir.path(), small(), T0).unwrap();
    let big = "y".repeat(1200);
    for _ in 0..3 {
        log.write_line(&big, T0).unwrap();
    }
    assert_eq!(rotated(dir.path()).len(), 2);
}

#[test]
fn a_line_stays_on_one_line_and_the_panic_path_does_not_wait() {
    let dir = tempfile::tempdir().unwrap();
    let log = RotatingLog::open_with(dir.path(), small(), T0).unwrap();
    log.write_line("a\nb\rc", T0).unwrap();
    assert!(log.try_write_line("{\"p\":1}", T0));
    let text = std::fs::read_to_string(dir.path().join(LOG_NAME)).unwrap();
    assert_eq!(text, "a b c\n{\"p\":1}\n");
}

// ----------------------------------------------------------- resource summary

#[test]
fn the_resource_summary_formats_megabytes_and_cpu() {
    let mib = 1024 * 1024;
    let prev = TreeSample {
        cpu: Duration::from_secs(10),
        ..TreeSample::default()
    };
    let now = TreeSample {
        processes: 6,
        working_set: 210 * mib + mib / 2,
        private_working_set: Some(67 * mib),
        private_bytes: 120 * mib,
        handles: 1500,
        threads: 140,
        cpu: Duration::from_secs(10 + 36),
        unreadable: 0,
    };
    // 36 s of CPU in an hour on 4 logical processors: 0.25%.
    let s = Summary::between(
        &prev,
        &now,
        Duration::from_secs(3600),
        4,
        Duration::from_secs(7200),
    );
    let l = line(at(T0), Level::Info, "resources", &s.fields());
    let d = detail(&l);
    assert_eq!(d["processes"], 6);
    assert_eq!(d["working_set_mb"], 210.5);
    assert_eq!(d["private_ws_mb"], 67.0);
    assert_eq!(d["private_bytes_mb"], 120.0);
    assert_eq!(d["handles"], 1500);
    assert_eq!(d["threads"], 140);
    assert_eq!(d["cpu_pct"], 0.25);
    assert_eq!(d["window_s"], 3600);
    assert_eq!(d["uptime_s"], 7200);
    // No private working set reading: the field is left out, never a partial sum.
    let partial = TreeSample {
        private_working_set: None,
        ..now
    };
    let s = Summary::between(&prev, &partial, Duration::ZERO, 4, Duration::ZERO);
    let d = detail(&line(at(T0), Level::Info, "resources", &s.fields()));
    assert!(d.get("private_ws_mb").is_none());
    assert!(d.get("cpu_pct").is_none());
    // A process that ended makes the CPU sum drop: never negative.
    let s = Summary::between(&now, &prev, Duration::from_secs(60), 4, Duration::ZERO);
    assert_eq!(s.cpu_pct, Some(0.0));
}

#[test]
fn an_unreadable_process_leaves_the_private_working_set_out() {
    let mib = 1024 * 1024;
    let p = |ws: u64, private: Option<u64>| {
        ProcessReading::Read(ProcessUse {
            working_set: ws * mib,
            private_working_set: private.map(|v| v * mib),
            private_bytes: ws * mib,
            handles: 100,
            threads: 10,
            cpu: Duration::from_secs(1),
        })
    };
    // Every process read: the private working set is the whole tree's.
    let all = TreeSample::sum([p(100, Some(40)), p(80, Some(30)), p(50, Some(20))]);
    assert_eq!(all.processes, 3);
    assert_eq!(all.private_working_set, Some(90 * mib));
    assert_eq!(all.unreadable, 0);
    // One live WebView2 descendant cannot be opened: no private total, not a smaller one.
    let one_closed = TreeSample::sum([
        p(100, Some(40)),
        ProcessReading::Unreadable,
        p(50, Some(20)),
    ]);
    assert_eq!(one_closed.private_working_set, None);
    assert_eq!(one_closed.unreadable, 1);
    assert_eq!(one_closed.processes, 2);
    let s = Summary::between(
        &TreeSample::default(),
        &one_closed,
        Duration::from_secs(3600),
        4,
        Duration::ZERO,
    );
    let d = detail(&line(at(T0), Level::Info, "resources", &s.fields()));
    assert!(d.get("private_ws_mb").is_none());
    assert_eq!(d["unreadable"], 1);
    // A process that ended before it could be opened is not part of the tree any more.
    let ended = TreeSample::sum([p(100, Some(40)), ProcessReading::Exited, p(50, Some(20))]);
    assert_eq!(ended.private_working_set, Some(60 * mib));
    assert_eq!(ended.unreadable, 0);
    let d = detail(&line(
        at(T0),
        Level::Info,
        "resources",
        &Summary::between(&ended, &ended, Duration::ZERO, 4, Duration::ZERO).fields(),
    ));
    assert!(d.get("unreadable").is_none());
    // A process without the newer counters, or nothing read at all: no private total.
    assert_eq!(
        TreeSample::sum([p(100, Some(40)), p(50, None)]).private_working_set,
        None
    );
    assert_eq!(TreeSample::sum([]).private_working_set, None);
    assert_eq!(
        TreeSample::sum([ProcessReading::Exited]).private_working_set,
        None
    );
}

// ------------------------------------------------------------------ redaction

#[test]
fn redaction_removes_paths_emails_ids_and_the_pcs_names() {
    let planted = ["jdoe", "OFFICE-PC-07", "CORP"];
    let cases = [
        (
            "C:\\Users\\jdoe\\AppData\\Local\\x.db failed",
            "[path] failed",
        ),
        ("at D:/data/hy.db", "at [path]"),
        ("share \\\\server\\hy\\file.zip", "share [path]"),
        ("verbatim \\\\?\\C:\\x", "verbatim [path]"),
        ("/home/jdoe/x and /Users/a/b", "[path] and [path]"),
        ("mail jane.doe+hy@example.co.in now", "mail [email] now"),
        ("dev 0192f1c2-7d3e-7a10-8b55-3c4d5e6f7a8b ok", "dev [id] ok"),
        ("id abcdef0123456789abcdef0123456789", "id [id]"),
        ("pc office-pc-07 in corp", "pc [redacted] in [redacted]"),
        ("user JDOE", "user [redacted]"),
        // Left alone: times, short numbers and hex, names chosen in code.
        ("2026-11-02T10:00:00.123456Z", "2026-11-02T10:00:00.123456Z"),
        (
            "render_process_exited 12345 deadbeef",
            "render_process_exited 12345 deadbeef",
        ),
        ("v1/sync", "v1/sync"),
    ];
    for (input, want) in cases {
        assert_eq!(scrub(input, &planted), want, "{input}");
    }
    // Planted values shorter than three characters are ignored.
    assert_eq!(scrub("a b", &["a", ""]), "a b");
}

#[test]
fn redaction_reaches_inside_the_detail_and_drops_foreign_lines() {
    let raw = r#"{"timestamp":"2026-11-02T10:00:00Z","level":"INFO","fields":{"event":"x","detail":"{\"where\":\"C:\\\\Users\\\\jdoe\\\\a.txt\"}"}}"#;
    let (l, n) = redact_line(raw, &["jdoe"]).unwrap();
    assert_eq!(n, 1);
    assert!(!l.contains("jdoe") && !l.contains("Users"), "{l}");
    assert!(l.contains("[path]"), "{l}");
    let text = format!(
        "{raw}\nnot json at all C:\\Users\\jdoe\n{{\"other\":1}}\n\n{}\n",
        line(at(T0), Level::Info, "ok", &[("n", V::N(3))])
    );
    let (lines, r) = redact_log(&text, &["jdoe"]);
    assert_eq!((r.kept, r.dropped, r.scrubbed), (2, 2, 1));
    assert!(lines.iter().all(|l| !l.contains("jdoe")));
}

// ------------------------------------------------------------------------ zip

#[test]
fn the_zip_is_a_standard_stored_zip_that_reads_back() {
    assert_eq!(zip::crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(zip::crc32(b""), 0);
    let at = DosTime::new(2026, 11, 2, 14, 30, 59);
    assert_eq!(at.date, ((2026 - 1980) << 9) | (11 << 5) | 2);
    assert_eq!(at.time, (14 << 11) | (30 << 5) | 29);
    let z = zip::write(
        &[("a.txt", b"hello"), ("b/c.log", b""), ("d", &[0u8; 300])],
        at,
    );
    assert_eq!(&z[..4], b"PK\x03\x04");
    let back = zip::read(&z).unwrap();
    assert_eq!(back.len(), 3);
    assert_eq!(back[0], ("a.txt".to_owned(), b"hello".to_vec()));
    assert_eq!(back[1].1, Vec::<u8>::new());
    assert_eq!(back[2].1.len(), 300);
    // A changed byte fails its CRC.
    let mut bad = z.clone();
    let pos = z.windows(5).position(|w| w == b"hello").unwrap();
    bad[pos] = b'j';
    assert!(zip::read(&bad).is_none());
    assert!(zip::read(b"not a zip").is_none());
}

// --------------------------------------------------------------------- bundle

const STAMP: Stamp = Stamp {
    date: (2026, 11, 2),
    time: (9, 5, 0),
};

#[test]
fn the_bundle_holds_exactly_what_the_preview_shows_and_no_canary() {
    let dir = tempfile::tempdir().unwrap();
    let log = RotatingLog::open_with(dir.path(), Limits::default(), T0).unwrap();
    log.write_line(
        &line(at(T0), Level::Info, "start", &[("pid_tree", V::N(1))]),
        T0,
    )
    .unwrap();
    // Lines that should never be written, as if a bug had let them through (T-S10's
    // canaries): a user name, a path, an id and an alias inside the detail.
    let canary = r#"{"timestamp":"2026-11-02T10:00:00Z","level":"INFO","fields":{"event":"bug","detail":"{\"p\":\"C:\\\\Users\\\\CANARY_USER\\\\x\",\"d\":\"0192f1c2-7d3e-7a10-8b55-3c4d5e6f7a8b\",\"a\":\"CANARY_ALIAS_7f3a\"}"}}"#;
    log.write_line(canary, T0 + 1).unwrap();
    log.write_line("free text CANARY_ALIAS_7f3a 987654", T0 + 2)
        .unwrap();
    log.write_line(
        &line(
            at(T0 + 3),
            Level::Info,
            "resources",
            &[("handles", V::N(9))],
        ),
        T0 + 3,
    )
    .unwrap();
    let b = bundle::build(
        dir.path(),
        &[
            ("Version", "0.1.0".into()),
            ("Folder", "C:\\Users\\CANARY_USER".into()),
        ],
        &["CANARY_USER", "CANARY_ALIAS_7f3a"],
        STAMP,
    );
    let p = &b.preview;
    assert_eq!(p.suggested_name, "HealthierYou-diagnostics-2026-11-02.zip");
    assert_eq!(p.kept_lines, 3);
    assert_eq!(p.dropped_lines, 1);
    assert_eq!(p.events.get("resources"), Some(&1));
    assert_eq!(p.events.get("bug"), Some(&1));
    let files = zip::read(&b.zip).unwrap();
    assert_eq!(p.zip_bytes, b.zip.len());
    assert_eq!(files.len(), p.entries.len());
    for ((name, data), e) in files.iter().zip(&p.entries) {
        assert_eq!(name, &e.name);
        assert_eq!(
            data,
            e.text.as_bytes(),
            "{name}: the zip differs from the preview"
        );
        assert_eq!(e.bytes, data.len());
        let text = String::from_utf8_lossy(data);
        for c in [
            "CANARY_USER",
            "CANARY_ALIAS_7f3a",
            "987654",
            "0192f1c2",
            "Users",
        ] {
            assert!(!text.contains(c), "{name} holds {c}: {text}");
        }
    }
    assert!(p.entries[0].text.contains("Version: 0.1.0"));
}

#[test]
fn the_bundle_keeps_the_newest_lines_within_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        rotate_bytes: 1024 * 1024,
        ..Limits::default()
    };
    let log = RotatingLog::open_with(dir.path(), limits, T0).unwrap();
    let mut n = 0u64;
    let mut written = 0;
    while written < MAX_LOG_BYTES + 200_000 {
        let l = line(at(T0 + n), Level::Info, "tick", &[("n", V::N(n))]);
        written += l.len() + 1;
        log.write_line(&l, T0 + n).unwrap();
        n += 1;
    }
    let b = bundle::build(dir.path(), &[], &[], STAMP);
    let p = &b.preview;
    assert!(p.older_lines_left_out > 0);
    assert_eq!(
        u64::try_from(p.kept_lines + p.older_lines_left_out).unwrap(),
        n
    );
    let text = &p.entries[1].text;
    assert!(text.len() <= MAX_LOG_BYTES);
    let last = text.lines().last().unwrap();
    assert_eq!(detail(last)["n"], n - 1);
}
