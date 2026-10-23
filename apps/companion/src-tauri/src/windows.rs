//! Window creation (architecture 4.1): the pet window always exists; the keyboard panel,
//! the settings window and the review window are created on demand and destroyed on
//! close, so their memory is returned (A7: idle at most 150 MB).

use std::sync::Arc;

use hy_pet::geometry::{Monitor, Rect};
use hy_pet::placement::screens_window_logical;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::shell::{PANEL, PET, PLAN, REVIEW, SCREENS, Shell, set_window_rect};

/// The same WebView2 arguments for every window: windows sharing one WebView2
/// environment must use identical options. These switch off Edge features that would
/// otherwise contact the internet (batch 1; architecture 9).
pub const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

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
        .additional_browser_args(BROWSER_ARGS)
        .build()?;
    #[cfg(windows)]
    if let Some(h) = crate::shell::hwnd_of(&w) {
        crate::win32::harden_pet(h);
    }
    #[cfg(not(windows))]
    let _ = w;
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
        .additional_browser_args(BROWSER_ARGS)
        .build()?;
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
        .additional_browser_args(BROWSER_ARGS)
        .build()?;
    w.set_focus()
}
