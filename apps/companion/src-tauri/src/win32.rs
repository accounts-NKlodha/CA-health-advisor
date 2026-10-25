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
//! - Install and update (slice V1-6): the standard Open dialog for choosing an update
//!   file, running an installer through the shell (`ShellExecuteExW`, so Windows applies
//!   its usual checks to a downloaded file: nothing is bypassed) and waiting for it,
//!   waiting for a process to end, the per-user Run value for start at sign-in, a named
//!   mutex that says the app is running, a yes/no message box, and the process's DPI
//!   awareness for the self-test.
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

// ------------------------------------------------------------ install and update (V1-6)

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, GetLastError};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetExitCodeProcess, INFINITE, OpenMutexW, OpenProcess, PROCESS_SYNCHRONIZE,
    SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
    OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::HiDpi::{
    GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO, MessageBoxW, SW_SHOWNORMAL,
};

/// A NUL-terminated UTF-16 copy of `s`.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A NUL-terminated UTF-16 copy of an OS string (a path).
fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Ask for a HealthierYou installer with the standard Open dialog, owned by `owner` (0
/// for none). Runs on the calling thread, which must not be the UI thread: the dialog has
/// its own message loop. `None` if the user cancelled.
pub fn choose_installer(owner: isize, title: &str) -> Option<PathBuf> {
    // Filter pairs are NUL-separated and the list ends with a second NUL.
    let filter: Vec<u16> = "HealthierYou installer (*.exe)\0*.exe\0\0"
        .encode_utf16()
        .collect();
    let title = wide(title);
    let mut buf = vec![0u16; 32_768];
    // SAFETY: COM is initialised for this thread only and released below if it was. The
    // OPENFILENAMEW structure is fully zeroed then filled; every pointer in it (filter,
    // title, file buffer with its length) points to a live local for the duration of the
    // call. The dialog writes at most `nMaxFile` UTF-16 units into `buf`.
    unsafe {
        let co = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED.cast_unsigned());
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        #[allow(clippy::cast_possible_truncation)]
        {
            ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
            ofn.nMaxFile = buf.len() as u32;
        }
        ofn.hwndOwner = hwnd(owner);
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_EXPLORER
            | OFN_NOCHANGEDIR
            | OFN_DONTADDTORECENT
            | OFN_HIDEREADONLY;
        let ok = GetOpenFileNameW(&raw mut ofn) != 0;
        if co >= 0 {
            CoUninitialize();
        }
        if !ok {
            return None;
        }
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

/// How running a program through the shell ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRun {
    /// It ran and exited with this code.
    Exited(u32),
    /// The shell did not start it: the user declined a Windows prompt (1223), or a policy,
    /// SmartScreen or antivirus blocked it. The Win32 error code.
    NotStarted(u32),
}

/// Run `file` with `params` through the shell (`ShellExecuteExW`, verb "open") and wait
/// for it to exit. The shell applies Windows' usual checks to a file marked as downloaded
/// (Attachment Manager, SmartScreen), exactly as a double-click would; nothing here
/// suppresses them.
pub fn shell_run_and_wait(file: &Path, params: &str, dir: &Path) -> ShellRun {
    let verb = wide("open");
    let file = wide_os(file.as_os_str());
    let params = wide(params);
    let dir = wide_os(dir.as_os_str());
    // SAFETY: the structure is zeroed then filled with pointers to live locals that
    // outlive the call. With SEE_MASK_NOCLOSEPROCESS the shell returns a process handle,
    // which is waited on and closed exactly once here.
    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        #[allow(clippy::cast_possible_truncation)]
        {
            sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        }
        sei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.lpDirectory = dir.as_ptr();
        sei.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&raw mut sei) == 0 {
            return ShellRun::NotStarted(GetLastError());
        }
        if sei.hProcess.is_null() {
            // Handed to an already-running process (not the case for an .exe).
            return ShellRun::Exited(0);
        }
        WaitForSingleObject(sei.hProcess, INFINITE);
        let mut code = 0u32;
        if GetExitCodeProcess(sei.hProcess, &raw mut code) == 0 {
            code = u32::MAX;
        }
        CloseHandle(sei.hProcess);
        ShellRun::Exited(code)
    }
}

/// Wait up to `timeout` for process `pid` to end. True if it has ended (or never
/// existed).
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: OpenProcess has no pointer arguments; a returned handle is waited on and
    // closed exactly once.
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return true;
        }
        let r = WaitForSingleObject(h, ms);
        CloseHandle(h);
        r == 0
    }
}

/// The named mutex that says a HealthierYou companion is running in this session.
const RUNNING_MUTEX: &str = "Local\\HealthierYou.Companion.Running";
const HELPER_MUTEX: &str = "Local\\HealthierYou.Companion.UpdateHelper";

/// Record that the companion is running (for the lifetime of this process).
pub fn mark_running() {
    hold_mutex(RUNNING_MUTEX);
}

/// Marks this process as the update helper until it ends (slice V1-6), so a companion
/// started meanwhile leaves the helper's attempt alone.
pub fn mark_update_helper() {
    hold_mutex(HELPER_MUTEX);
}

/// Whether an update helper is running in this session.
pub fn update_helper_running() -> bool {
    mutex_exists(HELPER_MUTEX)
}

fn hold_mutex(mutex: &str) {
    let name = wide(mutex);
    // SAFETY: a null security descriptor and a live name. The handle is deliberately kept
    // open until the process ends, which is what marks the app as running.
    unsafe {
        let _ = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
    }
}

/// Whether a companion is running in this session (one that called [`mark_running`]).
pub fn companion_running() -> bool {
    mutex_exists(RUNNING_MUTEX)
}

fn mutex_exists(mutex: &str) -> bool {
    let name = wide(mutex);
    // SAFETY: a live name; a returned handle is closed at once.
    unsafe {
        let h = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr());
        if h.is_null() {
            return false;
        }
        CloseHandle(h);
        true
    }
}

/// Ask a yes/no question in a standard message box (before any window exists). True for
/// Yes.
pub fn ask_yes_no(title: &str, text: &str) -> bool {
    let (title, text) = (wide(title), wide(text));
    // SAFETY: live, NUL-terminated strings; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND,
        ) == IDYES
    }
}

/// Tell the user something in a standard message box (before any window exists).
pub fn tell(title: &str, text: &str) {
    let (title, text) = (wide(title), wide(text));
    // SAFETY: as in `ask_yes_no`.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
        );
    }
}

/// The per-user Run key (start at sign-in, architecture 4.1).
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Where Windows records that the user switched a startup app off (Task Manager or
/// Settings > Apps > Startup).
const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// The command in the per-user Run value `name`, if there is one.
pub fn run_value(name: &str) -> Option<String> {
    let key = wide(RUN_KEY);
    let value = wide(name);
    let mut buf = vec![0u16; 2048];
    let mut bytes = u32::try_from(buf.len() * 2).unwrap_or(0);
    // SAFETY: live, NUL-terminated key and value names; `buf` holds `bytes` bytes and
    // RegGetValueW writes at most that many (it NUL-terminates REG_SZ data).
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if r != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Write the per-user Run value `name` = `command`.
///
/// # Errors
/// The registry error.
pub fn set_run_value(name: &str, command: &str) -> std::io::Result<()> {
    let key = wide(RUN_KEY);
    let value = wide(name);
    let data = wide(command);
    let len = u32::try_from(data.len() * 2).map_err(std::io::Error::other)?;
    // SAFETY: live, NUL-terminated strings; `data` holds `len` bytes including the NUL.
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            len,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(r.cast_signed()))
    }
}

/// Remove the per-user Run value `name` (no error if it is not there).
///
/// # Errors
/// The registry error.
pub fn delete_run_value(name: &str) -> std::io::Result<()> {
    let key = wide(RUN_KEY);
    let value = wide(name);
    // SAFETY: live, NUL-terminated strings.
    let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
    if r == 0 || r == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(r.cast_signed()))
    }
}

/// Whether the user switched startup app `name` off in Windows (Task Manager or
/// Settings). Windows keeps that choice separately from the Run value; an odd first byte
/// means "off".
pub fn startup_switched_off(name: &str) -> bool {
    let key = wide(STARTUP_APPROVED_KEY);
    let value = wide(name);
    let mut buf = [0u8; 32];
    let mut bytes = u32::try_from(buf.len()).unwrap_or(0);
    // SAFETY: live, NUL-terminated names; `buf` holds `bytes` bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    r == 0 && bytes > 0 && buf[0] & 1 == 1
}

/// The calling thread's DPI awareness: 2 is per-monitor (the manifest asks for
/// per-monitor v2).
pub fn dpi_awareness() -> i32 {
    // SAFETY: no arguments beyond the context the first call returns.
    unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) }
}

// ------------------------------------------------------------------ camera (slice V2-1)

/// The process tree's use: this process and every process it started (the WebView2
/// processes), summed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TreeUsage {
    /// Processes counted.
    pub processes: u32,
    /// Working set, bytes.
    pub working_set: u64,
    /// Private bytes (commit charge).
    pub private_bytes: u64,
    /// Kernel plus user CPU time since each process started.
    pub cpu: Duration,
}

/// The process tree's memory and CPU time (architecture 4.5's camera budget is checked
/// from two samples). Processes that cannot be opened are skipped.
pub fn process_tree_usage() -> TreeUsage {
    use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut pairs: Vec<(u32, u32)> = Vec::new();
    // SAFETY: the snapshot handle is checked and closed once; the entry's size field is
    // set before the first call as the API requires.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return TreeUsage::default();
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0);
        if Process32FirstW(snap, &raw mut e) != 0 {
            loop {
                pairs.push((e.th32ProcessID, e.th32ParentProcessID));
                if Process32NextW(snap, &raw mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentProcessId() };
    let tree = tree_of(me, &pairs);
    let mut usage = TreeUsage::default();
    let ft = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    for pid in tree {
        // SAFETY: OpenProcess has no pointer arguments; the counters and times are written
        // into live locals of the stated sizes; the handle is closed once.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                continue;
            }
            let mut m: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            let cb = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>()).unwrap_or(0);
            if GetProcessMemoryInfo(h, (&raw mut m).cast::<PROCESS_MEMORY_COUNTERS>(), cb) != 0 {
                usage.working_set += m.WorkingSetSize as u64;
                usage.private_bytes += m.PrivateUsage as u64;
            }
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut c, mut x, mut k, mut u) = (zero, zero, zero, zero);
            if GetProcessTimes(h, &raw mut c, &raw mut x, &raw mut k, &raw mut u) != 0 {
                // FILETIME counts 100 ns units.
                usage.cpu += Duration::from_nanos((ft(k) + ft(u)).saturating_mul(100));
            }
            usage.processes += 1;
            CloseHandle(h);
        }
    }
    usage
}

/// `root` and its descendants, from (process, parent) pairs. A recycled parent id can
/// make a stale entry look like a child; the walk only follows ids it has not seen.
fn tree_of(root: u32, pairs: &[(u32, u32)]) -> Vec<u32> {
    let mut ids = vec![root];
    let mut i = 0;
    while i < ids.len() {
        let parent = ids[i];
        for &(pid, ppid) in pairs {
            if ppid == parent && pid != parent && !ids.contains(&pid) {
                ids.push(pid);
            }
        }
        i += 1;
    }
    ids
}

/// A permission decision for one request: whether the kind asked for is the camera, and
/// the answer (true allows).
pub type PermissionRule = Box<dyn Fn(bool) -> bool>;

/// Install the permission guard on a webview (architecture 4.5): every permission request
/// is answered by `rule` and the answer is never saved in the WebView2 profile, so an
/// allowed camera on the camera page never carries over to another page of the same
/// origin, and a refusal never outlives the moment. Returns false if the guard could not
/// be installed (the caller must then not start capture).
pub fn guard_permissions(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    rule: PermissionRule,
) -> bool {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DENY,
        ICoreWebView2PermissionRequestedEventArgs3,
    };
    use webview2_com::PermissionRequestedEventHandler;
    use windows_core::Interface;

    let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
        // SAFETY: COM calls on the live event arguments, on the UI thread that raised the
        // event; `kind` is a live local.
        unsafe {
            args.PermissionKind(&raw mut kind)?;
            // A runtime that cannot be told not to save the answer gets a refusal (which
            // it then saves; that only ever errs towards no camera).
            let saves_off = args
                .cast::<ICoreWebView2PermissionRequestedEventArgs3>()
                .and_then(|a3| a3.SetSavesInProfile(false))
                .is_ok();
            let allow = saves_off && rule(kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA);
            args.SetState(if allow {
                COREWEBVIEW2_PERMISSION_STATE_ALLOW
            } else {
                COREWEBVIEW2_PERMISSION_STATE_DENY
            })?;
        }
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: COM calls on the webview's own controller, on the UI thread (`with_webview`
    // runs there); the handler lives as long as the webview holds it.
    unsafe {
        let Ok(core) = controller.CoreWebView2() else {
            return false;
        };
        core.add_PermissionRequested(&handler, &raw mut token)
            .is_ok()
    }
}

#[cfg(test)]
mod tree_tests {
    use super::tree_of;

    #[test]
    fn the_tree_holds_the_root_and_its_descendants_only() {
        let pairs = [
            (1, 0),
            (10, 1),
            (11, 10),
            (12, 11),
            (20, 2),
            (21, 20),
            (13, 1),
        ];
        let mut t = tree_of(1, &pairs);
        t.sort_unstable();
        assert_eq!(t, vec![1, 10, 11, 12, 13]);
        // A process listing itself as its parent does not loop.
        assert_eq!(tree_of(5, &[(5, 5)]), vec![5]);
    }
}
