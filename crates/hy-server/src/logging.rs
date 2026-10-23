//! Logging without health data (V1-5 contract 11).
//!
//! Every log line goes through [`log`], whose fields are a closed set of typed values
//! ([`V`]): random ids, seq numbers, counts, HTTP statuses, the error enum, endpoint
//! names, latencies, 8-hex-character fingerprint prefixes and /24 network prefixes.
//! There is no variant for free text, so a payload, alias, label, token, slip, passphrase,
//! cookie or path cannot be logged by mistake. Request bodies are never logged. The
//! subscriber passes only this crate's events, so a dependency cannot log a request
//! either. The panic hook logs only the source location, never the panic message.
//!
//! The canary test (T-S10) runs the full pairing and admin flow with planted values and
//! searches every log line, the audit log, the admin pages and panic output for them.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use hy_core::protocol::wire::WireError;
use hy_core::protocol::{hex, prefix8};
use serde_json::{Map, Value};

/// The tracing target of every server event.
pub const TARGET: &str = "hy_server";

/// An allowlisted log value.
#[derive(Debug, Clone, Copy)]
pub enum V<'a> {
    /// A random user or device id.
    Id(&'a [u8; 16]),
    /// A seq number, count, size or latency.
    N(u64),
    /// An HTTP status.
    Status(u16),
    /// A wire error.
    Error(WireError),
    /// A fixed name chosen in code (endpoint, reason, outcome).
    Name(&'static str),
    /// The first 8 hex characters of a fingerprint or key.
    Prefix(&'a [u8; 32]),
    /// The /24 (IPv4) or /48 (IPv6) network of a source address.
    Net(IpAddr),
}

/// The /24 (IPv4) or /48 (IPv6) network of an address, the only form of an address that
/// may be logged or audited.
#[must_use]
pub fn net_prefix(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.0/24", o[0], o[1], o[2])
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2])
        }
    }
}

fn render(v: V<'_>) -> Value {
    match v {
        V::Id(id) => Value::String(hex(id)),
        V::N(n) => Value::from(n),
        V::Status(s) => Value::from(s),
        V::Error(e) => Value::String(e.as_str().to_owned()),
        V::Name(n) => Value::String(n.to_owned()),
        V::Prefix(p) => Value::String(prefix8(p)),
        V::Net(ip) => Value::String(net_prefix(ip)),
    }
}

/// Log one event with allowlisted fields.
pub fn log(event: &'static str, fields: &[(&'static str, V<'_>)]) {
    let mut map = Map::new();
    for (k, v) in fields {
        map.insert((*k).to_owned(), render(*v));
    }
    let detail = Value::Object(map).to_string();
    tracing::info!(target: TARGET, event, detail = %detail);
}

/// A writer that appends JSON lines to `server.log`, rotating at 10 MB and keeping
/// rotated files for 7 days (architecture 10).
#[derive(Debug)]
pub struct RotatingFile {
    dir: PathBuf,
    file: Mutex<File>,
}

const ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const KEEP_DAYS: u64 = 7;

impl RotatingFile {
    /// Open `dir/server.log` for appending.
    ///
    /// # Errors
    /// An I/O error if the folder or file cannot be opened.
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("server.log"))?;
        Ok(Self {
            dir: dir.to_owned(),
            file: Mutex::new(file),
        })
    }

    fn rotate_if_needed(&self, file: &mut File) -> io::Result<()> {
        if file.metadata()?.len() < ROTATE_BYTES {
            return Ok(());
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        fs::rename(
            self.dir.join("server.log"),
            self.dir.join(format!("server.log.{stamp}")),
        )?;
        *file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("server.log"))?;
        let cutoff = stamp.saturating_sub(KEEP_DAYS * 86_400);
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let name = entry.file_name();
            let Some(old) = name
                .to_str()
                .and_then(|n| n.strip_prefix("server.log."))
                .and_then(|n| n.parse::<u64>().ok())
            else {
                continue;
            };
            if old < cutoff {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
}

impl Write for &RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut f = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        self.rotate_if_needed(&mut f)?;
        f.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush()
    }
}

/// An in-memory log sink, for tests (the canary test reads it back).
#[derive(Debug, Clone, Default)]
pub struct MemorySink(Arc<Mutex<Vec<u8>>>);

impl MemorySink {
    /// Everything written so far.
    #[must_use]
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

impl Write for MemorySink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Install the global subscriber: JSON lines from this crate only, to `writer`.
/// Returns false if a subscriber was already installed.
pub fn install<W>(writer: W) -> bool
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    use tracing_subscriber::layer::SubscriberExt;
    let filter =
        tracing_subscriber::filter::Targets::new().with_target(TARGET, tracing::Level::INFO);
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_target(false)
        .with_writer(writer);
    let subscriber = tracing_subscriber::registry().with(filter).with(layer);
    tracing::subscriber::set_global_default(subscriber).is_ok()
}

/// Replace the panic hook with one that logs only the location (contract 11: "a panic
/// hook strips payloads").
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let (file, line) = info
            .location()
            .map_or(("unknown", 0), |l| (l.file(), l.line()));
        // The file is a source path inside this repository, never a user's path.
        tracing::error!(target: TARGET, event = "panic", file, line);
        eprintln!("hy-server: internal error at {file}:{line}");
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks_are_truncated() {
        assert_eq!(
            net_prefix("192.168.7.42".parse().unwrap()),
            "192.168.7.0/24"
        );
        assert_eq!(
            net_prefix("fe80::1:2:3:4".parse().unwrap()),
            "fe80:0:0::/48"
        );
    }

    #[test]
    fn values_render_as_ids_numbers_and_prefixes_only() {
        assert_eq!(render(V::Id(&[0xAB; 16])), Value::String("ab".repeat(16)));
        assert_eq!(
            render(V::Prefix(&[0xCD; 32])),
            Value::String("cdcdcdcd".into())
        );
        assert_eq!(
            render(V::Error(WireError::SlipInvalid)),
            Value::String("slip_invalid".into())
        );
    }
}
