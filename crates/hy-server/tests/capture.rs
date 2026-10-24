//! T-S23 on the wire (V1-5 contract 14.2; slice V1-7): the CI stand-in for the network
//! capture an office PC runs with pktmon (`scripts/windows/Test-NetworkCapture.ps1`).
//!
//! The companion's real pairing and sync code talks to the real server through a relay
//! that records every byte in both directions, one record per TCP read, for every
//! connection. The phases follow T-S23: before pairing; pending; paired with every
//! category off; hydration on; hydration off again. The test then checks, on the bytes
//! as they crossed the wire:
//!
//! - every connection is TLS 1.3 only: the ClientHello offers no other version, no early
//!   data and no session resumption (no pre-shared key), and asks for ALPN `hy/1`; the
//!   ServerHello selects TLS 1.3; after the hellos only encrypted application records
//!   (and the one-byte compatibility ChangeCipherSpec) cross in either direction;
//! - no planted value or protocol text appears in clear: not the alias, the PC label,
//!   the token, the slip secret, a water amount, an HTTP request line, a path, a header
//!   name or a JSON key;
//! - per phase (from the server's own request log, which names endpoints only): nothing
//!   before pairing; only pairing and status while pending; no upload with every
//!   category off; uploads of hydration rows that are exactly their schema's canonical
//!   form while it is on; no further upload or row after it is switched off.
//!
//! With `HY_CAPTURE_PCAP=<file>` set, the recorded connections are also written as a
//! pcapng file (the real TLS bytes in synthesised Ethernet, IPv4 and TCP headers, server
//! 192.168.50.10:8443, companion 192.168.50.21), which CI feeds to the capture script's
//! analyser to show it passes genuine companion traffic. What only the physical capture
//! proves (the real network path, Windows and WebView2 making no other connection, the
//! process attribution) is listed in `docs/V1_Acceptance.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use common::{Harness, opts};
use hy_core::clock::Clock;
use hy_core::link::flow::{SyncOutcome, sync_once};
use hy_core::protocol::slip::{Slip, encode_secret};
use hy_core::protocol::{ALPN, b64, share};
use hy_core::schema::Category;
use hy_server::logging::{self, MemorySink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ALIAS: &str = "CANARY_ALIAS_7f3a";
const LABEL: &str = "CANARY_LABEL_5b1e";
/// A water amount no other field produces (two entries: 1723 and 1723).
const WATER_ML: u32 = 1723;
const WATER_TOTAL: &str = "3446";

/// Which way a chunk went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
}

#[derive(Debug, Clone, Default)]
struct Conn {
    chunks: Vec<(Dir, u128, Vec<u8>)>,
}

impl Conn {
    fn stream(&self, d: Dir) -> Vec<u8> {
        self.chunks
            .iter()
            .filter(|c| c.0 == d)
            .flat_map(|c| c.2.iter().copied())
            .collect()
    }
}

type Conns = Arc<Mutex<Vec<Conn>>>;

/// A relay in front of `target` recording both directions of every connection.
async fn recording_relay(target: std::net::SocketAddr) -> (u16, Conns) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let conns: Conns = Arc::new(Mutex::new(Vec::new()));
    let conns2 = conns.clone();
    let t0 = Instant::now();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let idx = {
                let mut c = conns2.lock().unwrap();
                c.push(Conn::default());
                c.len() - 1
            };
            let conns = conns2.clone();
            tokio::spawn(async move {
                let Ok(server) = TcpStream::connect(target).await else {
                    return;
                };
                let (cr, cw) = client.into_split();
                let (sr, sw) = server.into_split();
                let pump = |mut r: tokio::net::tcp::OwnedReadHalf,
                            mut w: tokio::net::tcp::OwnedWriteHalf,
                            d: Dir,
                            conns: Conns| async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    while let Ok(n) = r.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        conns.lock().unwrap()[idx].chunks.push((
                            d,
                            t0.elapsed().as_micros(),
                            buf[..n].to_vec(),
                        ));
                        if w.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    let _ = w.shutdown().await;
                };
                tokio::join!(
                    pump(cr, sw, Dir::Up, conns.clone()),
                    pump(sr, cw, Dir::Down, conns)
                );
            });
        }
    });
    (port, conns)
}

/// TLS records in a byte stream: (content type, body).
fn records(bytes: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 5 <= bytes.len() {
        let len = usize::from(u16::from_be_bytes([bytes[at + 3], bytes[at + 4]]));
        let end = (at + 5 + len).min(bytes.len());
        out.push((bytes[at], &bytes[at + 5..end]));
        at += 5 + len;
    }
    out
}

/// The extensions of a ClientHello or ServerHello handshake message: (type, body).
fn hello_extensions(msg: &[u8], client: bool) -> Vec<(u16, &[u8])> {
    // type(1) length(3) legacy_version(2) random(32) session_id(1 + n)
    let mut at = 4 + 2 + 32;
    let sid = usize::from(msg[at]);
    at += 1 + sid;
    if client {
        let suites = usize::from(u16::from_be_bytes([msg[at], msg[at + 1]]));
        at += 2 + suites;
        let comp = usize::from(msg[at]);
        at += 1 + comp;
    } else {
        at += 2 + 1;
    }
    let total = usize::from(u16::from_be_bytes([msg[at], msg[at + 1]]));
    at += 2;
    let end = at + total;
    let mut out = Vec::new();
    while at + 4 <= end {
        let t = u16::from_be_bytes([msg[at], msg[at + 1]]);
        let len = usize::from(u16::from_be_bytes([msg[at + 2], msg[at + 3]]));
        out.push((t, &msg[at + 4..at + 4 + len]));
        at += 4 + len;
    }
    out
}

const EXT_ALPN: u16 = 16;
const EXT_PRE_SHARED_KEY: u16 = 41;
const EXT_EARLY_DATA: u16 = 42;
const EXT_SUPPORTED_VERSIONS: u16 = 43;
const TLS13: [u8; 2] = [0x03, 0x04];

/// Check one connection's TLS profile; returns a description of the first problem.
fn tls13_only(c: &Conn) -> Result<(), String> {
    let up = c.stream(Dir::Up);
    let down = c.stream(Dir::Down);
    let up_recs = records(&up);
    let down_recs = records(&down);
    let (t, ch) = up_recs.first().ok_or("no client records")?;
    if *t != 22 || ch.first() != Some(&1) {
        return Err(format!(
            "the first client record is not a ClientHello ({t})"
        ));
    }
    let exts = hello_extensions(ch, true);
    let versions = exts
        .iter()
        .find(|e| e.0 == EXT_SUPPORTED_VERSIONS)
        .ok_or("no supported_versions")?
        .1;
    if versions.get(1..) != Some(&TLS13[..]) || versions.first() != Some(&2) {
        return Err(format!("offered versions {versions:02x?}"));
    }
    for (bad, name) in [
        (EXT_EARLY_DATA, "early data"),
        (EXT_PRE_SHARED_KEY, "resumption"),
    ] {
        if exts.iter().any(|e| e.0 == bad) {
            return Err(format!("the ClientHello offers {name}"));
        }
    }
    let alpn = exts.iter().find(|e| e.0 == EXT_ALPN).ok_or("no ALPN")?.1;
    if !alpn.windows(ALPN.len()).any(|w| w == ALPN) {
        return Err("ALPN hy/1 not offered".into());
    }
    let (t, sh) = down_recs.first().ok_or("no server records")?;
    if *t != 22 || sh.first() != Some(&2) {
        return Err(format!(
            "the first server record is not a ServerHello ({t})"
        ));
    }
    let chosen = hello_extensions(sh, false)
        .into_iter()
        .find(|e| e.0 == EXT_SUPPORTED_VERSIONS)
        .ok_or("the server chose no version")?
        .1;
    if chosen != TLS13 {
        return Err(format!("the server chose {chosen:02x?}"));
    }
    for (side, recs) in [("client", &up_recs), ("server", &down_recs)] {
        for (t, body) in recs.iter().skip(1) {
            match t {
                20 if body.len() == 1 => {}
                23 => {}
                _ => {
                    return Err(format!(
                        "{side} sent a clear record of type {t} after the hello"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Requests per endpoint in the server's log text.
fn endpoint_count(log: &str, endpoint: &str) -> usize {
    // The detail is JSON inside the line's JSON, so its quotes may be escaped.
    let want = format!("\"endpoint\":\"{endpoint}\"");
    log.lines()
        .map(|l| l.replace('\\', ""))
        .filter(|l| l.contains("\"request\"") && l.contains(&want))
        .count()
}

// ------------------------------------------------------------------ pcapng output

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn block(kind: u32, body: &[u8]) -> Vec<u8> {
    let pad = (4 - body.len() % 4) % 4;
    let total = u32::try_from(12 + body.len() + pad).unwrap();
    let mut b = Vec::new();
    b.extend_from_slice(&le32(kind));
    b.extend_from_slice(&le32(total));
    b.extend_from_slice(body);
    b.extend(std::iter::repeat_n(0, pad));
    b.extend_from_slice(&le32(total));
    b
}

fn ipv4_checksum(h: &[u8]) -> u16 {
    let mut sum: u32 = h
        .chunks(2)
        .map(|c| u32::from(u16::from_be_bytes([c[0], c[1]])))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !u16::try_from(sum).unwrap()
}

/// One Ethernet + IPv4 + TCP frame carrying `payload`.
fn frame(
    (src, sport): ([u8; 4], u16),
    (dst, dport): ([u8; 4], u16),
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&[0x02, 0, 0, 0, 0, dst[3]]);
    f.extend_from_slice(&[0x02, 0, 0, 0, 0, src[3]]);
    f.extend_from_slice(&[0x08, 0x00]);
    let total = u16::try_from(20 + 20 + payload.len()).unwrap();
    let mut ip = vec![0x45, 0];
    ip.extend_from_slice(&total.to_be_bytes());
    ip.extend_from_slice(&[0, 0, 0x40, 0, 64, 6, 0, 0]);
    ip.extend_from_slice(&src);
    ip.extend_from_slice(&dst);
    let c = ipv4_checksum(&ip);
    ip[10..12].copy_from_slice(&c.to_be_bytes());
    f.extend_from_slice(&ip);
    f.extend_from_slice(&sport.to_be_bytes());
    f.extend_from_slice(&dport.to_be_bytes());
    f.extend_from_slice(&seq.to_be_bytes());
    f.extend_from_slice(&ack.to_be_bytes());
    f.extend_from_slice(&[0x50, flags, 0xff, 0xff, 0, 0, 0, 0]);
    f.extend_from_slice(payload);
    f
}

/// The recorded connections as a pcapng file.
fn pcapng(conns: &[Conn]) -> Vec<u8> {
    const SERVER: [u8; 4] = [192, 168, 50, 10];
    const PC: [u8; 4] = [192, 168, 50, 21];
    let mut out = Vec::new();
    let mut shb = Vec::new();
    shb.extend_from_slice(&le32(0x1A2B_3C4D));
    shb.extend_from_slice(&le16(1));
    shb.extend_from_slice(&le16(0));
    shb.extend_from_slice(&u64::MAX.to_le_bytes());
    out.extend(block(0x0A0D_0D0A, &shb));
    let mut idb = Vec::new();
    idb.extend_from_slice(&le16(1)); // Ethernet
    idb.extend_from_slice(&le16(0));
    idb.extend_from_slice(&le32(65535));
    out.extend(block(1, &idb));
    let base: u64 = 1_793_000_000_000_000; // a fixed start time, in microseconds
    let mut packets: Vec<(u128, Vec<u8>)> = Vec::new();
    for (i, c) in conns.iter().enumerate() {
        let sport = 50_000 + u16::try_from(i).unwrap();
        let (mut up_seq, mut down_seq) = (1_000u32, 9_000_000u32);
        let first = c.chunks.first().map_or(0, |x| x.1);
        packets.push((
            first,
            frame((PC, sport), (SERVER, 8443), up_seq - 1, 0, 0x02, &[]),
        ));
        packets.push((
            first,
            frame((SERVER, 8443), (PC, sport), down_seq - 1, up_seq, 0x12, &[]),
        ));
        for (d, t, bytes) in &c.chunks {
            for seg in bytes.chunks(1400) {
                let n = u32::try_from(seg.len()).unwrap();
                let f = match d {
                    Dir::Up => {
                        let f = frame((PC, sport), (SERVER, 8443), up_seq, down_seq, 0x18, seg);
                        up_seq = up_seq.wrapping_add(n);
                        f
                    }
                    Dir::Down => {
                        let f = frame((SERVER, 8443), (PC, sport), down_seq, up_seq, 0x18, seg);
                        down_seq = down_seq.wrapping_add(n);
                        f
                    }
                };
                packets.push((*t, f));
            }
        }
    }
    packets.sort_by_key(|p| p.0);
    for (t, f) in packets {
        let ts = base + u64::try_from(t).unwrap();
        let mut epb = Vec::new();
        epb.extend_from_slice(&le32(0));
        epb.extend_from_slice(&le32(u32::try_from(ts >> 32).unwrap()));
        epb.extend_from_slice(&le32(u32::try_from(ts & 0xffff_ffff).unwrap()));
        let len = u32::try_from(f.len()).unwrap();
        epb.extend_from_slice(&le32(len));
        epb.extend_from_slice(&le32(len));
        epb.extend_from_slice(&f);
        epb.extend(std::iter::repeat_n(0, (4 - f.len() % 4) % 4));
        out.extend(block(6, &epb));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s23_only_tls13_with_nothing_in_clear_and_nothing_for_a_category_that_is_off() {
    let sink = MemorySink::default();
    let writer = sink.clone();
    assert!(logging::install(move || writer.clone()));
    let h = Harness::start().await;
    let (port, conns) = recording_relay(h.api).await;
    let u = h.user(ALIAS);
    let slip = h.slip(&u);
    let secret = encode_secret(slip.secret.expose()).expose().clone();
    let moved = Slip::new("127.0.0.1", port, h.pin(), *slip.secret.expose()).unwrap();
    let log_now = || sink.contents();

    // Before pairing: an unpaired companion makes no connection at all.
    let clock: Arc<dyn hy_core::clock::Clock> = h.clock.clone();
    let unpaired = Mutex::new(hy_core::store::Store::open_in_memory(clock).unwrap());
    let today = unpaired.lock().unwrap().today().unwrap();
    let out = sync_once(
        &unpaired,
        &hy_core::secret::EphemeralVault::new(),
        opts(),
        today,
    )
    .await
    .unwrap();
    assert_eq!(out, SyncOutcome::NotPaired);
    assert_eq!(conns.lock().unwrap().len(), 0, "nothing before pairing");

    // Pending: pairing and status only.
    let pc = h.pending_pc_with(&moved, LABEL).await;
    assert_eq!(pc.sync().await.unwrap(), SyncOutcome::Pending);
    let log = log_now();
    assert_eq!(endpoint_count(&log, "pair"), 1);
    for e in ["history", "control", "upload", "receipt", "manifest"] {
        assert_eq!(endpoint_count(&log, e), 0, "{e} while pending");
    }

    // Confirmed, every category off: operational data only, no upload, no row.
    h.confirm(&pc.dev);
    pc.water(h.clock.now(), WATER_ML);
    assert!(matches!(
        pc.synced().await,
        SyncOutcome::Synced { uploaded: 0, .. }
    ));
    assert_eq!(
        endpoint_count(&log_now(), "upload"),
        0,
        "no upload with every category off"
    );
    assert_eq!(h.count("summaries"), 0);

    // Hydration on: rows of exactly the schema's canonical form, hydration only.
    pc.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    pc.synced().await;
    pc.water(h.clock.now(), WATER_ML);
    let out = pc.synced().await;
    assert!(
        matches!(out, SyncOutcome::Synced { uploaded: 1.., .. }),
        "{out:?}"
    );
    let rows: Vec<(String, String)> = h
        .server
        .db()
        .prepare("SELECT category, payload FROM summaries")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!rows.is_empty());
    for (category, payload) in &rows {
        assert_eq!(category, "hydration", "only the category that is on");
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(
            share::validate(Category::Hydration, &v).unwrap(),
            v,
            "schema fields only"
        );
        assert_eq!(v["ml"], 2 * WATER_ML);
    }
    let uploads_on = endpoint_count(&log_now(), "upload");

    // Hydration off: nothing more for it, even with new water logged.
    pc.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    pc.synced().await;
    pc.water(h.clock.now(), WATER_ML);
    pc.synced().await;
    pc.synced().await;
    assert_eq!(
        endpoint_count(&log_now(), "upload"),
        uploads_on,
        "no upload after the disable"
    );
    let after: i64 = h
        .server
        .db()
        .query_row(
            "SELECT coalesce(sum(json_extract(payload, '$.ml')), 0) FROM summaries",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        after,
        i64::from(2 * WATER_ML),
        "the third entry never reached the server"
    );

    // On the wire: every connection TLS 1.3 only, nothing in clear.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let conns = conns.lock().unwrap().clone();
    assert!(conns.len() >= 6, "{} connections", conns.len());
    let token = b64(pc.dev.token.expose());
    let needles: Vec<&str> = vec![
        ALIAS,
        LABEL,
        secret.as_str(),
        token.as_str(),
        WATER_TOTAL,
        "1723",
        "HTTP/1.1",
        "/v1/",
        "uthorization",
        "content-type",
        "hydration",
        "\"ml\"",
    ];
    for (i, c) in conns.iter().enumerate() {
        tls13_only(c).unwrap_or_else(|e| panic!("connection {i}: {e}"));
        for d in [Dir::Up, Dir::Down] {
            let bytes = c.stream(d);
            for n in &needles {
                assert!(
                    !bytes
                        .windows(n.len())
                        .any(|w| w.eq_ignore_ascii_case(n.as_bytes())),
                    "connection {i} {d:?} carries {n:?} in clear"
                );
            }
        }
    }
    if let Some(path) = std::env::var_os("HY_CAPTURE_PCAP") {
        std::fs::write(path, pcapng(&conns)).unwrap();
    }
}

#[test]
fn the_checks_catch_a_tls12_offer_early_data_and_a_clear_record() {
    // A minimal ClientHello builder: versions and extra extensions as given.
    fn client_hello(versions: &[u8], extra: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut exts = Vec::new();
        let mut sv = vec![u8::try_from(versions.len()).unwrap()];
        sv.extend_from_slice(versions);
        let mut alpn = vec![0, 5, 4];
        alpn.extend_from_slice(ALPN);
        for (t, b) in [(EXT_SUPPORTED_VERSIONS, sv), (EXT_ALPN, alpn)]
            .into_iter()
            .chain(extra.iter().cloned())
        {
            exts.extend_from_slice(&t.to_be_bytes());
            exts.extend_from_slice(&u16::try_from(b.len()).unwrap().to_be_bytes());
            exts.extend_from_slice(&b);
        }
        let mut body = vec![3, 3];
        body.extend_from_slice(&[0; 32]);
        body.extend_from_slice(&[0, 0, 2, 0x13, 0x01, 1, 0]);
        body.extend_from_slice(&u16::try_from(exts.len()).unwrap().to_be_bytes());
        body.extend_from_slice(&exts);
        let mut msg = vec![1, 0];
        msg.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        msg.extend_from_slice(&body);
        let mut rec = vec![22, 3, 1];
        rec.extend_from_slice(&u16::try_from(msg.len()).unwrap().to_be_bytes());
        rec.extend_from_slice(&msg);
        rec
    }
    fn server_hello(version: [u8; 2]) -> Vec<u8> {
        let mut ext = Vec::new();
        ext.extend_from_slice(&EXT_SUPPORTED_VERSIONS.to_be_bytes());
        ext.extend_from_slice(&[0, 2]);
        ext.extend_from_slice(&version);
        let mut body = vec![3, 3];
        body.extend_from_slice(&[0; 32]);
        body.extend_from_slice(&[0, 0x13, 0x01, 0]);
        body.extend_from_slice(&u16::try_from(ext.len()).unwrap().to_be_bytes());
        body.extend_from_slice(&ext);
        let mut msg = vec![2, 0];
        msg.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        msg.extend_from_slice(&body);
        let mut rec = vec![22, 3, 3];
        rec.extend_from_slice(&u16::try_from(msg.len()).unwrap().to_be_bytes());
        rec.extend_from_slice(&msg);
        rec
    }
    let conn = |up: Vec<u8>, down: Vec<u8>| Conn {
        chunks: vec![(Dir::Up, 0, up), (Dir::Down, 1, down)],
    };
    let app = [23u8, 3, 3, 0, 3, 1, 2, 3];
    let good = conn(
        [client_hello(&TLS13, &[]), app.to_vec()].concat(),
        [server_hello(TLS13), app.to_vec()].concat(),
    );
    assert_eq!(tls13_only(&good), Ok(()));
    let tls12 = conn(client_hello(&[3, 4, 3, 3], &[]), server_hello(TLS13));
    assert!(tls13_only(&tls12).unwrap_err().contains("offered versions"));
    let early = conn(
        client_hello(&TLS13, &[(EXT_EARLY_DATA, vec![])]),
        server_hello(TLS13),
    );
    assert!(tls13_only(&early).unwrap_err().contains("early data"));
    let chose12 = conn(client_hello(&TLS13, &[]), server_hello([3, 3]));
    assert!(tls13_only(&chose12).unwrap_err().contains("chose"));
    let clear = conn(
        [client_hello(&TLS13, &[]), vec![22, 3, 3, 0, 1, 0]].concat(),
        server_hello(TLS13),
    );
    assert!(tls13_only(&clear).unwrap_err().contains("clear record"));
}
