//! The coaching engine in the companion (slices V1-1 and V1-2 joined).
//!
//! The engine runs on its own thread, which owns the [`CoachLink`] (the engine bound to
//! the store and the pet shell). Everything reaches it as a message, in order:
//!
//! - the signal pump's 5-second idle tick (`GetLastInputInfo`) and presentation changes
//!   (sampled every second, confirmed by two equal samples, ignored while away);
//! - lock, unlock, sleep and wake from the pet window's subclass (see
//!   [`crate::win32::Signal`]), forwarded by the shell;
//! - the user's answers: the shell's sink routes each event on the UI thread without
//!   blocking ([`LinkSink`]) and sends the resulting command here.
//!
//! The link calls the shell through its `PetShell` handle, which posts every call to the
//! UI thread, so the engine thread never waits for the UI and the UI never waits for the
//! store, except for the final save on exit ([`CoachRunner::shutdown`]).
//!
//! The rules (what to show, what an answer means, what counts as a presentation) are in
//! `hy_pet::link`; this module only moves messages and reads the Windows values.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use hy_core::coach::Input;
use hy_core::error::Error;
use hy_core::store::Store;
use hy_pet::link::signals::{PRESENCE_EVERY, PresenceTracker, TICK_EVERY};
use hy_pet::link::{CoachLink, Command, LinkSink};
use hy_pet::shell::PetShell;

use crate::shell::Shell;

/// A system event from the pet window, independent of the Windows message that carried it.
/// Only the Windows build receives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum SystemSignal {
    /// The session was locked.
    Lock,
    /// The session was unlocked.
    Unlock,
    /// The PC is going to sleep.
    Suspend,
    /// The PC woke up.
    Resume,
    /// Windows is signing out or shutting down.
    EndSession,
}

/// How long exit waits for the final save.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
/// How long [`CoachRunner::call`] waits for the engine thread.
pub const CALL_WAIT: Duration = Duration::from_secs(15);

/// Whether the engine thread goes on after a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    /// The job replaced the data: the engine stops at once, without stepping or saving
    /// again (its state belongs to the data that was replaced).
    Stop,
}

type Job = Box<dyn FnOnce(&mut CoachLink) -> Flow + Send>;

/// Why a job on the engine thread gave no answer. Only [`CallError::NotRunning`] means
/// the job certainly did not run; the others leave its outcome unknown to the caller, so
/// a page must not be told "nothing was changed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    /// The engine is stopped (or never started): the job was not queued.
    NotRunning,
    /// No answer within the wait: the job is queued or running and may still finish.
    Pending,
    /// The engine thread ended before answering (it stopped after an import, or failed).
    Lost,
}

enum Msg {
    Input(Input),
    Command(Command),
    Job(Job),
    Shutdown(Sender<()>),
}

/// The engine's thread and the signal pump.
#[derive(Debug)]
pub struct CoachRunner {
    tx: Sender<Msg>,
    locked: AtomicBool,
    asleep: AtomicBool,
    /// Set by [`CoachRunner::shutdown`], or by the engine thread itself when a job
    /// replaced the data ([`CoachRunner::replace_and_stop`]); nothing is queued after.
    stopped: Arc<AtomicBool>,
}

fn report(what: &str, r: Result<impl Sized, Error>) {
    if let Err(e) = r {
        eprintln!("HealthierYou: {what} failed: {e}");
    }
}

fn run(mut link: CoachLink, rx: &Receiver<Msg>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Input(i) => report("coaching step", link.input(i)),
            Msg::Command(c) => report("coaching command", link.command(c)),
            Msg::Job(f) => {
                if f(&mut link) == Flow::Stop {
                    // No flush: the engine's state went with the replaced data.
                    return;
                }
            }
            Msg::Shutdown(ack) => {
                report("saving coaching state", link.flush());
                let _ = ack.send(());
                return;
            }
        }
    }
    report("saving coaching state", link.flush());
}

impl CoachRunner {
    /// Open the engine over `store` on its own thread, connect it to the shell (its
    /// first view is drawn at once) and, with `pump`, start the signal pump. The
    /// self-test runs without the pump and drives a simulated clock itself.
    ///
    /// # Errors
    /// The engine could not be opened (an unreadable store or invalid settings).
    pub fn start(shell: &Arc<Shell>, store: Store, pump: bool) -> Result<Arc<Self>, Error> {
        let runner = Self::launch(store, Arc::new(shell.handle()))?;
        if pump {
            let weak = Arc::downgrade(&runner);
            let _ = std::thread::Builder::new()
                .name("hy-signals".into())
                .spawn(move || pump_signals(&weak));
        }
        Ok(runner)
    }

    /// Open the engine over `store` on its own thread, connected to `pet`.
    fn launch(store: Store, pet: Arc<dyn PetShell>) -> Result<Arc<Self>, Error> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), Error>>();
        let to_engine = tx.clone();
        std::thread::Builder::new()
            .name("hy-coach".into())
            .spawn(move || match CoachLink::open(store, Arc::clone(&pet)) {
                Ok((link, _)) => {
                    let sink = LinkSink::new(link.router(), move |c| {
                        let _ = to_engine.send(Msg::Command(c));
                    });
                    pet.set_event_sink(Arc::new(sink));
                    let _ = ready_tx.send(Ok(()));
                    run(link, &rx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(Error::Io)?;
        ready_rx
            .recv()
            .map_err(|_| Error::Invalid("the coaching thread stopped while opening".into()))??;
        Ok(Arc::new(Self {
            tx,
            locked: AtomicBool::new(false),
            asleep: AtomicBool::new(false),
            stopped: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// Queue `msg` unless the engine is stopped; whether it was queued.
    fn send(&self, msg: Msg) -> bool {
        !self.stopped.load(Ordering::SeqCst) && self.tx.send(msg).is_ok()
    }

    /// Apply an engine input on the engine thread.
    pub fn input(&self, input: Input) {
        let _ = self.send(Msg::Input(input));
    }

    /// Whether the session is locked or the PC asleep.
    fn away(&self) -> bool {
        self.locked.load(Ordering::SeqCst) || self.asleep.load(Ordering::SeqCst)
    }

    /// A system event from the pet window. Called inside the window procedure: it only
    /// sends messages, except at the end of the Windows session, when it waits briefly for
    /// the final save.
    pub fn system(&self, s: SystemSignal) {
        match s {
            SystemSignal::Lock => {
                self.locked.store(true, Ordering::SeqCst);
                self.input(Input::Lock);
            }
            SystemSignal::Unlock => {
                self.locked.store(false, Ordering::SeqCst);
                self.input(Input::Unlock);
            }
            SystemSignal::Suspend => {
                self.asleep.store(true, Ordering::SeqCst);
                self.input(Input::Suspend);
            }
            SystemSignal::Resume => {
                self.asleep.store(false, Ordering::SeqCst);
                self.input(Input::Resume);
            }
            SystemSignal::EndSession => {
                self.shutdown(SHUTDOWN_WAIT);
            }
        }
    }

    /// Queue `f` on the engine thread without waiting for it (the posture cue asks the
    /// engine and hears back through a post to the shell). Whether it was queued.
    pub fn submit(&self, f: impl FnOnce(&mut CoachLink) + Send + 'static) -> bool {
        self.send(Msg::Job(Box::new(move |link| {
            f(link);
            Flow::Continue
        })))
    }

    /// Run `f` on the engine thread and wait for its result, for reads and the
    /// self-test: `None` whatever the reason.
    pub fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut CoachLink) -> T + Send + 'static,
    ) -> Option<T> {
        self.try_call(f).ok()
    }

    /// Run `f` on the engine thread and wait up to [`CALL_WAIT`] for its result. A
    /// timeout does not cancel `f`: it is [`CallError::Pending`], and a change `f` makes
    /// may still be saved after the caller stopped waiting.
    ///
    /// # Errors
    /// See [`CallError`].
    pub fn try_call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut CoachLink) -> T + Send + 'static,
    ) -> Result<T, CallError> {
        self.job(move |link| (f(link), Flow::Continue), CALL_WAIT)
    }

    /// Replace the store's data on the engine thread and complete the replacement
    /// there, whether or not the caller is still waiting:
    ///
    /// - `f` runs on the engine's own connection;
    /// - if it succeeds, the engine stops at once (no further step and no save: its
    ///   state belongs to the replaced data; nothing more is queued), then `after` runs
    ///   on the engine thread with the result (the shell restarts the app);
    /// - if it fails, nothing was changed, the engine goes on, and `after` runs with the
    ///   error.
    ///
    /// The caller waits at most `wait`. [`CallError::Pending`] then means the outcome is
    /// not known yet: `after` still reports it, so a late success is never left without
    /// the engine stop and restart, and never reported as "nothing was changed".
    ///
    /// # Errors
    /// See [`CallError`]; `Ok` carries what `f` returned.
    pub fn replace_and_stop<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut CoachLink) -> Result<T, Error> + Send + 'static,
        after: impl FnOnce(&Result<T, Error>) + Send + 'static,
        wait: Duration,
    ) -> Result<Result<T, Error>, CallError> {
        let stopped = Arc::clone(&self.stopped);
        self.job(
            move |link| {
                let out = f(link);
                let flow = if out.is_ok() {
                    // Refuse anything queued from now on, before anyone hears of success.
                    stopped.store(true, Ordering::SeqCst);
                    Flow::Stop
                } else {
                    Flow::Continue
                };
                after(&out);
                (out, flow)
            },
            wait,
        )
    }

    fn job<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut CoachLink) -> (T, Flow) + Send + 'static,
        wait: Duration,
    ) -> Result<T, CallError> {
        let (tx, rx) = mpsc::channel();
        let queued = self.send(Msg::Job(Box::new(move |link| {
            let (out, flow) = f(link);
            let _ = tx.send(out);
            flow
        })));
        if !queued {
            return Err(CallError::NotRunning);
        }
        rx.recv_timeout(wait).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => CallError::Pending,
            mpsc::RecvTimeoutError::Disconnected => CallError::Lost,
        })
    }

    /// Whether the engine is stopped (shut down, or stopped by an import).
    #[cfg(test)]
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Save, stop the engine thread and the pump. Waits at most `wait` for the save.
    /// Returns whether the save finished in time. Later calls do nothing.
    pub fn shutdown(&self, wait: Duration) -> bool {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return false;
        }
        let (ack, done) = mpsc::channel();
        if self.tx.send(Msg::Shutdown(ack)).is_err() {
            return false;
        }
        done.recv_timeout(wait).is_ok()
    }
}

/// One presentation sample: Windows' notification state or a full-screen foreground app.
#[cfg(windows)]
fn sample_presenting() -> bool {
    hy_pet::link::signals::presenting(
        crate::win32::notification_state(),
        crate::win32::foreground_window().as_ref(),
    )
}

#[cfg(not(windows))]
const fn sample_presenting() -> bool {
    false
}

#[cfg(windows)]
fn idle_time() -> Duration {
    crate::win32::idle_time()
}

#[cfg(not(windows))]
const fn idle_time() -> Duration {
    Duration::ZERO
}

/// The signal pump: presentation every second, the idle tick every 5 seconds.
fn pump_signals(runner: &Weak<CoachRunner>) {
    let mut presence = PresenceTracker::new();
    let mut was_away = false;
    let mut last_tick: Option<Instant> = None;
    loop {
        let Some(r) = runner.upgrade() else { break };
        if r.stopped.load(Ordering::SeqCst) {
            break;
        }
        let away = r.away();
        if away != was_away {
            presence.set_away(away);
            was_away = away;
        }
        if let Some(on) = presence.observe(sample_presenting()) {
            r.input(Input::PresentationChanged(on));
        }
        if last_tick.is_none_or(|t| t.elapsed() >= TICK_EVERY) {
            last_tick = Some(Instant::now());
            r.input(Input::Tick { idle: idle_time() });
        }
        drop(r);
        std::thread::sleep(PRESENCE_EVERY);
    }
}

#[cfg(test)]
mod tests {
    //! The import's completion on the engine thread: a worker held past the caller's
    //! wait (Codex review of #14). Deterministic: the job waits on a gate the test opens
    //! only after the caller has given up.

    use std::sync::Mutex;

    use hy_core::backup::KdfParams;
    use hy_core::clock::SystemClock;
    use hy_core::coach::Servings;
    use hy_pet::shell::{Bubble, BubbleOutcome, HideReason, MenuAction, ShellEventSink};
    use hy_pet::state::PetState;

    use super::*;

    const FAST: KdfParams = KdfParams {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };
    const SHORT: Duration = Duration::from_millis(50);
    const LONG: Duration = Duration::from_secs(30);

    struct NoPet;

    impl PetShell for NoPet {
        fn set_state(&self, _: PetState, _: Option<String>) {}
        fn show_bubble(&self, _: Bubble) -> BubbleOutcome {
            BubbleOutcome::Shown
        }
        fn close_bubble(&self, _: &str) {}
        fn hide(&self, _: HideReason) {}
        fn unhide(&self, _: HideReason) {}
        fn is_visible(&self) -> bool {
            true
        }
        fn set_menu_actions(&self, _: Vec<MenuAction>) {}
        fn set_event_sink(&self, _: Arc<dyn ShellEventSink>) {}
    }

    struct Env {
        dir: std::path::PathBuf,
        path: std::path::PathBuf,
        runner: Arc<CoachRunner>,
    }

    impl Drop for Env {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// An engine over a store on disk (as in the app), and a backup whose only change
    /// from a new store is its serving buttons, 200 and 750 ml.
    fn env(name: &str) -> (Env, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("hy-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.sqlite3");
        let clock = Arc::new(SystemClock::new());
        let store = Store::open(&path, clock.clone()).unwrap();
        let runner = CoachRunner::launch(store, Arc::new(NoPet)).unwrap();
        // The engine has saved its state once.
        assert!(runner.try_call(|l| l.flush().is_ok()).unwrap());
        let source = Store::open_in_memory(clock).unwrap();
        source
            .set_water_servings(&Servings::new(vec![200, 750]).unwrap())
            .unwrap();
        let (file, _) = source.export_backup_with("long enough", FAST).unwrap();
        (Env { dir, path, runner }, file)
    }

    fn on_disk(path: &std::path::Path) -> Store {
        Store::open(path, Arc::new(SystemClock::new())).unwrap()
    }

    type Seen = Arc<Mutex<Vec<Result<usize, String>>>>;

    struct Held {
        answer: Result<Result<usize, Error>, CallError>,
        /// Lets the held job go on.
        open: Sender<()>,
        /// Signalled when the engine thread reported the outcome.
        done: Receiver<()>,
        seen: Seen,
    }

    /// Start an import whose job waits for the gate before replacing the data (already
    /// open with `opened`); `after` records what the engine thread reports.
    fn held_import(
        e: &Env,
        file: Vec<u8>,
        password: &'static str,
        wait: Duration,
        opened: bool,
    ) -> Held {
        let (open, gate) = mpsc::channel::<()>();
        if opened {
            open.send(()).unwrap();
        }
        let (done_tx, done) = mpsc::channel::<()>();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let answer = e.runner.replace_and_stop(
            move |link| {
                let _ = gate.recv_timeout(LONG);
                link.replace_data(|s| s.import_backup(&file, password).map(|x| x.events))
            },
            move |out| {
                record
                    .lock()
                    .unwrap()
                    .push(out.as_ref().map(|n| *n).map_err(ToString::to_string));
                let _ = done_tx.send(());
            },
            wait,
        );
        Held {
            answer,
            open,
            done,
            seen,
        }
    }

    #[test]
    fn a_late_import_is_pending_then_completed_by_the_engine_thread() {
        let (e, file) = env("late-ok");
        let Held {
            answer,
            open,
            done,
            seen,
        } = held_import(&e, file, "long enough", SHORT, false);
        // The caller gave up: the outcome is not known, nothing is claimed.
        assert!(matches!(answer, Err(CallError::Pending)), "{answer:?}");
        assert!(!e.runner.is_stopped());
        assert_eq!(
            on_disk(&e.path).water_servings().unwrap(),
            Servings::default()
        );
        // The worker goes on and commits the replacement after the caller stopped waiting.
        open.send(()).unwrap();
        done.recv_timeout(LONG).unwrap();
        // The commit was reported, and the engine was stopped by the engine thread.
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(seen.lock().unwrap()[0].is_ok());
        assert!(e.runner.is_stopped());
        assert_eq!(
            e.runner.try_call(|l| l.flush().is_ok()),
            Err(CallError::NotRunning)
        );
        e.runner.input(Input::Refresh); // refused quietly
        let after = on_disk(&e.path);
        assert_eq!(after.water_servings().unwrap().ml(), &[200, 750]);
        // The old engine never saved its state over the imported data.
        assert!(after.coach_state().unwrap().is_none());
    }

    #[test]
    fn a_late_refused_import_reports_it_and_the_engine_goes_on() {
        let (e, file) = env("late-refused");
        let Held {
            answer,
            open,
            done,
            seen,
        } = held_import(&e, file, "the wrong one", SHORT, false);
        assert!(matches!(answer, Err(CallError::Pending)), "{answer:?}");
        open.send(()).unwrap();
        done.recv_timeout(LONG).unwrap();
        assert!(seen.lock().unwrap()[0].is_err());
        assert!(!e.runner.is_stopped());
        assert_eq!(e.runner.try_call(|l| l.flush().is_ok()), Ok(true));
        let after = on_disk(&e.path);
        assert_eq!(after.water_servings().unwrap(), Servings::default());
        assert!(after.coach_state().unwrap().is_some());
    }

    #[test]
    fn an_import_within_the_wait_answers_and_stops_the_engine() {
        let (e, file) = env("in-time");
        let Held {
            answer, done, seen, ..
        } = held_import(&e, file, "long enough", LONG, true);
        assert!(matches!(answer, Ok(Ok(_))), "{answer:?}");
        done.recv_timeout(LONG).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(e.runner.is_stopped());
        // A job queued behind is refused, never run on the replaced data.
        assert_eq!(e.runner.try_call(|_| ()), Err(CallError::NotRunning));
        assert_eq!(on_disk(&e.path).water_servings().unwrap().ml(), &[200, 750]);
    }
}
