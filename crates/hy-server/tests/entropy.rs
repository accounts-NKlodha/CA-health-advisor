//! The server fails closed when the system random generator fails: no user id, slip,
//! token, device id or passphrase salt is issued from bytes that were not filled.

#![allow(clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{TimeZone, Utc};
use hy_core::clock::{Clock, ManualClock};
use hy_core::protocol::PROTOCOL;
use hy_core::protocol::slip::encode_secret;
use hy_core::protocol::wire::{PairRequest, WireError};
use hy_core::secret::Secret;
use hy_server::keys::{ServerKeys, UnprotectedForTests};
use hy_server::server::{AdminError, Entropy, NoRandom, Refusal};
use hy_server::{Config, Server, db};
use uuid::Uuid;

/// The system generator until `broken` is set, then a failing one.
struct Switchable {
    broken: Arc<AtomicBool>,
    rng: SystemRandom,
}

impl Entropy for Switchable {
    fn fill(&self, out: &mut [u8]) -> Result<(), NoRandom> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(NoRandom);
        }
        self.rng.fill(out).map_err(|_| NoRandom)
    }
}

fn count(server: &Server, table: &str) -> i64 {
    server
        .db()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_failing_random_generator_issues_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {
        host_name: "127.0.0.1".into(),
        argon2: (256, 1, 1),
        ..Config::default()
    };
    let keys = ServerKeys::create(tmp.path(), &UnprotectedForTests, "127.0.0.1", 2026).unwrap();
    let conn = db::open(&tmp.path().join("server.db")).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 10, 23, 6, 0, 0).unwrap(),
    ));
    let broken = Arc::new(AtomicBool::new(false));
    let server = Server::new(conn, keys, clock, config).with_entropy(Box::new(Switchable {
        broken: broken.clone(),
        rng: SystemRandom::new(),
    }));
    let user = server.create_user("u1").unwrap();
    let slip = server.issue_slip(&user).unwrap();
    let req = PairRequest {
        protocol: PROTOCOL,
        secret: encode_secret(slip.secret.expose()).expose().clone(),
        origin_device: Uuid::from_bytes([3; 16]),
        device_label: "PC-1".into(),
        app_version: "0.5.0".into(),
        windows_build: "26100".into(),
    };
    let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 30));
    let (users, slips) = (count(&server, "users"), count(&server, "slips"));

    broken.store(true, Ordering::SeqCst);
    assert_eq!(server.create_user("u2"), Err(AdminError::NoRandom));
    assert_eq!(server.issue_slip(&user).unwrap_err(), AdminError::NoRandom);
    assert_eq!(
        server.set_admin_passphrase(&Secret::new("a long passphrase".to_owned())),
        Err(AdminError::NoRandom)
    );
    assert!(matches!(
        server.redeem(&req, ip),
        Err(Refusal::Wire(WireError::CannotRecord))
    ));
    assert_eq!(count(&server, "users"), users);
    assert_eq!(count(&server, "slips"), slips);
    assert_eq!(count(&server, "devices"), 0, "no token or device id issued");
    assert_eq!(
        count(&server, "admin"),
        0,
        "no passphrase stored with an unfilled salt"
    );
    let used: i64 = server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM slips WHERE used_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(used, 0, "the slip was not spent");

    // Once the generator works again the same slip still pairs.
    broken.store(false, Ordering::SeqCst);
    let (resp, _token) = server.redeem(&req, ip).unwrap();
    assert_eq!(resp.user_id.as_bytes(), &user);
    assert_eq!(count(&server, "devices"), 1);
}
