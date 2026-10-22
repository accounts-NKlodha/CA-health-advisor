//! The decisions behind the Windows signals the companion feeds the engine (architecture
//! 4.2 tracker and suppression, 7 activity signals): input idle time from
//! `GetLastInputInfo`, and presentation or full-screen detection from
//! `SHQueryUserNotificationState` plus a check of the foreground window. The companion
//! only reads the raw values; every rule is here so it is tested without Windows.

use std::time::Duration;

use crate::geometry::Rect;

/// The engine's idle poll (architecture 4.2: every 5 s).
pub const TICK_EVERY: Duration = Duration::from_secs(5);
/// How often presentation and full-screen state is sampled. Faster than the tick so the
/// pet leaves a starting slide show within about two seconds.
pub const PRESENCE_EVERY: Duration = Duration::from_secs(1);
/// Consecutive equal samples needed before a change is reported, so switching windows
/// past a full-screen app does not flash the pet away and back.
pub const PRESENCE_CONFIRM: u8 = 2;

/// Time since the last keyboard or mouse input, from `GetTickCount` and
/// `LASTINPUTINFO.dwTime`. Both are 32-bit millisecond counters that wrap after about
/// 49.7 days, so the difference is taken with wrap-around.
#[must_use]
pub const fn idle_from_ticks(now_ms: u32, last_input_ms: u32) -> Duration {
    Duration::from_millis(now_ms.wrapping_sub(last_input_ms) as u64)
}

/// `SHQueryUserNotificationState` results (`QUERY_USER_NOTIFICATION_STATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationState {
    /// `QUNS_NOT_PRESENT` (1): a screen saver or the lock screen; the user is away.
    NotPresent,
    /// `QUNS_BUSY` (2): a full-screen app is running or Presentation Settings are on.
    Busy,
    /// `QUNS_RUNNING_D3D_FULL_SCREEN` (3): a full-screen Direct3D app (a game or video).
    D3dFullScreen,
    /// `QUNS_PRESENTATION_MODE` (4): Windows presentation mode.
    PresentationMode,
    /// `QUNS_ACCEPTS_NOTIFICATIONS` (5): normal.
    AcceptsNotifications,
    /// `QUNS_QUIET_TIME` (6): the first hour after a new user signs in.
    QuietTime,
    /// `QUNS_APP` (7): a Windows Store app is running full screen.
    App,
    /// Anything else, or the call failed.
    Unknown,
}

impl NotificationState {
    /// From the raw value.
    #[must_use]
    pub const fn from_raw(v: i32) -> Self {
        match v {
            1 => Self::NotPresent,
            2 => Self::Busy,
            3 => Self::D3dFullScreen,
            4 => Self::PresentationMode,
            5 => Self::AcceptsNotifications,
            6 => Self::QuietTime,
            7 => Self::App,
            _ => Self::Unknown,
        }
    }

    /// Whether this state means a presentation or a full-screen app.
    #[must_use]
    pub const fn is_presenting(self) -> bool {
        matches!(
            self,
            Self::Busy | Self::D3dFullScreen | Self::PresentationMode | Self::App
        )
    }
}

/// What the companion read about the foreground window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundWindow {
    /// Its rectangle, physical pixels (`GetWindowRect`).
    pub rect: Rect,
    /// The full rectangle of the monitor it is on (`MONITORINFO.rcMonitor`).
    pub monitor: Rect,
    /// It has a title bar (`WS_CAPTION`). A maximised window with the taskbar set to
    /// auto-hide covers its monitor too, but keeps its title bar.
    pub has_caption: bool,
    /// It is visible and not minimised.
    pub shown: bool,
    /// It is the desktop or shell window (`GetShellWindow`, `GetDesktopWindow`, or the
    /// `Progman` and `WorkerW` desktop classes), which always covers the monitor.
    pub desktop: bool,
    /// It belongs to this app.
    pub own: bool,
}

/// Whether the foreground window is a full-screen app: shown, not the desktop, not ours,
/// without a title bar, and covering its whole monitor (the taskbar included).
#[must_use]
pub const fn is_fullscreen(w: &ForegroundWindow) -> bool {
    w.shown && !w.desktop && !w.own && !w.has_caption && w.rect.contains(&w.monitor)
}

/// Whether one sample means "presenting": the notification state says so, or the
/// foreground window is full screen.
#[must_use]
pub fn presenting(state: NotificationState, foreground: Option<&ForegroundWindow>) -> bool {
    state.is_presenting() || foreground.is_some_and(is_fullscreen)
}

/// Turns raw presentation samples into changes for the engine.
///
/// A change is reported only after [`PRESENCE_CONFIRM`] equal samples in a row. While the
/// session is locked or the PC asleep, samples are ignored (the lock screen is itself a
/// full-screen window) and the last reported state is kept; after the user returns, the
/// count starts again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresenceTracker {
    reported: bool,
    candidate: Option<(bool, u8)>,
    away: bool,
}

impl PresenceTracker {
    /// Not presenting, user present.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reported: false,
            candidate: None,
            away: false,
        }
    }

    /// The last state reported to the engine.
    #[must_use]
    pub const fn reported(&self) -> bool {
        self.reported
    }

    /// The session was locked or the PC went to sleep (`true`), or the user is back.
    pub const fn set_away(&mut self, away: bool) {
        self.away = away;
        self.candidate = None;
    }

    /// One sample. Returns the new state when it changes.
    pub fn observe(&mut self, presenting: bool) -> Option<bool> {
        if self.away {
            return None;
        }
        if presenting == self.reported {
            self.candidate = None;
            return None;
        }
        let n = match self.candidate {
            Some((v, n)) if v == presenting => n.saturating_add(1),
            _ => 1,
        };
        if n >= PRESENCE_CONFIRM {
            self.reported = presenting;
            self.candidate = None;
            Some(presenting)
        } else {
            self.candidate = Some((presenting, n));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_time_survives_the_tick_counter_wrapping() {
        assert_eq!(idle_from_ticks(10_000, 4_000), Duration::from_secs(6));
        assert_eq!(idle_from_ticks(5, 5), Duration::ZERO);
        // The counter wrapped between the last input and now.
        assert_eq!(
            idle_from_ticks(1_000, u32::MAX - 999),
            Duration::from_millis(2_000)
        );
    }

    #[test]
    fn notification_states() {
        let presenting: Vec<i32> = (0..10)
            .filter(|v| NotificationState::from_raw(*v).is_presenting())
            .collect();
        assert_eq!(presenting, [2, 3, 4, 7]);
        assert_eq!(
            NotificationState::from_raw(1),
            NotificationState::NotPresent
        );
        assert_eq!(NotificationState::from_raw(-3), NotificationState::Unknown);
    }

    fn window() -> ForegroundWindow {
        let monitor = Rect::new(0, 0, 1920, 1080);
        ForegroundWindow {
            rect: monitor,
            monitor,
            has_caption: false,
            shown: true,
            desktop: false,
            own: false,
        }
    }

    #[test]
    fn full_screen_window_rules() {
        assert!(is_fullscreen(&window()));
        // A borderless window larger than the monitor still covers it.
        let mut w = window();
        w.rect = Rect::new(-8, -8, 1936, 1096);
        assert!(is_fullscreen(&w));
        // A maximised window with an auto-hidden taskbar keeps its title bar.
        w.has_caption = true;
        assert!(!is_fullscreen(&w));
        // A maximised window above a visible taskbar does not cover the monitor.
        let mut w = window();
        w.rect = Rect::new(-8, -8, 1936, 1048);
        assert!(!is_fullscreen(&w));
        // The desktop, our own windows, and hidden or minimised windows never count.
        for f in [
            |w: &mut ForegroundWindow| w.desktop = true,
            |w: &mut ForegroundWindow| w.own = true,
            |w: &mut ForegroundWindow| w.shown = false,
        ] {
            let mut w = window();
            f(&mut w);
            assert!(!is_fullscreen(&w));
        }
        // On a second monitor to the left.
        let mut w = window();
        w.monitor = Rect::new(-1280, 0, 1280, 1024);
        w.rect = w.monitor;
        assert!(is_fullscreen(&w));
    }

    #[test]
    fn a_sample_presents_when_either_source_says_so() {
        let w = window();
        assert!(presenting(NotificationState::Busy, None));
        assert!(presenting(
            NotificationState::AcceptsNotifications,
            Some(&w)
        ));
        assert!(!presenting(NotificationState::AcceptsNotifications, None));
        let mut normal = w;
        normal.has_caption = true;
        assert!(!presenting(NotificationState::QuietTime, Some(&normal)));
    }

    #[test]
    fn changes_need_two_equal_samples() {
        let mut t = PresenceTracker::new();
        assert_eq!(t.observe(false), None);
        assert_eq!(t.observe(true), None);
        assert_eq!(t.observe(true), Some(true));
        assert!(t.reported());
        assert_eq!(t.observe(true), None);
        // A single sample the other way (switching past the full-screen app) is ignored.
        assert_eq!(t.observe(false), None);
        assert_eq!(t.observe(true), None);
        assert_eq!(t.observe(false), None);
        assert_eq!(t.observe(false), Some(false));
        assert!(!t.reported());
    }

    #[test]
    fn samples_while_away_are_ignored() {
        let mut t = PresenceTracker::new();
        t.set_away(true);
        // The lock screen covers the monitor: not a presentation.
        for _ in 0..5 {
            assert_eq!(t.observe(true), None);
        }
        assert!(!t.reported());
        t.set_away(false);
        assert_eq!(t.observe(true), None);
        assert_eq!(t.observe(true), Some(true));
        // Locking during a presentation keeps it reported until the user is back.
        t.set_away(true);
        assert_eq!(t.observe(false), None);
        assert_eq!(t.observe(false), None);
        assert!(t.reported());
        t.set_away(false);
        assert_eq!(t.observe(false), None);
        assert_eq!(t.observe(false), Some(false));
    }
}
