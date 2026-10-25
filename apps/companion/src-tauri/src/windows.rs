//! Window creation (architecture 4.1): the pet window always exists; the keyboard panel,
//! the settings window and the review window are created on demand and destroyed on
//! close, so their memory is returned (A7: idle at most 150 MB). The camera page (slice
//! V2-1) exists only while the posture check needs it.
//!
//! Every window gets the camera permission guard (architecture 4.5): only the camera
//! window may use the camera, only while the posture check wants capture, and no answer
//! is ever saved in the WebView2 profile (all pages share one origin).

use std::sync::{Arc, OnceLock};

use hy_pet::geometry::{Monitor, Rect};
use hy_pet::placement::screens_window_logical;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::shell::{CAMERA, PANEL, PET, PLAN, REVIEW, SCREENS, SETUP, Shell, set_window_rect};

/// The same WebView2 arguments for every window: windows sharing one WebView2
/// environment must use identical options. These switch off Edge features that would
/// otherwise contact the internet (batch 1; architecture 9). V2-5: the camera soak found
/// a WebView2 process of the app holding a TLS connection to a Microsoft address on the
/// CI runner (architecture 10: no internet traffic; A12), so Chromium's background
/// networking (variations, component updates, pings) is switched off too.
pub const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-background-networking --disable-component-update --no-pings";

/// The self-test's extra argument: Chromium's synthetic camera (a generated test pattern,
/// no image of anyone), so the camera page runs end to end on a CI runner without a
/// webcam.
pub const SELFTEST_CAMERA_ARG: &str = "--use-fake-device-for-media-stream";

/// The arguments for this run's windows ([`BROWSER_ARGS`], plus the synthetic camera in
/// the self-test only). Every window gets the same string.
pub fn browser_args() -> &'static str {
    static ARGS: OnceLock<String> = OnceLock::new();
    ARGS.get_or_init(|| {
        let selftest = std::env::var_os("HY_SELFTEST").is_some_and(|v| !v.is_empty());
        if selftest {
            format!("{BROWSER_ARGS} {SELFTEST_CAMERA_ARG}")
        } else {
            BROWSER_ARGS.to_owned()
        }
    })
}

/// The camera page's size, logical pixels.
pub const CAMERA_LOGICAL: (f64, f64) = (520.0, 600.0);

/// Install the permission guard on a window (Windows; see the module notes). `then` runs
/// on the UI thread once the guard is in place (with false if it could not be).
fn guard(w: &WebviewWindow, then: impl FnOnce(bool) + Send + 'static) {
    let label = w.label().to_owned();
    #[cfg(windows)]
    {
        let app = w.app_handle().clone();
        let installed = w.with_webview(move |pw| {
            let rule: crate::win32::PermissionRule = Box::new(move |camera| {
                camera
                    && app
                        .try_state::<Arc<Shell>>()
                        .is_some_and(|s| s.camera_permission(&label))
            });
            then(crate::win32::guard_permissions(&pw.controller(), rule));
        });
        if let Err(e) = installed {
            eprintln!("HealthierYou: could not reach a webview to guard it: {e}");
        }
    }
    #[cfg(not(windows))]
    {
        // Development builds on other systems: Tauri's own handler (main.rs) decides.
        let _ = label;
        then(true);
    }
}

/// The keyboard panel's size, logical pixels.
pub const PANEL_LOGICAL: (f64, f64) = (340.0, 400.0);

/// Create the pet window: transparent, undecorated, always on top, no taskbar entry,
/// never focusable (Tao adds `WS_EX_NOACTIVATE` for a non-focusable window; the shell
/// adds `WS_EX_TOOLWINDOW` as proven in batch 1). It starts hidden and the shell shows
/// it without activation once placed.
///
/// # Errors
/// A Tauri window-creation error.
pub fn create_pet_window(app: &AppHandle) -> tauri::Result<()> {
    let w = WebviewWindowBuilder::new(app, PET, WebviewUrl::App("pet.html".into()))
        .title("HealthierYou pet")
        .inner_size(144.0, 144.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .additional_browser_args(browser_args())
        .build()?;
    #[cfg(windows)]
    if let Some(h) = crate::shell::hwnd_of(&w) {
        crate::win32::harden_pet(h);
    }
    guard(&w, |_| {});
    Ok(())
}

/// Create the keyboard panel beside the pet and give it focus.
///
/// # Errors
/// A Tauri window-creation error.
pub fn open_panel(shell: &Arc<Shell>, rect: Option<Rect>) -> tauri::Result<()> {
    let app = shell.app();
    let w = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("panel.html".into()))
        .title("HealthierYou pet panel")
        .inner_size(PANEL_LOGICAL.0, PANEL_LOGICAL.1)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .visible(false)
        .focused(true)
        .additional_browser_args(browser_args())
        .build()?;
    guard(&w, |_| {});
    if let Some(r) = rect {
        set_window_rect(&w, r);
    } else {
        let _ = w.center();
    }
    let weak = Arc::downgrade(shell);
    w.on_window_event(move |e| {
        if let tauri::WindowEvent::Focused(false) = e
            && let Some(s) = weak.upgrade()
        {
            // Focus went elsewhere (a click on another app): close quietly, focus is
            // already where the user put it.
            s.post(|s| s.close_panel(false));
        }
    });
    w.show()?;
    w.set_focus()?;
    Ok(())
}

/// Open the settings window (or focus it if open). It is at most 90% of the work area
/// and at least 640 x 440 logical pixels where the display allows (wireframe section 6).
///
/// # Errors
/// A Tauri window-creation error.
pub fn open_settings(app: &AppHandle, monitor: Option<&Monitor>) -> tauri::Result<()> {
    open_screen(
        app,
        SCREENS,
        "screens.html",
        "HealthierYou - Settings",
        monitor,
    )
}

/// Open the review window (wireframes W13 and W14), or focus it if open. Sized like the
/// settings window.
///
/// # Errors
/// A Tauri window-creation error.
pub fn open_review(app: &AppHandle, monitor: Option<&Monitor>) -> tauri::Result<()> {
    open_screen(
        app,
        REVIEW,
        "review.html",
        "HealthierYou - Weekly review",
        monitor,
    )
}

/// Open the recovery-day plan window (wireframe W15), or focus it if open. Sized like the
/// settings window.
///
/// # Errors
/// A Tauri window-creation error.
pub fn open_plan(app: &AppHandle, monitor: Option<&Monitor>) -> tauri::Result<()> {
    open_screen(
        app,
        PLAN,
        "plan.html",
        "HealthierYou - Recovery-day plan",
        monitor,
    )
}

/// Open the setup wizard (wireframe W1), or focus it if open. Sized like the settings
/// window.
///
/// # Errors
/// A Tauri window-creation error.
pub fn open_setup(app: &AppHandle, monitor: Option<&Monitor>) -> tauri::Result<()> {
    open_screen(app, SETUP, "setup.html", "HealthierYou - Setup", monitor)
}

/// An ordinary window (created on open, destroyed on close), or focus it if it is open.
fn open_screen(
    app: &AppHandle,
    label: &str,
    page: &str,
    title: &str,
    monitor: Option<&Monitor>,
) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        return w.set_focus();
    }
    let (width, height) = monitor.map_or((900.0, 640.0), screens_window_logical);
    let w = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title(title)
        .inner_size(width, height)
        .min_inner_size(width.min(640.0), height.min(440.0))
        .resizable(true)
        .center()
        .focused(true)
        .additional_browser_args(browser_args())
        .build()?;
    guard(&w, |_| {});
    w.set_focus()
}

/// Create the camera page (architecture 4.5): its own window, so its memory goes back
/// when it closes. It is on screen when the user opened it (`show`), else hidden while
/// the posture check runs in the background. Capture starts only after the permission
/// guard is in place ([`Shell::camera_guarded`]). Closing it while the camera runs only
/// hides it.
///
/// # Errors
/// A Tauri window-creation error.
pub fn open_camera(shell: &Arc<Shell>, monitor: Option<&Monitor>, show: bool) -> tauri::Result<()> {
    let app = shell.app();
    let (width, height) = monitor.map_or(CAMERA_LOGICAL, |m| {
        let (w, h) = screens_window_logical(m);
        (CAMERA_LOGICAL.0.min(w), CAMERA_LOGICAL.1.min(h))
    });
    let w = WebviewWindowBuilder::new(app, CAMERA, WebviewUrl::App("camera.html".into()))
        .title("HealthierYou - Posture check")
        .inner_size(width, height)
        .resizable(true)
        .maximizable(false)
        .center()
        .visible(show)
        .focused(show)
        .additional_browser_args(browser_args())
        .build()?;
    let weak = Arc::downgrade(shell);
    w.on_window_event(move |e| {
        let Some(s) = weak.upgrade() else { return };
        match e {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if !s.camera_close_requested() {
                    api.prevent_close();
                }
            }
            tauri::WindowEvent::Destroyed => s.post(|s| s.camera_window_closed()),
            _ => {}
        }
    });
    let weak = Arc::downgrade(shell);
    guard(&w, move |ok| {
        if let Some(s) = weak.upgrade() {
            if ok {
                s.post(|s| s.camera_guarded());
            } else {
                eprintln!("HealthierYou: the camera permission guard could not be installed");
                s.post(|s| s.camera_guard_failed());
            }
        }
    });
    Ok(())
}
