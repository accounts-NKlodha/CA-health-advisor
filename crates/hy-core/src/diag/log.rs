//! The companion's rotating JSON-line log (architecture 10: "rotated at 7 days or 10 MB").
//!
//! Lines are appended to `companion.log`. Before a line is written the file is rotated
//! when it has reached [`Limits::rotate_bytes`] or its first line is older than
//! [`Limits::rotate_age_s`]: it is renamed `companion.log.<Unix seconds>` and a new file is
//! started. Rotated files older than [`Limits::keep_s`] are deleted, and never more than
//! [`Limits::keep_files`] are kept, so the folder stays under about
//! `(keep_files + 1) * rotate_bytes` however busy the app is. The age is read from the
//! first line's own timestamp, not the file's creation time, which Windows carries over
//! to a new file of the same name made within seconds of a rename ("tunnelling").
//!
//! Writing never fails the caller in a way that matters: the companion ignores the
//! result, and the panic hook uses [`RotatingLog::try_write_line`], which gives up rather
//! than wait for a lock the panicking thread may hold.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, TryLockError};

/// The log's file name in its folder.
pub const LOG_NAME: &str = "companion.log";
/// Rotate at this size (architecture 10).
pub const ROTATE_BYTES: u64 = 10 * 1024 * 1024;
/// Rotate when the first line is this old (architecture 10).
pub const ROTATE_AGE_S: u64 = 7 * 86_400;
/// Delete rotated files this old.
pub const KEEP_S: u64 = 7 * 86_400;
/// Keep at most this many rotated files.
pub const KEEP_FILES: usize = 4;

/// When to rotate and what to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Rotate before a write once the file has this many bytes.
    pub rotate_bytes: u64,
    /// Rotate before a write once the first line is this many seconds old.
    pub rotate_age_s: u64,
    /// Delete rotated files older than this many seconds.
    pub keep_s: u64,
    /// Keep at most this many rotated files (the newest).
    pub keep_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            rotate_bytes: ROTATE_BYTES,
            rotate_age_s: ROTATE_AGE_S,
            keep_s: KEEP_S,
            keep_files: KEEP_FILES,
        }
    }
}

#[derive(Debug)]
struct Inner {
    file: Option<File>,
    size: u64,
    /// Unix seconds of the current file's first line (`None` while it is empty).
    started: Option<u64>,
}

/// The rotating log file.
#[derive(Debug)]
pub struct RotatingLog {
    dir: PathBuf,
    limits: Limits,
    inner: Mutex<Inner>,
}

/// The Unix seconds of a log line's `timestamp`, if it has one.
#[must_use]
pub fn line_time(line: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let at = chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?).ok()?;
    u64::try_from(at.timestamp()).ok()
}

fn first_line_time(path: &Path) -> Option<u64> {
    let f = File::open(path).ok()?;
    let mut first = String::new();
    // A first line longer than this is not one of ours.
    BufReader::new(f.take(64 * 1024))
        .read_line(&mut first)
        .ok()?;
    line_time(first.trim_end())
}

fn modified_s(path: &Path) -> Option<u64> {
    let m = fs::metadata(path).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// The stamp of a rotated file's name (`companion.log.<stamp>`).
fn rotated_stamp(name: &str) -> Option<u64> {
    name.strip_prefix(LOG_NAME)?
        .strip_prefix('.')?
        .parse::<u64>()
        .ok()
}

/// The log files in `dir`, oldest first: the rotated files by their stamp, then the
/// current file. Only files that exist are listed.
#[must_use]
pub fn files(dir: &Path) -> Vec<PathBuf> {
    let mut rotated: Vec<(u64, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let stamp = rotated_stamp(e.file_name().to_str()?)?;
            Some((stamp, e.path()))
        })
        .collect();
    rotated.sort();
    let mut out: Vec<PathBuf> = rotated.into_iter().map(|(_, p)| p).collect();
    let current = dir.join(LOG_NAME);
    if current.is_file() {
        out.push(current);
    }
    out
}

impl RotatingLog {
    /// Open (creating if needed) `dir/companion.log` with the default limits.
    ///
    /// # Errors
    /// The folder or file cannot be created or opened.
    pub fn open(dir: &Path, now_s: u64) -> io::Result<Self> {
        Self::open_with(dir, Limits::default(), now_s)
    }

    /// Open with the given limits, deleting rotated files past keeping.
    ///
    /// # Errors
    /// The folder or file cannot be created or opened.
    pub fn open_with(dir: &Path, limits: Limits, now_s: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(LOG_NAME);
        let file = open_append(&path)?;
        let size = file.metadata()?.len();
        let started = (size > 0).then(|| {
            first_line_time(&path)
                .or_else(|| modified_s(&path))
                .unwrap_or(now_s)
        });
        let log = Self {
            dir: dir.to_owned(),
            limits,
            inner: Mutex::new(Inner {
                file: Some(file),
                size,
                started,
            }),
        };
        log.prune(now_s);
        Ok(log)
    }

    /// The log's folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Append `line` (one JSON object, no newline) at `now_s`, rotating first if due.
    ///
    /// # Errors
    /// The file cannot be rotated or written.
    pub fn write_line(&self, line: &str, now_s: u64) -> io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        self.write_locked(&mut inner, line, now_s)
    }

    /// As [`write_line`](Self::write_line), but gives up at once if another thread holds
    /// the log (the panic hook: the panicking thread may be the one holding it).
    pub fn try_write_line(&self, line: &str, now_s: u64) -> bool {
        let mut inner = match self.inner.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return false,
        };
        self.write_locked(&mut inner, line, now_s).is_ok()
    }

    fn write_locked(&self, inner: &mut Inner, line: &str, now_s: u64) -> io::Result<()> {
        let too_big = inner.size >= self.limits.rotate_bytes;
        let too_old = inner
            .started
            .is_some_and(|s| now_s.saturating_sub(s) >= self.limits.rotate_age_s);
        if inner.size > 0 && (too_big || too_old) {
            self.rotate(inner, now_s)?;
        }
        if inner.file.is_none() {
            inner.file = Some(open_append(&self.dir.join(LOG_NAME))?);
        }
        let mut buf = Vec::with_capacity(line.len() + 1);
        // A line never spans two lines of the file.
        buf.extend(
            line.bytes()
                .map(|b| if b == b'\n' || b == b'\r' { b' ' } else { b }),
        );
        buf.push(b'\n');
        if let Some(f) = inner.file.as_mut() {
            f.write_all(&buf)?;
            f.flush()?;
        }
        if inner.size == 0 {
            inner.started = Some(now_s);
        }
        inner.size += buf.len() as u64;
        Ok(())
    }

    fn rotate(&self, inner: &mut Inner, now_s: u64) -> io::Result<()> {
        // Close the file before renaming it.
        inner.file = None;
        let mut stamp = now_s;
        while self.dir.join(format!("{LOG_NAME}.{stamp}")).exists() {
            stamp += 1;
        }
        fs::rename(
            self.dir.join(LOG_NAME),
            self.dir.join(format!("{LOG_NAME}.{stamp}")),
        )?;
        inner.file = Some(open_append(&self.dir.join(LOG_NAME))?);
        inner.size = 0;
        inner.started = None;
        self.prune(now_s);
        Ok(())
    }

    /// Delete rotated files past keeping: older than `keep_s` by their stamp, then the
    /// oldest beyond `keep_files`.
    fn prune(&self, now_s: u64) {
        let cutoff = now_s.saturating_sub(self.limits.keep_s);
        let mut rotated: Vec<(u64, PathBuf)> = Vec::new();
        for e in fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let Some(stamp) = e.file_name().to_str().and_then(rotated_stamp) else {
                continue;
            };
            if stamp < cutoff {
                let _ = fs::remove_file(e.path());
            } else {
                rotated.push((stamp, e.path()));
            }
        }
        rotated.sort();
        let extra = rotated.len().saturating_sub(self.limits.keep_files);
        for (_, p) in rotated.into_iter().take(extra) {
            let _ = fs::remove_file(p);
        }
    }
}
