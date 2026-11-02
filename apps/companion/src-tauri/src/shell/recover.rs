//! A pet that never stays dead (W2, architecture 10).
//!
//! Every window's webview is watched for WebView2 process failures (`crate::windows`).
//! When the page is lost (its renderer exited or hangs, or the whole browser process
//! exited), the pet window is destroyed and made again: a new window, a new webview and
//! a new page, placed where the pet was, with the system signals and the hotkey attached
//! to the new window and the pet shown again unless the user or the engine had hidden
//! it. At most 5 recreations in 10 minutes ([`RecoveryBudget::pet`]): a page that dies
//! again at once is left, and the log says so. Another window whose page is lost is
//! closed (the user opens it again from the tray); the camera page is left to the
//! camera's own watchdog, which notices the missing frames and releases the camera.
//! Failures of WebView2's helper processes (GPU, utility) are only logged: WebView2
//! restarts them and the page goes on.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hy_core::diag::{Level, RecoveryBudget, V, webview_failure};
use tauri::Manager;

use super::{CAMERA, PANEL, PET, PLAN, REVIEW, SCREENS, SETUP, Shell, lock};
use crate::diag::log;

/// How long the old pet window may take to go before giving up.
const GONE_WAIT: Duration = Duration::from_secs(10);

fn budget() -> &'static Mutex<RecoveryBudget> {
    static B: OnceLock<Mutex<RecoveryBudget>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(RecoveryBudget::pet()))
}

/// The window's fixed name for the log.
fn window_name(label: &str) -> &'static str {
    match label {
        PET => "pet",
        PANEL => "panel",
        SCREENS => "screens",
        REVIEW => "review",
        PLAN => "plan",
        SETUP => "setup",
        CAMERA => "camera",
        _ => "other",
    }
}

impl Shell {
    /// The webview of window `label` reported a process failure of `kind`
    /// (`COREWEBVIEW2_PROCESS_FAILED_KIND`). On the UI thread.
    pub fn webview_failed(self: &Arc<Self>, label: &str, kind: i32) {
        let f = webview_failure(kind);
        let level = if f.page_lost {
            Level::Warn
        } else {
            Level::Info
        };
        log(
            level,
            "webview_failed",
            &[
                ("window", V::Name(window_name(label))),
                ("kind", V::Name(f.kind)),
                ("page_lost", V::Bool(f.page_lost)),
            ],
        );
        if !f.page_lost {
            return;
        }
        match label {
            PET => {
                self.recreate_pet_window();
            }
            CAMERA => {}
            _ => {
                if let Some(w) = self.app.get_webview_window(label) {
                    let _ = w.destroy();
                }
            }
        }
    }

    /// Destroy the pet window and make it again (see the module notes). On the UI
    /// thread. Returns false if the budget is spent (nothing is done then).
    pub fn recreate_pet_window(self: &Arc<Self>) -> bool {
        let allowed = budget()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_take(Instant::now());
        if !allowed {
            log(
                Level::Error,
                "webview_recreate_gave_up",
                &[("window", V::Name("pet"))],
            );
            return false;
        }
        lock(&self.inner).page_ready = false;
        if let Some(w) = self.pet_window() {
            let _ = w.destroy();
        }
        self.create_pet_when_gone(Instant::now());
        true
    }

    /// Make the pet window once the old one has gone from the app's windows (its
    /// destruction finishes in a later turn of the event loop).
    fn create_pet_when_gone(self: &Arc<Self>, since: Instant) {
        if self.pet_window().is_some() {
            if since.elapsed() > GONE_WAIT {
                log(
                    Level::Error,
                    "webview_recreate_failed",
                    &[
                        ("window", V::Name("pet")),
                        ("reason", V::Name("old_window_stayed")),
                    ],
                );
                return;
            }
            let me = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("hy-pet-recreate".into())
                .spawn(move || {
                    std::thread::sleep(Duration::from_millis(100));
                    me.post(move |s| s.create_pet_when_gone(since));
                });
            return;
        }
        if let Err(e) = crate::windows::create_pet_window(&self.app) {
            eprintln!("HealthierYou: could not make the pet window again: {e}");
            log(
                Level::Error,
                "webview_recreate_failed",
                &[("window", V::Name("pet")), ("reason", V::Name("create"))],
            );
            return;
        }
        self.attach_pet_window();
        // Placed again from where the pet was; the new window gets its rectangle.
        self.on_display_change(true);
        self.render();
        if lock(&self.inner).core.model.is_visible() {
            self.show_window();
        }
        log(
            Level::Warn,
            "webview_recreated",
            &[
                ("window", V::Name("pet")),
                (
                    "after_ms",
                    V::N(u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)),
                ),
            ],
        );
    }
}
