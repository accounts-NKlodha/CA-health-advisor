//! The few Win32 calls the pet shell needs (Windows only).
//!
//! - The pet window gets `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, as proven by feasibility
//!   batch 1 (P3: the typing cursor stayed in Notepad for 10 of 10 pet clicks), so a click
//!   never takes keyboard focus and the pet has no taskbar or Alt+Tab entry.
//! - The pet is shown with `SW_SHOWNOACTIVATE` and hidden with `SW_HIDE`, because a plain
//!   show would activate it.
//! - A window subclass on the pet hears `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE` (work area,
//!   taskbar), `WM_DPICHANGED` and the panel hotkey's `WM_HOTKEY`. It only forwards a
//!   signal; all work happens later on the UI thread.
//! - The keyboard panel records the foreground window before it takes focus and gives
//!   focus back to it on Escape (PET-04; architecture A8).
//!
//! Window handles cross this module as `isize` so callers need no unsafe code.

#![allow(unsafe_code)]

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, HWND_TOPMOST,
    IsWindow, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_HOTKEY, WM_SETTINGCHANGE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

/// Something Windows told the pet window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The display layout may have changed (monitor, resolution, scaling or work area).
    DisplayChanged,
    /// The keyboard-panel hotkey was pressed.
    Hotkey,
}

type SignalSink = Box<dyn Fn(Signal) + Send + Sync>;

static SINK: OnceLock<SignalSink> = OnceLock::new();

/// The subclass id (any constant unique to this app).
const SUBCLASS_ID: usize = 0x4859_5045;
/// The hotkey id passed to `RegisterHotKey`.
const HOTKEY_ID: i32 = 0x4859;

fn hwnd(h: isize) -> HWND {
    h as HWND
}

/// Make the pet window non-activating and keep it out of the taskbar and Alt+Tab.
/// Tao can rewrite extended styles when window flags change, so this is applied again
/// after every show.
pub fn harden_pet(h: isize) {
    // SAFETY: `h` is a live window handle owned by this process (from Tauri). Reading and
    // writing GWL_EXSTYLE has no memory-safety requirements beyond a valid handle; on an
    // invalid handle both calls fail harmlessly.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd(h), GWL_EXSTYLE);
        let want =
            (ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize) & !(WS_EX_APPWINDOW as isize);
        if want != ex {
            SetWindowLongPtrW(hwnd(h), GWL_EXSTYLE, want);
        }
    }
}

/// The window's extended style (for the self-test).
pub fn ex_style(h: isize) -> u32 {
    // SAFETY: as in `harden_pet`; a read only.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    unsafe {
        GetWindowLongPtrW(hwnd(h), GWL_EXSTYLE) as u32
    }
}

/// Whether the extended style has both pet bits.
pub fn is_hardened(h: isize) -> bool {
    let ex = ex_style(h);
    ex & WS_EX_NOACTIVATE != 0 && ex & WS_EX_TOOLWINDOW != 0
}

/// Show without activating, then re-apply the pet styles.
pub fn show_no_activate(h: isize) {
    // SAFETY: valid handle owned by this process; ShowWindow has no pointer arguments.
    unsafe {
        ShowWindow(hwnd(h), SW_SHOWNOACTIVATE);
    }
    harden_pet(h);
}

/// Put the window back on top of other always-on-top windows without activating it
/// ("Bring pet above clock" also recovers a pet hidden behind other windows).
pub fn raise_topmost(h: isize) {
    // SAFETY: valid handle owned by this process; HWND_TOPMOST is a documented sentinel.
    unsafe {
        SetWindowPos(
            hwnd(h),
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Hide.
pub fn hide(h: isize) {
    // SAFETY: as in `show_no_activate`.
    unsafe {
        ShowWindow(hwnd(h), SW_HIDE);
    }
}

/// Whether the window is visible.
pub fn is_visible(h: isize) -> bool {
    // SAFETY: a read on a handle; returns 0 for an invalid handle.
    unsafe { IsWindowVisible(hwnd(h)) != 0 }
}

unsafe extern "system" fn subclass_proc(
    h: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let signal = match msg {
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => Some(Signal::DisplayChanged),
        WM_HOTKEY if usize::try_from(HOTKEY_ID).ok() == Some(wparam) => Some(Signal::Hotkey),
        _ => None,
    };
    if let (Some(s), Some(sink)) = (signal, SINK.get()) {
        sink(s);
    }
    // SAFETY: forwarding the unchanged message to the next handler in the subclass
    // chain, as the subclassing contract requires.
    unsafe { DefSubclassProc(h, msg, wparam, lparam) }
}

/// Start forwarding display-change and hotkey messages of the pet window to `sink`.
/// Must be called on the thread that owns the window (Tauri's main thread). The sink
/// must not block: it runs inside the window procedure.
pub fn install_signals(h: isize, sink: SignalSink) -> bool {
    if SINK.set(sink).is_err() {
        return false;
    }
    // SAFETY: `subclass_proc` matches SUBCLASSPROC and stays valid for the program's
    // life; the handle belongs to this thread's window. The subclass is never removed
    // because the pet window lives until the process exits.
    unsafe { SetWindowSubclass(hwnd(h), Some(subclass_proc), SUBCLASS_ID, 0) != 0 }
}

/// Register the panel hotkey on the pet window; false if another app holds it.
pub fn register_hotkey(h: isize, modifiers: u32, vk: u32) -> bool {
    // SAFETY: valid handle owned by this thread; plain integer arguments.
    unsafe {
        UnregisterHotKey(hwnd(h), HOTKEY_ID);
        RegisterHotKey(hwnd(h), HOTKEY_ID, modifiers, vk) != 0
    }
}

/// Remove the panel hotkey.
pub fn unregister_hotkey(h: isize) {
    // SAFETY: as in `register_hotkey`; failing when none is registered is harmless.
    unsafe {
        UnregisterHotKey(hwnd(h), HOTKEY_ID);
    }
}

/// The current foreground window, or 0.
pub fn foreground() -> isize {
    // SAFETY: no arguments; returns a handle value or null.
    unsafe { GetForegroundWindow() as isize }
}

/// Whether the window belongs to this process (the panel never hands focus to our own
/// windows).
pub fn is_own_window(h: isize) -> bool {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer for the call's duration.
    unsafe {
        GetWindowThreadProcessId(hwnd(h), &raw mut pid);
        pid == GetCurrentProcessId()
    }
}

/// Give focus back to `prev` if it still exists. Called while the panel has focus, so
/// Windows allows this process to set the foreground window.
pub fn restore_foreground(prev: isize) -> bool {
    if prev == 0 {
        return false;
    }
    // SAFETY: IsWindow validates the handle first; SetForegroundWindow takes a handle only.
    unsafe { IsWindow(hwnd(prev)) != 0 && SetForegroundWindow(hwnd(prev)) != 0 }
}
