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
const CALL_WAIT: Duration = Duration::from_secs(15);

type Job = Box<dyn FnOnce(&mut CoachLink) + Send>;

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
    stopped: AtomicBool,
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
            Msg::Job(f) => f(&mut link),
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
        let (tx, rx) = mpsc::channel::<Msg>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), Error>>();
        let pet: Arc<dyn PetShell> = Arc::new(shell.handle());
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
        let runner = Arc::new(Self {
            tx,
            locked: AtomicBool::new(false),
            asleep: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        });
        if pump {
            let weak = Arc::downgrade(&runner);
            let _ = std::thread::Builder::new()
                .name("hy-signals".into())
                .spawn(move || pump_signals(&weak));
        }
        Ok(runner)
    }

    fn send(&self, msg: Msg) {
        if !self.stopped.load(Ordering::SeqCst) {
            let _ = self.tx.send(msg);
        }
    }

    /// Apply an engine input on the engine thread.
    pub fn input(&self, input: Input) {
        self.send(Msg::Input(input));
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

    /// Run `f` on the engine thread and wait for its result (the self-test).
    pub fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut CoachLink) -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = mpsc::channel();
        self.send(Msg::Job(Box::new(move |link| {
            let _ = tx.send(f(link));
        })));
        rx.recv_timeout(CALL_WAIT).ok()
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
