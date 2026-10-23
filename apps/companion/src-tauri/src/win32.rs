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
//! - The tracker's signals (architecture 4.2 and 7): the same subclass hears session lock
//!   and unlock (`WTSRegisterSessionNotification`, `WM_WTSSESSION_CHANGE`), sleep and wake
//!   (`WM_POWERBROADCAST`, also registered with `RegisterSuspendResumeNotification` for
//!   modern standby) and the end of the Windows session (`WM_ENDSESSION`). Input idle time
//!   comes from `GetLastInputInfo`, and presentation or full-screen state from
//!   `SHQueryUserNotificationState` plus a look at the foreground window. Only raw values
//!   are read here; the rules are in `hy_pet::link::signals`.
//!
//! - The server link's token is sealed with user-scope DPAPI (`CryptProtectData`, V1-5
//!   contract 6.1): only this Windows user on this PC can open it.
//!
//! Window handles cross this module as `isize` so callers need no unsafe code.

#![allow(unsafe_code)]

use std::sync::OnceLock;

use std::time::Duration;

use hy_core::secret::Secret;
use hy_pet::geometry::Rect;
use hy_pet::link::signals::{ForegroundWindow, NotificationState, idle_from_ticks};
use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, LocalFree, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::System::Power::RegisterSuspendResumeNotification;
use windows_sys::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, LASTINPUTINFO, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, QUERY_USER_NOTIFICATION_STATE, SHQueryUserNotificationState, SetWindowSubclass,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_WINDOW_HANDLE, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetDesktopWindow,
    GetForegroundWindow, GetShellWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, HWND_TOPMOST, IsIconic, IsWindow, IsWindowVisible,
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_HOTKEY, WM_POWERBROADCAST,
    WM_SETTINGCHANGE, WM_WTSSESSION_CHANGE, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

/// Something Windows told the pet window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The display layout may have changed (monitor, resolution, scaling or work area).
    DisplayChanged,
    /// The keyboard-panel hotkey was pressed.
    Hotkey,
    /// The session was locked.
    Lock,
    /// The session was unlocked.
    Unlock,
    /// The PC is going to sleep.
    Suspend,
    /// The PC woke up (both resume broadcasts map here; the engine ignores a repeat).
    Resume,
    /// Windows is signing out or shutting down: save now.
    EndSession,
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
        WM_WTSSESSION_CHANGE => match u32::try_from(wparam) {
            Ok(WTS_SESSION_LOCK) => Some(Signal::Lock),
            Ok(WTS_SESSION_UNLOCK) => Some(Signal::Unlock),
            _ => None,
        },
        WM_POWERBROADCAST => match u32::try_from(wparam) {
            Ok(PBT_APMSUSPEND) => Some(Signal::Suspend),
            Ok(PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) => Some(Signal::Resume),
            _ => None,
        },
        // wParam is TRUE when the session really ends.
        WM_ENDSESSION if wparam != 0 => Some(Signal::EndSession),
        _ => None,
    };
    if let (Some(s), Some(sink)) = (signal, SINK.get()) {
        sink(s);
    }
    // SAFETY: forwarding the unchanged message to the next handler in the subclass
    // chain, as the subclassing contract requires.
    unsafe { DefSubclassProc(h, msg, wparam, lparam) }
}

/// Start forwarding display-change, hotkey, session, power and end-of-session messages of
/// the pet window to `sink`, and ask Windows to send the session and power ones. Must be
/// called on the thread that owns the window (Tauri's main thread). The sink must not
/// block for long: it runs inside the window procedure.
pub fn install_signals(h: isize, sink: SignalSink) -> bool {
    if SINK.set(sink).is_err() {
        return false;
    }
    // SAFETY: `subclass_proc` matches SUBCLASSPROC and stays valid for the program's
    // life; the handle belongs to this thread's window. The subclass is never removed
    // because the pet window lives until the process exits.
    let subclassed = unsafe { SetWindowSubclass(hwnd(h), Some(subclass_proc), SUBCLASS_ID, 0) };
    // SAFETY: a valid top-level window handle owned by this thread; plain integer flags.
    // Both registrations last for the window's (the process's) life, so the returned
    // registration is never unregistered. A failure leaves the plain broadcasts, which
    // top-level windows receive anyway on most PCs.
    unsafe {
        WTSRegisterSessionNotification(hwnd(h), NOTIFY_FOR_THIS_SESSION);
        RegisterSuspendResumeNotification(hwnd(h) as HANDLE, DEVICE_NOTIFY_WINDOW_HANDLE);
    }
    subclassed != 0
}

/// Time since the last keyboard or mouse input in this session (`GetLastInputInfo`), or
/// zero if Windows cannot say.
pub fn idle_time() -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>()).unwrap_or(8),
        dwTime: 0,
    };
    // SAFETY: `info` is a valid, correctly sized out structure for the call's duration;
    // GetTickCount takes no arguments.
    unsafe {
        if GetLastInputInfo(&raw mut info) == 0 {
            return Duration::ZERO;
        }
        idle_from_ticks(GetTickCount(), info.dwTime)
    }
}

/// The shell's notification state (presentation mode, a full-screen app, and so on).
pub fn notification_state() -> NotificationState {
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    // SAFETY: `state` is a valid out pointer for the call's duration.
    let hr = unsafe { SHQueryUserNotificationState(&raw mut state) };
    if hr < 0 {
        NotificationState::Unknown
    } else {
        NotificationState::from_raw(state)
    }
}

fn rect(r: RECT) -> Rect {
    Rect::new(
        r.left,
        r.top,
        r.right.saturating_sub(r.left),
        r.bottom.saturating_sub(r.top),
    )
}

fn is_desktop_class(h: HWND) -> bool {
    let mut buf = [0u16; 32];
    // SAFETY: `buf` is a valid buffer of the length passed; the handle may be stale, in
    // which case the call returns 0.
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), 32) };
    let name = String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0).min(32)]);
    name == "Progman" || name == "WorkerW"
}

/// What the foreground window looks like, for the full-screen check; `None` if there is
/// no foreground window or Windows will not describe it.
pub fn foreground_window() -> Option<ForegroundWindow> {
    // SAFETY: every call takes the foreground handle (validated by IsWindow) or a no-arg
    // query, and the out structures are valid and correctly sized for each call.
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || IsWindow(fg) == 0 {
            return None;
        }
        let mut wr = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(fg, &raw mut wr) == 0 {
            return None;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(40);
        if mon.is_null() || GetMonitorInfoW(mon, &raw mut mi) == 0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        Some(ForegroundWindow {
            rect: rect(wr),
            monitor: rect(mi.rcMonitor),
            has_caption: style & WS_CAPTION == WS_CAPTION,
            shown: IsWindowVisible(fg) != 0 && IsIconic(fg) == 0,
            desktop: fg == GetShellWindow() || fg == GetDesktopWindow() || is_desktop_class(fg),
            own: is_own_window(fg as isize),
        })
    }
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

fn dpapi_blob(bytes: &[u8]) -> std::io::Result<CRYPT_INTEGER_BLOB> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| std::io::Error::other("blob too large"))?,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

/// Copy a DPAPI output blob out, overwrite the OS buffer and free it.
fn dpapi_take(out: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: on success DPAPI returns `cbData` bytes at `pbData`, allocated with
    // LocalAlloc; they are copied, the OS buffer is overwritten (it may hold the token)
    // and freed exactly once.
    unsafe {
        let len = out.cbData as usize;
        let v = std::slice::from_raw_parts(out.pbData, len).to_vec();
        std::ptr::write_bytes(out.pbData, 0, len);
        LocalFree(out.pbData.cast());
        v
    }
}

/// Seal `plain` for the current Windows user (user-scope DPAPI, no UI).
///
/// # Errors
/// The OS error if DPAPI refuses.
pub fn protect_user(plain: &[u8], entropy: &[u8]) -> std::io::Result<Vec<u8>> {
    let input = dpapi_blob(plain)?;
    let ent = dpapi_blob(entropy)?;
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: `input` and `ent` point to live slices for the duration of the call and
    // DPAPI only reads them; `out` is a valid out-pointer; the description, reserved and
    // prompt arguments may be null.
    let ok = unsafe {
        CryptProtectData(
            &raw const input,
            std::ptr::null(),
            &raw const ent,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut out,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(dpapi_take(&out))
}

/// Open a blob sealed by [`protect_user`]. Fails for another Windows user, another PC or
/// a damaged blob ("re-pair needed").
///
/// # Errors
/// The OS error if the blob cannot be opened here.
pub fn unprotect_user(sealed: &[u8], entropy: &[u8]) -> std::io::Result<Secret<Vec<u8>>> {
    let input = dpapi_blob(sealed)?;
    let ent = dpapi_blob(entropy)?;
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: as in `protect_user`; the description out-pointer may be null.
    let ok = unsafe {
        CryptUnprotectData(
            &raw const input,
            std::ptr::null_mut(),
            &raw const ent,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut out,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Secret::new(dpapi_take(&out)))
}
