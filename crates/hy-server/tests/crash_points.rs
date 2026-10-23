//! Control-log crash points and server backups (slice V1-5b; architecture 11 T-J1 to
//! T-J6; contract 7.3 and 10). Crashes are simulated in process with injected faults
//! followed by a real restart of the server on the same folders and mirror.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Duration;
use common::{Harness, Pc};
use hy_core::clock::Clock;
use hy_core::link::flow::SyncOutcome;
use hy_core::schema::{Category, PolicyOpState};
use hy_server::mirror::MirrorFault;
use hy_server::recovery::Startup;
use hy_server::server::Faults;

async fn paired(h: &Harness) -> ([u8; 16], Pc) {
    let u = h.user("u1");
    let a = h.pc(&u, "PC-A").await;
    a.synced().await;
    (u, a)
}

fn entries_of_kind(h: &Harness, user: &[u8; 16], kind: u8) -> i64 {
    h.server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM control_log WHERE user_id = ?1 AND kind = ?2",
            rusqlite::params![user.as_slice(), kind],
            |r| r.get(0),
        )
        .unwrap()
}

fn mirror_lines(h: &Harness) -> usize {
    std::fs::read_to_string(&h.mirror).unwrap().lines().count()
}

/// The enable went through exactly once and the PC settled it.
fn settled_once(h: &Harness, user: &[u8; 16], a: &Pc, op: uuid::Uuid) {
    assert_eq!(entries_of_kind(h, user, 1), 1, "exactly one policy entry");
    assert!(
        h.server
            .policy_of(user, Category::Hydration)
            .unwrap()
            .enabled
    );
    let op = a.with(|s| s.policy_op(op).unwrap().unwrap());
    assert_eq!(op.state, PolicyOpState::Accepted);
    assert!(a.with(|s| s.sharing_policy(Category::Hydration).unwrap().enabled));
    let (tail, hwm) = h.tail_and_hwm(user);
    assert_eq!(tail, hwm, "T-J6: the high-waters are equal");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j1_t_j6_a_crash_mid_transaction_leaves_nothing_and_the_retry_applies_once() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    let before = h.tail_and_hwm(&u);
    let lines = mirror_lines(&h);
    let op = a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    h.server.inject_faults(Faults {
        fail_before_commit: true,
        ..Faults::default()
    });
    assert!(a.sync().await.is_err());
    assert_eq!(h.tail_and_hwm(&u), before, "nothing visible");
    assert_eq!(mirror_lines(&h), lines);
    assert!(!h.server.policy_of(&u, Category::Hydration).unwrap().enabled);
    let h = h.restart().await;
    assert_eq!(h.startup, Startup::Opened);
    a.synced().await;
    a.synced().await;
    settled_once(&h, &u, &a, op);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j2_t_j6_a_crash_after_commit_is_repaired_at_startup_and_the_retry_is_idempotent() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    let op = a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    h.server.inject_faults(Faults {
        crash_after_commit: true,
        ..Faults::default()
    });
    assert!(a.sync().await.is_err());
    let (tail, hwm) = h.tail_and_hwm(&u);
    assert_eq!(tail, hwm + 1, "committed, not mirrored");
    let h = h.restart().await;
    assert_eq!(
        h.startup,
        Startup::Repaired {
            reappended: 1,
            torn: false
        }
    );
    let (tail_after, hwm_after) = h.tail_and_hwm(&u);
    assert_eq!((tail_after, hwm_after), (tail, tail));
    // The PC never saw an answer, so it sends the same op again: the original result.
    a.synced().await;
    a.synced().await;
    settled_once(&h, &u, &a, op);
    assert_eq!(h.tail_and_hwm(&u).0, tail, "no duplicate entry");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j3_t_j6_a_crash_after_the_mirror_append_returns_the_same_seq_on_retry() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    let op = a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    h.server.inject_faults(Faults {
        stop_after_mirror: true,
        ..Faults::default()
    });
    assert!(a.sync().await.is_err());
    let lines = mirror_lines(&h);
    let (tail, _) = h.tail_and_hwm(&u);
    // The retry, without a restart: same seq, no duplicate, nothing appended twice.
    a.synced().await;
    a.synced().await;
    settled_once(&h, &u, &a, op);
    assert_eq!(h.tail_and_hwm(&u).0, tail);
    assert_eq!(mirror_lines(&h), lines);
    // And across a restart the two still agree.
    let h = h.restart().await;
    assert_eq!(h.startup, Startup::Opened);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j3_variant_a_restart_between_the_mirror_append_and_the_high_water() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    let op = a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    h.server.inject_faults(Faults {
        stop_after_mirror: true,
        ..Faults::default()
    });
    assert!(a.sync().await.is_err());
    let h = h.restart().await;
    assert!(
        matches!(
            h.startup,
            Startup::Opened | Startup::Repaired { reappended: 0, .. }
        ),
        "{:?}",
        h.startup
    );
    a.synced().await;
    a.synced().await;
    settled_once(&h, &u, &a, op);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j4_t_j6_power_loss_during_the_mirror_write_truncates_and_re_appends() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    let op = a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    h.server.set_mirror_fault(Some(MirrorFault::TornWrite));
    assert!(a.sync().await.is_err());
    let text = std::fs::read(&h.mirror).unwrap();
    assert_ne!(text.last(), Some(&b'\n'), "a torn final line");
    let h = h.restart().await;
    assert_eq!(
        h.startup,
        Startup::Repaired {
            reappended: 1,
            torn: true
        }
    );
    let (tail, hwm) = h.tail_and_hwm(&u);
    assert_eq!(tail, hwm);
    a.synced().await;
    a.synced().await;
    settled_once(&h, &u, &a, op);
    let h = h.restart().await;
    assert_eq!(h.startup, Startup::Opened, "the repaired mirror verifies");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j5_a_corrupt_mirror_entry_makes_a_restore_fail_closed() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    a.synced().await;
    let file = h.backup();
    a.with(|s| s.disable_sharing(Category::Hydration).unwrap());
    a.synced().await;
    let h = h
        .restore_after(&file, |m| {
            let mut b = std::fs::read(m).unwrap();
            let i = b.len() - 20;
            b[i] = if b[i] == b'0' { b'1' } else { b'0' };
            std::fs::write(m, b).unwrap();
        })
        .await;
    assert!(
        matches!(h.startup, Startup::GlobalDiscard { .. }),
        "{:?}",
        h.startup
    );
    assert!(!h.server.policy_of(&u, Category::Hydration).unwrap().enabled);
    assert!(a.sync().await.is_err(), "every device was revoked");
    assert_eq!(h.audits("global_discard"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_j6_uploads_wait_until_the_mirror_has_the_entries() {
    let h = Harness::start().await;
    let (u, a) = paired(&h).await;
    a.with(|s| {
        s.request_enable(Category::Hydration, "v1-5-sharing-1")
            .unwrap()
    });
    // The mirror share goes offline: the control batch cannot be recorded at all.
    h.server.set_mirror_fault(Some(MirrorFault::Unavailable));
    assert!(a.sync().await.is_err());
    assert!(!h.server.policy_of(&u, Category::Hydration).unwrap().enabled);
    h.server.set_mirror_fault(None);
    assert!(matches!(a.synced().await, SyncOutcome::Synced { .. }));
    let (tail, hwm) = h.tail_and_hwm(&u);
    assert_eq!(tail, hwm);
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_are_encrypted_bound_to_this_server_and_pruned() {
    use hy_server::backup;
    let h = Harness::start().await;
    let (_u, a) = paired(&h).await;
    let _ = a;
    let dir = h.dir.parent().unwrap().join("backups");
    let key = Harness::backup_key();
    // A first backup is due at once (none yet); then not again until 13:00 IST or 24 h.
    let first = backup::run_if_due(&h.server, &dir, &key).unwrap();
    assert!(first.is_some());
    assert!(backup::run_if_due(&h.server, &dir, &key).unwrap().is_none());
    let file = first.unwrap().path;
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.starts_with(backup::MAGIC));
    assert!(
        !bytes.windows(6).any(|w| w == b"SQLite"),
        "the database is never written in the clear"
    );
    // Another key (another machine) cannot open it; a damaged file is refused.
    let other = backup::BackupKey::from_bytes([9u8; 32]);
    assert!(matches!(
        backup::open(&file, &other),
        Err(backup::BackupError::Invalid)
    ));
    let mut damaged = bytes.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    let bad = dir.join("damaged.hysbak");
    std::fs::write(&bad, damaged).unwrap();
    assert!(matches!(
        backup::open(&bad, &key),
        Err(backup::BackupError::Invalid)
    ));
    std::fs::remove_file(&bad).unwrap();
    assert!(backup::open(&file, &key).is_ok());
    // Forty daily runs: 14 daily and 2 weekly copies at most, none older than 30 days.
    for _ in 0..40 {
        // Just over a day: the catch-up rule makes each run due.
        h.clock.advance(Duration::days(1) + Duration::minutes(1));
        assert!(backup::run_if_due(&h.server, &dir, &key).unwrap().is_some());
    }
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(files.iter().all(|f| f.ends_with(".hysbak")), "{files:?}");
    assert!(
        files.len() <= backup::KEEP_DAILY + backup::KEEP_WEEKLY,
        "{}",
        files.len()
    );
    assert!(files.len() >= backup::KEEP_DAILY);
    let oldest: i64 = h
        .server
        .db()
        .query_row("SELECT min(created_at) FROM backups", [], |r| r.get(0))
        .unwrap();
    assert!(h.clock.now().timestamp_millis() - oldest <= backup::MAX_AGE_MS);
    let rows: i64 = h
        .server
        .db()
        .query_row("SELECT COUNT(*) FROM backups", [], |r| r.get(0))
        .unwrap();
    assert_eq!(usize::try_from(rows).unwrap(), files.len());
}
