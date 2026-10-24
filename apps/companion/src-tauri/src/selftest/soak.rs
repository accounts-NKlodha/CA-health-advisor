//! The accelerated soak (slice V1-7, architecture 11 "Soak"), run by the Windows CI job
//! after the self-test: `HY_SELFTEST=<report> HY_SELFTEST_SOAK_S=<real seconds>
//! [HY_SELFTEST_SOAK_SIM_H=<simulated hours>] hy-companion.exe`.
//!
//! The real 8-hour soak runs on an office PC with the real clock
//! (`scripts/windows/Test-Soak.ps1`). A CI runner cannot wait 8 hours, so this phase
//! keeps the installed app running for the given real time while the coaching engine's
//! simulated clock covers several office days (72 simulated hours by default): workdays
//! and evenings, a lunchtime lock, an overnight sleep, and the settings and review
//! windows opened and closed every three simulated hours (each opening makes a new
//! webview and each closing returns it). Every reminder the engine offers is shown on the
//! real bubble window and answered there in turn ("Already did it", "Snooze", "Not now",
//! or left unanswered to collapse), and any other card is closed. The script outside
//! samples the processes' memory, handles, threads and CPU throughout and checks there is
//! no growth trend; this side checks what only the app knows:
//!
//! - the reminder cadence: at least three reminders on every full simulated workday,
//!   never two within the 5-minute calm gap, none while locked or asleep, none outside
//!   work hours (architecture 4.3);
//! - every reminder appeared on the real bubble window inside the work area and every
//!   answer reached the store;
//! - the database and its write-ahead log stay within bounds (no unbounded growth);
//! - the engine answered every step, and the soak kept to its real-time budget.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc, Weekday};
use hy_core::clock::{Clock, ManualClock};
use hy_core::coach::Input;
use hy_core::soak::SoakStats;
use hy_pet::link::present::action;
use serde::Serialize;

use super::{Report, close_window, engine_bubble_on_show, on_main, wait_until, window_open};
use crate::coach::{CoachRunner, SystemSignal};
use crate::shell::Shell;

/// What the soak phase is asked to do.
#[derive(Debug, Clone)]
pub struct SoakPlan {
    /// Real time to spread the soak over.
    pub real: Duration,
    /// Simulated time to cover.
    pub sim: Duration,
    /// The self-test's database (in the temp folder).
    pub store: PathBuf,
}

impl SoakPlan {
    /// The plan from `HY_SELFTEST_SOAK_S` (and `HY_SELFTEST_SOAK_SIM_H`, default 72), if
    /// the soak was asked for.
    pub fn from_env(store: PathBuf) -> Option<Self> {
        let secs: u64 = std::env::var("HY_SELFTEST_SOAK_S").ok()?.parse().ok()?;
        let hours: u64 = std::env::var("HY_SELFTEST_SOAK_SIM_H")
            .ok()
            .and_then(|h| h.parse().ok())
            .unwrap_or(72);
        (secs > 0 && hours > 0).then(|| Self {
            real: Duration::from_secs(secs),
            sim: Duration::from_secs(hours * 3600),
            store,
        })
    }
}

/// The default work window and zone (architecture 4.3; `WorkSchedule::default`). Asia/
/// Kolkata has no daylight saving, so local time is UTC + 5:30 throughout.
const LOCAL_OFFSET_MIN: i64 = 330;
const WORK_START: (u32, u32) = (9, 30);
const WORK_END: (u32, u32) = (18, 30);
/// The engine's calm gap after any answer (architecture 4.3).
const CALM_GAP_MIN: f64 = 5.0;
/// Bounds on the store (no figure in the architecture; generous, so only real growth
/// fails): the write-ahead log is checkpointed by SQLite at about 4 MiB.
const WAL_MAX_BYTES: u64 = 16 * 1024 * 1024;
const DB_GROWTH_PER_SIM_DAY_MAX: u64 = 2 * 1024 * 1024;

/// One store sample.
#[derive(Debug, Clone, Copy, Serialize)]
struct StoreSample {
    sim_hour: u64,
    db_bytes: u64,
    wal_bytes: u64,
}

/// What the soak did, for the report and the script outside.
#[derive(Debug, Default, Serialize)]
pub struct SoakSummary {
    /// Real wall-clock start and end (UTC, RFC 3339): the script samples only between.
    pub started_utc: String,
    pub ended_utc: String,
    real_s: u64,
    sim_hours: u64,
    ticks: u64,
    unanswered_steps: u64,
    reminders_shown: u64,
    reminders_on_real_bubble: u64,
    answers: Vec<(String, u64)>,
    other_cards_closed: u64,
    /// Each offer's simulated local time ("Tue 11:15").
    offers_local: Vec<String>,
    offers_off_hours: u64,
    offers_while_away: u64,
    full_workdays: u64,
    locks: u64,
    sleeps: u64,
    window_cycles: u64,
    window_cycle_failures: u64,
    store: Vec<StoreSample>,
    stats: Option<SoakStats>,
}

fn local(t: DateTime<Utc>) -> chrono::NaiveDateTime {
    t.naive_utc() + chrono::Duration::minutes(LOCAL_OFFSET_MIN)
}

fn is_workday(d: Weekday) -> bool {
    d != Weekday::Sun
}

fn in_work_hours(t: chrono::NaiveDateTime) -> bool {
    let start = NaiveTime::from_hms_opt(WORK_START.0, WORK_START.1, 0).unwrap_or(NaiveTime::MIN);
    let end = NaiveTime::from_hms_opt(WORK_END.0, WORK_END.1, 0).unwrap_or(NaiveTime::MIN);
    is_workday(t.weekday()) && t.time() >= start && t.time() < end
}

fn file_len(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

fn store_sample(plan: &SoakPlan, sim_hour: u64) -> StoreSample {
    let mut wal = plan.store.as_os_str().to_owned();
    wal.push("-wal");
    StoreSample {
        sim_hour,
        db_bytes: file_len(&plan.store),
        wal_bytes: file_len(std::path::Path::new(&wal)),
    }
}

/// The workdays whose whole work window lies between `from` and `to` (local times).
fn full_workdays(from: chrono::NaiveDateTime, to: chrono::NaiveDateTime) -> Vec<chrono::NaiveDate> {
    from.date()
        .iter_days()
        .take_while(|d| *d <= to.date())
        .filter(|d| {
            is_workday(d.weekday())
                && d.and_hms_opt(WORK_START.0, WORK_START.1, 0)
                    .zip(d.and_hms_opt(WORK_END.0, WORK_END.1, 0))
                    .is_some_and(|(f, t)| f >= from && t <= to)
        })
        .collect()
}

/// Open the settings window and the review window, let each page load, close them.
fn cycle_windows(shell: &Arc<Shell>) -> bool {
    let mut ok = true;
    for (label, open) in [
        (
            crate::shell::SCREENS,
            Shell::open_settings as fn(&Arc<Shell>),
        ),
        (crate::shell::REVIEW, Shell::open_review as fn(&Arc<Shell>)),
    ] {
        let _ = on_main(shell, open);
        let opened = wait_until(Duration::from_secs(15), || window_open(shell, label));
        std::thread::sleep(Duration::from_millis(1500));
        ok &= opened && close_window(shell, label);
    }
    ok
}

/// Away state on the simulated clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Away {
    Here,
    Locked(DateTime<Utc>),
    Asleep,
}

/// Run the soak and add its checks to `r`.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    r: &mut Report,
    shell: &Arc<Shell>,
    runner: &Arc<CoachRunner>,
    clock: &Arc<ManualClock>,
    plan: &SoakPlan,
) -> SoakSummary {
    let mut s = SoakSummary {
        started_utc: Utc::now().to_rfc3339(),
        sim_hours: plan.sim.as_secs() / 3600,
        ..SoakSummary::default()
    };
    let tick = chrono::Duration::seconds(5);
    let total = plan.sim.as_secs() / 5;
    let since_ms = clock.now().timestamp_millis();
    let sim_start = clock.now();
    let real_start = Instant::now();
    let mut away = Away::Here;
    let mut away_spans: Vec<(i64, i64)> = Vec::new();
    let mut away_from = 0i64;
    // A reminder is new when its offer is; another card when its bubble is.
    let mut last_offer: Option<String> = None;
    let mut last_card: Option<String> = None;
    let mut turn = 0usize;
    let mut answers: [(&str, u64); 4] = [
        (action::DONE, 0),
        (action::SNOOZE, 0),
        (action::DISMISS, 0),
        ("unanswered", 0),
    ];
    let mut offer_times = Vec::new();
    s.store.push(store_sample(plan, 0));
    let mut i = 0;
    while i < total {
        let now = clock.now();
        let lt = local(now);
        if i % 12 == 0 {
            // Once a simulated minute: the day's schedule.
            let (h, m) = (lt.hour(), lt.minute());
            match (h, m, away) {
                (13, 0, Away::Here) if is_workday(lt.weekday()) => {
                    runner.system(SystemSignal::Lock);
                    away = Away::Locked(now);
                    away_from = now.timestamp_millis();
                    s.locks += 1;
                }
                (13, 30, Away::Locked(_)) => {
                    runner.system(SystemSignal::Unlock);
                    away = Away::Here;
                    away_spans.push((away_from, now.timestamp_millis()));
                }
                (22, 0, Away::Here) => {
                    runner.system(SystemSignal::Suspend);
                    away = Away::Asleep;
                    away_from = now.timestamp_millis();
                    s.sleeps += 1;
                }
                (7, 0, Away::Asleep) => {
                    runner.system(SystemSignal::Resume);
                    away = Away::Here;
                    away_spans.push((away_from, now.timestamp_millis()));
                }
                _ => {}
            }
            if m == 0 {
                let hour = u64::try_from((now - sim_start).num_hours()).unwrap_or(0);
                s.store.push(store_sample(plan, hour));
                if h % 3 == 0 && away == Away::Here {
                    s.window_cycles += 1;
                    if !cycle_windows(shell) {
                        s.window_cycle_failures += 1;
                    }
                }
            }
        }
        clock.advance(tick);
        i += 1;
        if away == Away::Asleep {
            // Asleep: no polling, the clock jumps on (as a real sleep does).
            continue;
        }
        let idle = match away {
            Away::Locked(since) => (clock.now() - since).to_std().unwrap_or_default(),
            _ => Duration::ZERO,
        };
        let step = runner.call(move |l| {
            let _ = l.input(Input::Tick { idle });
            (
                l.shown_bubble_id().map(str::to_owned),
                l.view().reminder().map(|(o, _)| o.to_string()),
            )
        });
        let Some((bubble, offer)) = step else {
            s.unanswered_steps += 1;
            continue;
        };
        match (bubble, offer) {
            (Some(id), Some(offer)) if last_offer.as_deref() != Some(offer.as_str()) => {
                s.reminders_shown += 1;
                let at = clock.now();
                offer_times.push(at);
                let lt = local(at);
                s.offers_local.push(lt.format("%a %H:%M").to_string());
                if !in_work_hours(lt) {
                    s.offers_off_hours += 1;
                }
                if away != Away::Here {
                    s.offers_while_away += 1;
                }
                let id2 = id.clone();
                if wait_until(Duration::from_secs(10), || {
                    engine_bubble_on_show(shell, &id2)
                }) {
                    s.reminders_on_real_bubble += 1;
                }
                let (act, n) = &mut answers[turn % answers.len()];
                *n += 1;
                turn += 1;
                if *act != "unanswered" {
                    let (id3, a) = (id.clone(), *act);
                    let _ = on_main(shell, move |sh| sh.bubble_action(&id3, a));
                    let _ = wait_until(Duration::from_secs(5), || {
                        shell.open_bubble_id().as_deref() != Some(id.as_str())
                    });
                }
                last_offer = Some(offer);
            }
            (Some(id), None) if last_card.as_deref() != Some(id.as_str()) => {
                // Another card (a check-in, an offer to lower intensity after dismissals,
                // a review offer): closed without an answer, as a user might.
                runner.input(Input::CloseBubble);
                s.other_cards_closed += 1;
                last_card = Some(id);
            }
            _ => {}
        }
        if i % 12 == 0 {
            // Keep to the real-time budget: sleep when ahead, give up at twice the budget.
            let due = plan.real.mul_f64(i as f64 / total as f64);
            let spent = real_start.elapsed();
            if spent < due {
                std::thread::sleep(due - spent);
            } else if spent > plan.real * 2 {
                break;
            }
        }
    }
    if away != Away::Here {
        let resume = if away == Away::Asleep {
            SystemSignal::Resume
        } else {
            SystemSignal::Unlock
        };
        runner.system(resume);
        away_spans.push((away_from, clock.now().timestamp_millis()));
    }
    let completed = i >= total;
    s.ticks = i;
    s.real_s = real_start.elapsed().as_secs();
    s.ended_utc = Utc::now().to_rfc3339();
    s.answers = answers.iter().map(|(a, n)| ((*a).to_owned(), *n)).collect();
    let _ = runner.call(|l| l.flush().is_ok());
    s.store.push(store_sample(plan, s.sim_hours));
    let stats = hy_core::soak::read(&plan.store, since_ms).ok();

    // Full workdays: local dates whose whole work window lies inside the soak.
    let days = full_workdays(local(sim_start), local(clock.now()));
    s.full_workdays = days.len() as u64;
    let per_day_ok = days.iter().all(|d| {
        offer_times
            .iter()
            .filter(|t| local(**t).date() == *d)
            .count()
            >= 3
    });

    r.check(
        "soak_completed",
        completed && s.unanswered_steps == 0,
        format!(
            "{} of {total} simulated 5 s steps in {} s (budget {} s); engine steps not answered: {}",
            s.ticks,
            s.real_s,
            plan.real.as_secs(),
            s.unanswered_steps
        ),
    );
    let min_gap = stats.as_ref().and_then(|st| st.min_offer_gap_min);
    r.check(
        "soak_reminder_cadence",
        s.full_workdays >= 1
            && per_day_ok
            && min_gap.is_some_and(|g| g >= CALM_GAP_MIN)
            && s.offers_off_hours == 0
            && s.offers_while_away == 0,
        format!(
            "{} reminders over {} simulated h ({} full workdays, at least 3 each: {per_day_ok}); shortest gap {min_gap:?} min (calm gap {CALM_GAP_MIN}); off hours {}; while locked or asleep {}; {} locks, {} sleeps",
            s.reminders_shown,
            s.sim_hours,
            s.full_workdays,
            s.offers_off_hours,
            s.offers_while_away,
            s.locks,
            s.sleeps
        ),
    );
    let answered: u64 = s
        .answers
        .iter()
        .filter(|(a, _)| a != "unanswered")
        .map(|(_, n)| n)
        .sum();
    let responses = stats
        .as_ref()
        .and_then(|st| st.events_by_type.get("reminder.response").copied())
        .unwrap_or(0);
    let offered = stats
        .as_ref()
        .and_then(|st| st.events_by_type.get("reminder.offered").copied())
        .unwrap_or(0);
    r.check(
        "soak_reminders_on_the_real_bubble_and_answers_stored",
        s.reminders_shown > 0
            && s.reminders_on_real_bubble == s.reminders_shown
            && offered == s.reminders_shown
            && responses >= answered,
        format!(
            "shown {} (on the bubble inside the work area {}); offers stored {offered}; answered {answered}, responses stored {responses}; answers {:?}; other cards closed {}",
            s.reminders_shown, s.reminders_on_real_bubble, s.answers, s.other_cards_closed
        ),
    );
    let first = s.store.first().copied();
    let last = s.store.last().copied();
    let wal_max = s.store.iter().map(|x| x.wal_bytes).max().unwrap_or(0);
    let growth = match (first, last) {
        (Some(f), Some(l)) => (l.db_bytes + l.wal_bytes).saturating_sub(f.db_bytes + f.wal_bytes),
        _ => 0,
    };
    let days = s.sim_hours.div_ceil(24).max(1);
    r.check(
        "soak_store_bounded",
        wal_max <= WAL_MAX_BYTES && growth <= DB_GROWTH_PER_SIM_DAY_MAX * days,
        format!(
            "database {:?} to {:?} bytes, write-ahead log at most {wal_max} (bound {WAL_MAX_BYTES}); growth {growth} bytes over {days} simulated days (bound {} a day)",
            first.map(|x| x.db_bytes),
            last.map(|x| x.db_bytes),
            DB_GROWTH_PER_SIM_DAY_MAX
        ),
    );
    r.check(
        "soak_windows_opened_and_closed",
        s.window_cycles > 0 && s.window_cycle_failures == 0,
        format!(
            "{} cycles of settings and review, {} failed",
            s.window_cycles, s.window_cycle_failures
        ),
    );
    s.stats = stats;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, m: u32) -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, d)
            .and_then(|x| x.and_hms_opt(h, m, 0))
            .unwrap_or_default()
    }

    #[test]
    fn work_hours_are_the_default_window_monday_to_saturday() {
        // 26 Oct 2026 is a Monday; 1 Nov a Sunday.
        assert!(in_work_hours(at(26, 9, 30)));
        assert!(in_work_hours(at(26, 18, 29)));
        assert!(!in_work_hours(at(26, 18, 30)));
        assert!(!in_work_hours(at(26, 9, 29)));
        assert!(in_work_hours(at(31, 12, 0)));
        assert!(!in_work_hours(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 1)
                .and_then(|x| x.and_hms_opt(12, 0, 0))
                .unwrap_or_default()
        ));
    }

    #[test]
    fn only_whole_work_windows_count_as_full_workdays() {
        // Monday 12:00 to Thursday 12:00: Tuesday and Wednesday.
        let days = full_workdays(at(26, 12, 0), at(29, 12, 0));
        let names: Vec<_> = days.iter().map(|d| d.weekday()).collect();
        assert_eq!(names, vec![Weekday::Tue, Weekday::Wed]);
        // Saturday 09:30 to Monday 18:30: Saturday and Monday, never Sunday.
        let days = full_workdays(at(31, 9, 30), at(31, 9, 30) + chrono::Duration::hours(57));
        let names: Vec<_> = days.iter().map(|d| d.weekday()).collect();
        assert_eq!(names, vec![Weekday::Sat, Weekday::Mon]);
    }

    #[test]
    fn local_time_is_kolkata() {
        let t = super::super::sim_start();
        assert_eq!(local(t), at(26, 10, 0));
    }
}
