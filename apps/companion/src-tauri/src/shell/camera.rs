//! The V2 posture check's camera on the shell (slice V2-1; architecture 4.1 and 4.5,
//! wireframes W10, W16 and B11).
//!
//! The rules are `hy_core::camera::Machine`'s; this module carries out its actions on
//! the camera page, a window of its own that exists only while the camera is on (or
//! while the user looks at its consent or status), so its memory goes back when it
//! closes. Camera permission is granted only to that window and only while the machine
//! wants capture ([`Shell::camera_permission`], installed on every window by
//! `crate::windows`), and never saved in the WebView2 profile.
//!
//! Locks: the camera lock is a leaf. It may be taken while `inner` is held (the pet's
//! view reads the status), so nothing here takes `inner` or the store while holding it:
//! the machine's actions are collected under the lock and carried out after.
//!
//! Slice V2-2 adds the posture detector's part: "Calibrate" on the page, the cue (asked
//! of the coaching engine on its thread, its answer posted back), the local
//! `posture.observation` records, and the one request to recalibrate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use hy_core::camera::{
    Action, CameraHold, CameraNotice, CameraSettings, FrameReport, Machine, SETTINGS_KEY, Started,
    Status, Unavailable,
};
use hy_core::coach::Input;
use hy_core::posture::quality::Coverage;
use hy_core::posture::{CueOutcome, Observation, PostureKind, PostureState, cue_outcome_of};
use hy_core::schema::{MeasurementStatus, NewEvent};
use hy_pet::shell::{Bubble, Effect};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, WebviewWindow};

use super::{CAMERA, SCREENS, Shell, lock};
use crate::view::{CameraMenu, CameraPage, PostureView};
use crate::windows;

/// How long a camera notice stays beside the pet.
const NOTICE_FOR: Duration = Duration::from_secs(8);
/// How often the machine's clock ticks (watchdog, stop deadline, retries).
pub const TICK_EVERY: Duration = Duration::from_millis(500);

/// How the page gets its frames to the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePath {
    /// A `MediaStreamTrackProcessor` stream handed to the worker (frames never pass
    /// through the page's main thread).
    Processor,
    /// A hidden video element; the worker asks for each frame as an `ImageBitmap`.
    Video,
}

/// What the camera page reports (besides frames).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageReport {
    /// The camera track and the pose model are ready.
    Started {
        /// Track width, pixels.
        width: u32,
        /// Track height, pixels.
        height: u32,
        /// `navigator.hardwareConcurrency`.
        cores: u32,
        /// Model load time, ms.
        model_ms: f64,
        /// How frames reach the worker.
        path: CapturePath,
    },
    /// The camera (a `getUserMedia` error name) or the model (`model`) failed.
    Error {
        /// The `DOMException` or error name.
        name: String,
        /// The model failed to load, not the camera.
        model: bool,
    },
    /// The track ended and the worker closed, after a stop request.
    Stopped,
    /// Self-test only: whether the worker's attempt to reach the network was blocked.
    Probe {
        /// Blocked by the page's content policy.
        blocked: bool,
        /// What happened, in words.
        detail: String,
    },
}

/// A command for the camera page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageCommand {
    /// Start capture (the camera and the worker), or start it again.
    Start,
    /// End the track and close the worker, then report `stopped`.
    Stop,
    /// Close the worker and start a new one on the same track.
    RestartWorker,
    /// Run at this frame rate.
    Fps {
        /// Frames a second.
        fps: u8,
    },
    /// Self-test only: the worker stops answering (to test the watchdog).
    Freeze,
    /// Self-test only: the worker tries to reach the network.
    Probe,
    /// Self-test only: the page ignores every command from now on (a hung page).
    Hang,
}

/// The camera's state on the shell.
#[derive(Debug)]
pub struct CameraState {
    machine: Machine,
    origin: Instant,
    /// The permission guard is installed on the current camera window.
    guarded: bool,
    /// The next camera window opens on screen (the user opened it); otherwise hidden.
    show_next: bool,
    /// The self-test drives the camera.
    selftest: bool,
    /// Self-test only: refuse camera permission, as Windows would when access is off.
    deny_for_test: bool,
    /// How the current page captures, once started.
    path: Option<CapturePath>,
    /// Self-test only: the last network probe result.
    probe: Option<(bool, String)>,
    /// The tray's camera entry was last built for this choice.
    tray_on: Option<bool>,
    /// Self-test only: added to the clock, so posture traces run in seconds.
    skew: Duration,
    /// Self-test only: the cue outcomes heard and the posture records written.
    cue_outcomes: Vec<CueOutcome>,
    records: Vec<Observation>,
}

impl CameraState {
    /// The state for the stored settings.
    pub fn new(settings: CameraSettings, selftest: bool) -> Self {
        let mut machine = Machine::new(settings);
        machine.set_wall_origin(Utc::now());
        Self {
            machine,
            origin: Instant::now(),
            guarded: false,
            show_next: false,
            selftest,
            deny_for_test: false,
            path: None,
            probe: None,
            tray_on: None,
            skew: Duration::ZERO,
            cue_outcomes: Vec::new(),
            records: Vec::new(),
        }
    }

    fn now(&self) -> Duration {
        self.origin.elapsed() + self.skew
    }
}

impl Shell {
    fn camera_window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(CAMERA)
    }

    /// The posture check's status.
    pub fn camera_status(&self) -> Status {
        lock(&self.camera).machine.status()
    }

    /// The pet menu's and tray's camera entry.
    pub fn camera_menu(&self) -> CameraMenu {
        let c = lock(&self.camera);
        CameraMenu::of(c.machine.status(), c.machine.settings().on)
    }

    /// The camera page's view (also the settings' Posture check tab).
    pub fn camera_page(&self) -> CameraPage {
        // Today's coverage from the records (slice V2-3), read before the camera lock.
        let today = {
            let store = lock(&self.store);
            store.today().and_then(|d| store.posture_day(d)).ok()
        };
        // Lock order: inner before camera (the camera lock is a leaf).
        let a11y = lock(&self.inner).a11y;
        let c = lock(&self.camera);
        let m = &c.machine;
        let status = m.status();
        CameraPage {
            status,
            words: status.words(),
            on: m.settings().on,
            consented: m.settings().consent_current(),
            notice_version: hy_core::camera::NOTICE_VERSION,
            capture: m.wants_capture() && c.guarded,
            fps: m.fps(),
            stats: m.stats(),
            posture: posture_view(m, today),
            selftest: c.selftest,
            a11y,
        }
    }

    /// Whether the window `label` may use the camera now: only the camera window, only
    /// while the machine wants capture (never for a consent screen or after off).
    pub fn camera_permission(&self, label: &str) -> bool {
        let c = lock(&self.camera);
        label == CAMERA && c.machine.wants_capture() && !c.deny_for_test
    }

    // ------------------------------------------------------------ user actions

    /// "Posture check" from the pet menu, the panel, the tray or Settings: the camera
    /// page on screen, with the consent text if none is recorded, else its status.
    pub fn camera_open(self: &Arc<Self>) {
        if let Some(w) = self.camera_window() {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            return;
        }
        lock(&self.camera).show_next = true;
        self.create_camera_window(true);
    }

    /// "Turn on camera" (`accept`) or "Not now" on the consent text.
    ///
    /// # Errors
    /// The refusal in words.
    pub fn camera_consent(self: &Arc<Self>, accept: bool) -> Result<CameraPage, String> {
        if !accept {
            let off = !lock(&self.camera).machine.settings().on;
            if off && let Some(w) = self.camera_window() {
                let _ = w.destroy();
            }
            return Ok(self.camera_page());
        }
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            let mut fx = c.machine.consent(Utc::now());
            fx.extend(c.machine.turn_on(now).map_err(|e| e.to_string())?);
            fx
        };
        self.camera_apply(actions);
        Ok(self.camera_page())
    }

    /// "Turn camera off" from anywhere: capture ends within two seconds.
    pub fn camera_off(self: &Arc<Self>) -> CameraPage {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.turn_off(now)
        };
        self.camera_apply(actions);
        self.camera_page()
    }

    /// "Turn camera on" from Settings when consent is on record (else the consent page).
    pub fn camera_on(self: &Arc<Self>) -> CameraPage {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.turn_on(now)
        };
        match actions {
            Ok(fx) => self.camera_apply(fx),
            Err(_) => self.camera_open(),
        }
        self.camera_page()
    }

    /// "Try again" after the camera was denied, failed or stopped.
    pub fn camera_try_again(self: &Arc<Self>) -> CameraPage {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            if !c.machine.settings().on && c.machine.settings().consent_current() {
                // After a stop for stalls the choice was switched off: turn it on again.
                c.machine.turn_on(now).unwrap_or_default()
            } else {
                c.machine.try_again(now)
            }
        };
        self.camera_apply(actions);
        self.camera_page()
    }

    /// "Withdraw consent" in Settings: off, and the consent text is shown next time.
    pub fn camera_withdraw(self: &Arc<Self>) -> CameraPage {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.withdraw_consent(now)
        };
        self.camera_apply(actions);
        self.camera_page()
    }

    /// "Calibrate" (`start`) or "Cancel" (slice V2-2).
    ///
    /// # Errors
    /// Calibration needs the camera on and reading.
    pub fn camera_calibrate(self: &Arc<Self>, start: bool) -> Result<CameraPage, String> {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            if start {
                c.machine.calibrate(now).map_err(|e| e.to_string())?
            } else {
                c.machine.cancel_calibration()
            }
        };
        self.camera_apply(actions);
        Ok(self.camera_page())
    }

    /// The coaching engine's answer to the posture cue.
    fn camera_cue_outcome(&self, outcome: CueOutcome) {
        let mut c = lock(&self.camera);
        c.machine.cue_outcome(outcome);
        if c.selftest {
            c.cue_outcomes.push(outcome);
        }
    }

    /// A drift episode started: ask the engine (on its thread) for the cue.
    fn camera_ask_cue(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let queued = self.runner().is_some_and(|r| {
            r.submit(move |link| {
                let outcome = match link.input(Input::PostureCue) {
                    Ok(out) => cue_outcome_of(&out.outcome),
                    Err(_) => CueOutcome::NoAnswer,
                };
                if let Some(s) = weak.upgrade() {
                    s.post(move |s| s.camera_cue_outcome(outcome));
                }
            })
        });
        if !queued {
            self.camera_cue_outcome(CueOutcome::NoAnswer);
        }
    }

    /// Write a posture observation (local only; never shared).
    fn camera_record(&self, o: Observation) {
        let status = match o.kind {
            PostureKind::DriftEpisode => MeasurementStatus::Estimated,
            PostureKind::NoReading | PostureKind::BaselineSuspect => MeasurementStatus::NoReading,
            PostureKind::Capture => MeasurementStatus::Measured,
        };
        // The record's time is when the interval ended (an episode ends when the calm
        // began, a few seconds before it is written), so its start is that time less its
        // length (slice V2-3's report and a day's coverage rely on it).
        let ended = Utc::now() - chrono::TimeDelta::from_std(o.ago).unwrap_or_default();
        let event = NewEvent::inferred(o.payload(), status).at(ended);
        if let Err(e) = lock(&self.store).append_event(event) {
            eprintln!("HealthierYou: could not record a posture observation: {e}");
            return;
        }
        let mut c = lock(&self.camera);
        if c.selftest {
            c.records.push(o);
        }
    }

    // ------------------------------------------------------------ the page

    /// A report from the camera page.
    ///
    /// # Errors
    /// A malformed report.
    pub fn camera_report(self: &Arc<Self>, report: PageReport) -> Result<(), String> {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            match report {
                PageReport::Started {
                    width,
                    height,
                    cores,
                    model_ms,
                    path,
                } => {
                    if !model_ms.is_finite() || model_ms < 0.0 {
                        return Err("bad model load time".into());
                    }
                    c.path = Some(path);
                    c.machine.page_started(
                        now,
                        Started {
                            width,
                            height,
                            cores,
                            model_ms,
                        },
                    )
                }
                PageReport::Error { name, model } => {
                    if name.is_empty() || name.chars().count() > 64 {
                        return Err("bad error name".into());
                    }
                    c.machine.page_error(now, &name, model)
                }
                PageReport::Stopped => c.machine.page_stopped(now),
                PageReport::Probe { blocked, detail } => {
                    let detail: String = detail.chars().take(200).collect();
                    c.probe = Some((blocked, detail));
                    Vec::new()
                }
            }
        };
        self.camera_apply(actions);
        Ok(())
    }

    /// One frame's derived result from the page.
    ///
    /// # Errors
    /// A report that breaks its rules.
    pub fn camera_frame(self: &Arc<Self>, report: FrameReport) -> Result<(), String> {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.frame(now, report).map_err(|e| e.to_string())?
        };
        self.camera_apply(actions);
        Ok(())
    }

    /// The permission guard is on the camera window: capture may start.
    pub fn camera_guarded(self: &Arc<Self>) {
        let start = {
            let mut c = lock(&self.camera);
            c.guarded = true;
            c.machine.wants_capture()
        };
        if start {
            self.camera_send(PageCommand::Start);
        }
        self.camera_views();
    }

    /// The permission guard could not be installed: no capture on this page.
    pub fn camera_guard_failed(self: &Arc<Self>) {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.page_error(now, "GuardFailed", false)
        };
        self.camera_apply(actions);
    }

    /// The camera window was destroyed (by the shell or the user).
    pub fn camera_window_closed(self: &Arc<Self>) {
        let actions = {
            let mut c = lock(&self.camera);
            c.guarded = false;
            c.path = None;
            let now = c.now();
            c.machine.page_gone(now)
        };
        self.camera_apply(actions);
    }

    /// The user asked to close the camera window: while the camera may be capturing it
    /// only hides (the CAM badge stays and the camera keeps running); otherwise it closes.
    /// Returns true if the close should go ahead.
    pub fn camera_close_requested(&self) -> bool {
        if self.camera_status().capturing() {
            if let Some(w) = self.camera_window() {
                let _ = w.hide();
            }
            return false;
        }
        true
    }

    // ------------------------------------------------------------ holds and time

    /// The engine's holds changed.
    pub fn camera_set_hold(self: &Arc<Self>, hold: Option<CameraHold>) {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.set_hold(now, hold)
        };
        self.camera_apply(actions);
    }

    /// The app started: the posture check comes back on if the user left it on.
    pub fn camera_resume(self: &Arc<Self>) {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.resume(now)
        };
        self.camera_apply(actions);
    }

    /// The machine's clock.
    pub fn camera_tick(self: &Arc<Self>) {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.tick(now)
        };
        self.camera_apply(actions);
    }

    // ------------------------------------------------------------ actions

    fn camera_apply(self: &Arc<Self>, actions: Vec<Action>) {
        let mut render = false;
        for a in actions {
            match a {
                Action::OpenPage => {
                    if self.camera_window().is_some() {
                        if lock(&self.camera).guarded {
                            self.camera_send(PageCommand::Start);
                        }
                    } else {
                        let show = std::mem::take(&mut lock(&self.camera).show_next);
                        self.create_camera_window(show);
                    }
                }
                Action::RequestStop => self.camera_send(PageCommand::Stop),
                Action::ClosePage => {
                    if !self.close_camera_window(false) {
                        self.camera_close_done();
                    }
                }
                Action::DestroyPage => {
                    if !self.close_camera_window(true) {
                        self.camera_close_done();
                    }
                }
                Action::RestartWorker => self.camera_send(PageCommand::RestartWorker),
                Action::Retry => self.camera_send(PageCommand::Start),
                Action::SetFps(fps) => self.camera_send(PageCommand::Fps { fps }),
                Action::Persist => {
                    let settings = lock(&self.camera).machine.settings();
                    if let Err(e) = lock(&self.store).put_setting(SETTINGS_KEY, &settings) {
                        eprintln!("HealthierYou: could not save the camera setting: {e}");
                    }
                }
                Action::Notify(n) => self.camera_notice(n),
                Action::Render => render = true,
                Action::Cue => self.camera_ask_cue(),
                Action::Record(o) => self.camera_record(o),
            }
        }
        if render {
            self.render();
            // The tray's entry changes only with the choice, not with every status.
            let tray_changed = {
                let mut c = lock(&self.camera);
                let on = c.machine.settings().on;
                c.tray_on.replace(on) != Some(on)
            };
            if tray_changed {
                crate::tray::refresh(&self.app);
            }
            self.camera_views();
        }
    }

    /// Send the open pages the camera's view.
    fn camera_views(&self) {
        let page = self.camera_page();
        let _ = self.app.emit_to(CAMERA, "camera://view", &page);
        if self.app.get_webview_window(SCREENS).is_some() {
            let _ = self.app.emit_to(SCREENS, "hy://camera", &page);
        }
    }

    fn camera_send(&self, command: PageCommand) {
        let _ = self.app.emit_to(CAMERA, "camera://command", command);
    }

    fn create_camera_window(self: &Arc<Self>, show: bool) {
        lock(&self.camera).guarded = false;
        let monitor = lock(&self.inner).core.monitor().cloned();
        if let Err(e) = windows::open_camera(self, monitor.as_ref(), show) {
            eprintln!("HealthierYou: could not open the camera page: {e}");
            let actions = {
                let mut c = lock(&self.camera);
                let now = c.now();
                c.machine.page_gone(now)
            };
            self.camera_apply(actions);
        }
    }

    /// Destroy the camera page, unless (`forced` false: the page confirmed its capture
    /// ended) the user is looking at it and it explains why the camera cannot be used: it
    /// then stays, without capture, until they close it. A forced close (the page did not
    /// confirm its stop in time) always destroys it, so a hung page cannot keep the
    /// camera. Returns whether a window is being destroyed (its end is reported by the
    /// window's Destroyed event).
    fn close_camera_window(&self, forced: bool) -> bool {
        let Some(w) = self.camera_window() else {
            return false;
        };
        let explains = matches!(self.camera_status(), Status::Unavailable(_));
        if !forced && explains && w.is_visible().unwrap_or(false) {
            return false;
        }
        w.destroy().is_ok()
    }

    /// No window end will be reported for the last close (none, or kept on screen).
    fn camera_close_done(self: &Arc<Self>) {
        let actions = {
            let mut c = lock(&self.camera);
            let now = c.now();
            c.machine.close_done(now)
        };
        self.camera_apply(actions);
    }

    fn camera_notice(self: &Arc<Self>, notice: CameraNotice) {
        // The page on screen says it itself, except "Camera off", which closes it.
        let visible = self
            .camera_window()
            .is_some_and(|w| w.is_visible().unwrap_or(false));
        if visible && !matches!(notice, CameraNotice::Off | CameraNotice::Recalibrate) {
            return;
        }
        let (title, line) = notice_text(notice);
        self.show_notice_for(title, line, NOTICE_FOR);
    }

    /// Show a short notice beside the pet for `for_how_long`.
    fn show_notice_for(self: &Arc<Self>, title: &str, line: &str, for_how_long: Duration) {
        let mut shown_id = None;
        self.run(|i, _| {
            i.notice_seq += 1;
            let id = format!("camera-{}", i.notice_seq);
            let mut bubble = Bubble::notice(&id, title);
            if !line.is_empty() {
                bubble.lines.push(line.to_owned());
            }
            let mut fx = i.core.model.show_notice(bubble);
            if !fx.is_empty() {
                shown_id = Some(id);
            }
            fx.push(Effect::Render);
            fx
        });
        if let Some(id) = shown_id {
            let me = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("hy-notice".into())
                .spawn(move || {
                    std::thread::sleep(for_how_long);
                    me.post(move |s| s.run(|i, _| i.core.model.notice_expired(&id)));
                });
        }
    }

    // ------------------------------------------------------------ self-test

    /// Self-test only: refuse camera permission from now on (as Windows would with camera
    /// access off), or allow it again.
    pub fn camera_test_deny(&self, deny: bool) {
        lock(&self.camera).deny_for_test = deny;
    }

    /// Self-test only: ask the page for a test-only action.
    pub fn camera_test_command(&self, command: PageCommand) {
        if lock(&self.camera).selftest {
            self.camera_send(command);
        }
    }

    /// Self-test only: what the page reported and was sent.
    pub fn camera_test_facts(&self) -> CameraFacts {
        let c = lock(&self.camera);
        CameraFacts {
            path: c.path,
            probe: c.probe.clone(),
            guarded: c.guarded,
            cue_outcomes: c.cue_outcomes.clone(),
            records: c.records.clone(),
            posture: c.machine.posture_state(),
        }
    }

    /// Self-test only: feed one synthetic frame report `step` after the last (the clock is
    /// moved on by `step`), as the page would. Used with the worker frozen, so only these
    /// frames arrive; nothing comes from a camera image.
    ///
    /// # Errors
    /// A report that breaks its rules.
    pub fn camera_test_frame(
        self: &Arc<Self>,
        report: FrameReport,
        step: Duration,
    ) -> Result<(), String> {
        {
            let mut c = lock(&self.camera);
            if !c.selftest {
                return Err("self-test only".into());
            }
            c.skew += step;
        }
        self.camera_frame(report)?;
        self.camera_tick();
        Ok(())
    }
}

/// What the self-test reads back.
#[derive(Debug, Clone)]
pub struct CameraFacts {
    /// How the page captures.
    pub path: Option<CapturePath>,
    /// The network probe's result.
    pub probe: Option<(bool, String)>,
    /// The permission guard is installed on the camera window.
    pub guarded: bool,
    /// The posture cue outcomes heard from the engine.
    pub cue_outcomes: Vec<CueOutcome>,
    /// The posture observations written.
    pub records: Vec<Observation>,
    /// The posture state.
    pub posture: PostureState,
}

fn posture_view(m: &Machine, today: Option<Coverage>) -> PostureView {
    let state = m.posture_state();
    PostureView {
        state,
        words: state.words(),
        issue: match state {
            PostureState::NotUsable(i) => Some(i.words()),
            _ => None,
        },
        calibrated_at: m.settings().calibration.map(|c| c.at),
        can_calibrate: matches!(m.status(), Status::Reading | Status::NoReading(_))
            && !matches!(state, PostureState::Calibrating { .. }),
        stats: m.posture_stats(),
        today,
    }
}

/// Start the camera's clock: a thread that posts a tick to the main thread every
/// [`TICK_EVERY`]. The tick is cheap while the camera is off.
pub fn start_camera_clock(shell: &Arc<Shell>) {
    let weak = Arc::downgrade(shell);
    let _ = std::thread::Builder::new()
        .name("hy-camera-clock".into())
        .spawn(move || {
            loop {
                std::thread::sleep(TICK_EVERY);
                let Some(s) = weak.upgrade() else { return };
                s.post(|s| s.camera_tick());
            }
        });
}

/// A camera notice's words (wireframe B11 and flow 9).
pub const fn notice_text(n: CameraNotice) -> (&'static str, &'static str) {
    match n {
        CameraNotice::Off => ("Camera off. Capture stopped.", ""),
        CameraNotice::Recalibrate => (
            "Camera position changed?",
            "The posture check no longer matches its calibration, so posture cues are paused. Choose Posture check, then Calibrate, to resume them.",
        ),
        CameraNotice::StoppedAfterStalls => (
            "Posture check stopped",
            "The camera page stopped responding twice, so the camera was turned off. Everything else keeps working. You can turn it on again from the pet menu.",
        ),
        CameraNotice::Unavailable(u) => match u {
            Unavailable::Denied => (
                "Posture check paused: Windows blocked the camera",
                "Allow camera access in Windows Settings > Privacy & security > Camera, then choose Posture check and Try again. Everything else keeps working.",
            ),
            Unavailable::Busy => (
                "Posture check paused: camera in use",
                "Another app is using the camera. Nothing is measured; it tries again every minute. Everything else keeps working.",
            ),
            Unavailable::NoCamera => (
                "Posture check paused: no camera found",
                "It tries again every minute. Everything else keeps working.",
            ),
            Unavailable::ModelFailed => (
                "Posture check could not start",
                "The posture model could not load. Everything else keeps working.",
            ),
            Unavailable::Stalled | Unavailable::Failed => (
                "Posture check could not start",
                "The camera page did not respond. Everything else keeps working.",
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_fit_a_bubble() {
        let all = [
            CameraNotice::Off,
            CameraNotice::Recalibrate,
            CameraNotice::StoppedAfterStalls,
            CameraNotice::Unavailable(Unavailable::Denied),
            CameraNotice::Unavailable(Unavailable::Busy),
            CameraNotice::Unavailable(Unavailable::NoCamera),
            CameraNotice::Unavailable(Unavailable::ModelFailed),
            CameraNotice::Unavailable(Unavailable::Stalled),
            CameraNotice::Unavailable(Unavailable::Failed),
        ];
        for n in all {
            let (title, line) = notice_text(n);
            let mut b = Bubble::notice("camera-1", title);
            if !line.is_empty() {
                b.lines.push(line.to_owned());
            }
            b.validate().unwrap();
        }
    }

    #[test]
    fn page_reports_and_commands_have_the_shapes_the_page_uses() {
        let r: PageReport = serde_json::from_value(serde_json::json!({
            "kind": "started", "width": 640, "height": 480, "cores": 8,
            "model_ms": 512.5, "path": "processor"
        }))
        .unwrap();
        assert!(matches!(
            r,
            PageReport::Started {
                path: CapturePath::Processor,
                ..
            }
        ));
        let r: PageReport = serde_json::from_value(
            serde_json::json!({ "kind": "error", "name": "NotAllowedError", "model": false }),
        )
        .unwrap();
        assert!(matches!(r, PageReport::Error { model: false, .. }));
        assert!(matches!(
            serde_json::from_value(serde_json::json!({ "kind": "stopped" })).unwrap(),
            PageReport::Stopped
        ));
        assert_eq!(
            serde_json::to_value(PageCommand::Fps { fps: 1 }).unwrap(),
            serde_json::json!({ "kind": "fps", "fps": 1 })
        );
        assert_eq!(
            serde_json::to_value(PageCommand::RestartWorker).unwrap(),
            serde_json::json!({ "kind": "restart_worker" })
        );
    }
}
