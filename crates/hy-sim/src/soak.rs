//! `hy-sim soak <real seconds> <simulated days> [<report.json>]` (slice V1-7, architecture
//! 11 "Soak"): the office server under a steady office load for many simulated days, for
//! the Windows CI job. The real server runs in process on a loopback port, as in the
//! week's simulation, with 10 simulated companions (5 users, 2 PCs each). On every
//! simulated workday (Monday to Saturday, 09:30 to 18:30 in Kolkata) each PC logs water
//! once an hour and syncs every 15 minutes, as the companion does; every simulated hour
//! the server runs its maintenance and its daily backup when one is due, as its service
//! loop does. The rounds are spread over the real time given, so the script outside
//! (`scripts/windows/Test-Soak.ps1 -Role Server -Sim`) can sample this process's memory,
//! handles and threads for a growth trend.
//!
//! At the end of every simulated day it checks that the server holds exactly what each PC
//! shared and that the control log and the mirror agree, and samples the database, its
//! write-ahead log, the mirror and the backups. It fails (non-zero exit code) on a wrong
//! row, a write-ahead log that is not checkpointed, more backups than the retention rules
//! keep, database growth that speeds up, or syncs that slow down. It prints and writes
//! only counts, sizes and timings, never data.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use hy_core::clock::{Clock, ManualClock};
use hy_core::schema::{Category, EventPayload, NewEvent};
use hy_server::backup;
use hy_server::keys::UnprotectedForTests;
use serde_json::json;

use super::{Expect, Fail, NOTICE, check, fail, key, p95, pair_pcs, start, stop, sync_all, water};

/// Kolkata has no daylight saving: local time is UTC + 5:30 throughout.
const LOCAL_OFFSET_MIN: i64 = 330;
/// Bounds (no figure in the architecture; generous, so only real growth fails). SQLite
/// checkpoints the write-ahead log at about 4 MiB.
const WAL_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Sync p95 may vary, but never grow past this many times the first workday's (or this
/// floor in milliseconds, for very fast first days).
const P95_GROWTH: u128 = 3;
const P95_FLOOR_MS: u128 = 250;

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

/// One end-of-day sample.
#[derive(Debug, Clone, serde::Serialize)]
struct DaySample {
    day: i64,
    weekday: String,
    syncs: usize,
    sync_p95_ms: u128,
    rows_verified: usize,
    summaries: i64,
    control_entries: i64,
    db_bytes: u64,
    wal_bytes: u64,
    mirror_bytes: u64,
    backups: usize,
    backup_bytes: u64,
}

/// Run the soak in `tmp`: `days` simulated days spread over `real` (no pacing if zero).
/// Returns whether every bound held, the one-line summary and the JSON report; a wrong
/// row or a failed sync is an error at once.
#[allow(clippy::too_many_lines)]
pub async fn run(tmp: &Path, real: Duration, days: i64) -> Result<(bool, String, String), Fail> {
    let dir = tmp.join("data");
    let mirror_dir = tmp.join("mirror");
    let backups = tmp.join("backups");
    for d in [&dir, &mirror_dir, &backups] {
        std::fs::create_dir_all(d).map_err(fail)?;
    }
    let mirror = mirror_dir.join("control.mirror");
    // Monday 2 Nov 2026, 00:00 in Kolkata.
    let start_at = Utc
        .with_ymd_and_hms(2026, 11, 1, 18, 30, 0)
        .single()
        .ok_or("start time")?;
    let clock = std::sync::Arc::new(ManualClock::new(start_at));
    let srv = start(&dir, &mirror, &clock, 0, &UnprotectedForTests).await?;
    let (users, mut pcs) = pair_pcs(&srv.server, &clock).await?;
    for pc in pcs.iter().step_by(super::PCS_PER_USER) {
        pc.with(|s| s.request_enable(Category::Hydration, NOTICE))
            .map_err(fail)?;
    }
    let exp = Expect {
        deleted: Vec::new(),
        off: None,
    };
    let work_start = NaiveTime::from_hms_opt(9, 30, 0).ok_or("time")?;
    let work_end = NaiveTime::from_hms_opt(18, 30, 0).ok_or("time")?;
    let rounds_per_day = 24 * 4;
    let total = days * rounds_per_day;
    let real_start = Instant::now();
    let mut samples: Vec<DaySample> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut day_times = Vec::new();
    for round in 0..total {
        let now = clock.now();
        let local = now.naive_utc() + chrono::Duration::minutes(LOCAL_OFFSET_MIN);
        let workday = local.weekday() != Weekday::Sun;
        let at_work = workday && local.time() >= work_start && local.time() < work_end;
        if at_work && local.minute() == 30 {
            let day =
                (local.date() - (start_at.naive_utc().date() + chrono::Days::new(1))).num_days();
            for (i, pc) in pcs.iter_mut().enumerate() {
                let ml = water(i, day);
                pc.with(|s| {
                    s.append_event(NewEvent::manual(EventPayload::WaterLogged {
                        amount_ml: ml,
                    }))
                })
                .map_err(fail)?;
                *pc.logged.entry(local.date()).or_insert(0) += ml;
            }
        }
        if at_work {
            let (outs, mut t) = sync_all(&pcs).await?;
            if let Some(o) = outs
                .iter()
                .find(|o| !matches!(o, hy_core::link::flow::SyncOutcome::Synced { .. }))
            {
                return Err(fail(format!("a PC did not sync: {o:?}")));
            }
            day_times.append(&mut t);
        }
        if local.minute() == 0 {
            let s = srv.server.clone();
            let (b, k) = (backups.clone(), key());
            tokio::task::spawn_blocking(move || -> Result<(), Fail> {
                s.maintenance().map_err(fail)?;
                backup::run_if_due(&s, &b, &k).map_err(fail)?;
                Ok(())
            })
            .await
            .map_err(fail)??;
        }
        clock.advance(chrono::Duration::minutes(15));
        if (round + 1) % rounds_per_day == 0 {
            // End of a simulated day.
            let rows = check(&srv.server, &pcs, &users, &exp)
                .map_err(|e| format!("day {}: {e}", samples.len()))?;
            let n = |sql: &str| -> Result<i64, Fail> {
                srv.server
                    .db()
                    .query_row(sql, [], |r| r.get(0))
                    .map_err(fail)
            };
            let files: Vec<u64> = std::fs::read_dir(&backups)
                .map_err(fail)?
                .filter_map(Result::ok)
                .map(|e| e.metadata().map_or(0, |m| m.len()))
                .collect();
            let syncs = day_times.len();
            let p = if day_times.is_empty() {
                0
            } else {
                p95(&mut day_times).as_millis()
            };
            day_times.clear();
            samples.push(DaySample {
                day: i64::try_from(samples.len()).unwrap_or(0),
                weekday: local.weekday().to_string(),
                syncs,
                sync_p95_ms: p,
                rows_verified: rows,
                summaries: n("SELECT count(*) FROM summaries")?,
                control_entries: n("SELECT count(*) FROM control_log")?,
                db_bytes: file_len(&dir.join("server.db")),
                wal_bytes: file_len(&dir.join("server.db-wal")),
                mirror_bytes: file_len(&mirror),
                backups: files.len(),
                backup_bytes: files.iter().sum(),
            });
        }
        if !real.is_zero() {
            #[allow(clippy::cast_precision_loss)]
            let due = real.mul_f64((round + 1) as f64 / total as f64);
            let spent = real_start.elapsed();
            if spent < due {
                tokio::time::sleep(due - spent).await;
            }
        }
    }
    stop(srv).await?;

    // Bounds over the whole run.
    let wal_max = samples.iter().map(|s| s.wal_bytes).max().unwrap_or(0);
    if wal_max > WAL_MAX_BYTES {
        failures.push(format!(
            "write-ahead log reached {wal_max} bytes (bound {WAL_MAX_BYTES})"
        ));
    }
    let keep = backup::KEEP_DAILY + backup::KEEP_WEEKLY;
    if let Some(s) = samples.iter().find(|s| s.backups > keep) {
        failures.push(format!(
            "day {}: {} backups kept (retention keeps at most {keep})",
            s.day, s.backups
        ));
    }
    if days >= 2 && samples.iter().skip(1).all(|s| s.backups == 0) {
        failures.push("no daily backup was written".into());
    }
    // Database growth must not speed up: the second half grows at most twice the first
    // half, plus 256 KiB of page-allocation slack.
    if samples.len() >= 4 {
        let mid = samples.len() / 2;
        let first = samples[mid].db_bytes.saturating_sub(samples[0].db_bytes);
        let second = samples[samples.len() - 1]
            .db_bytes
            .saturating_sub(samples[mid].db_bytes);
        if second > 2 * first + 256 * 1024 {
            failures.push(format!(
                "database growth sped up: {first} bytes in the first half, {second} in the second"
            ));
        }
    }
    let worked: Vec<&DaySample> = samples.iter().filter(|s| s.syncs > 0).collect();
    if let (Some(first), Some(last)) = (worked.first(), worked.last()) {
        let bound = (first.sync_p95_ms * P95_GROWTH).max(P95_FLOOR_MS);
        if last.sync_p95_ms > bound {
            failures.push(format!(
                "syncs slowed down: p95 {} ms on the last workday against {} ms on the first (bound {bound} ms)",
                last.sync_p95_ms, first.sync_p95_ms
            ));
        }
    }
    let ok = failures.is_empty();
    let report = json!({
        "ok": ok,
        "simulated_days": days,
        "real_s": real_start.elapsed().as_secs(),
        "pcs": pcs.len(),
        "failures": failures,
        "days": samples,
    });
    let last = samples.last();
    let line = format!(
        "hy-sim soak: {days} simulated days in {} s; {} rows verified on the last day; sync p95 first/last workday {}/{} ms; write-ahead log at most {wal_max} bytes; {} backups kept",
        real_start.elapsed().as_secs(),
        last.map_or(0, |s| s.rows_verified),
        worked.first().map_or(0, |s| s.sync_p95_ms),
        worked.last().map_or(0, |s| s.sync_p95_ms),
        last.map_or(0, |s| s.backups),
    );
    let text = serde_json::to_string_pretty(&report).map_err(fail)?;
    let line = if ok {
        line
    } else {
        format!("{line}; FAILED: {}", failures.join("; "))
    };
    Ok((ok, line, text))
}
