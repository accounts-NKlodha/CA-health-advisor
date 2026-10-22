//! The pet window's frame: where the pet is, which monitor it is on, what is open beside
//! it, the bubble's measured size and layout, and the rectangle the companion must give
//! the real window.
//!
//! Every change of the pet's placement goes through [`ShellCore::place`], which lays out
//! the open bubble or menu again for the new spot (or, when the monitor's scaling or work
//! area changed, drops the old measurement so the page measures again). The rectangle
//! the companion passes to the window-position call comes only from
//! [`ShellCore::frame_for_render`], so it is always derived from the current pet and
//! monitor and lies inside the current work area. Nothing here activates a window.

use crate::geometry::{Monitor, Rect};
use crate::placement::{
    BubbleLayout, Placement, bring_above_clock, bubble_limits_logical, gap_physical, layout_bubble,
    reconcile,
};
use crate::shell::{Effect, Open, ShellModel};
use hy_core::schema::PetEdge;

/// A key for what is open: it changes whenever something new opens.
#[must_use]
pub fn open_key(open: &Open) -> String {
    match open {
        Open::None => String::new(),
        Open::Bubble { bubble } => format!("bubble:{}", bubble.id),
        Open::Notice { bubble } => format!("notice:{}", bubble.id),
        Open::Menu => "menu".to_owned(),
    }
}

/// The window rectangles for one render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderFrame {
    /// The window rectangle the view is laid out against.
    pub window: Option<Rect>,
    /// The rectangle to pass to the window-position call now, if it changed.
    pub apply: Option<Rect>,
}

/// The shell's rules ([`ShellModel`]) plus the window frame, without windows.
#[derive(Debug, Clone)]
pub struct ShellCore {
    /// What is shown, hidden and open.
    pub model: ShellModel,
    pet: Option<Rect>,
    monitor: Option<Monitor>,
    open_token: u64,
    open_key: String,
    /// The measured bubble size for `open_token`, logical pixels.
    measured: Option<(f64, f64)>,
    layout: Option<BubbleLayout>,
    /// The rectangle last passed to the window-position call.
    applied: Option<Rect>,
    /// The pet's size, percent (bounds the bubble).
    scale_pct: u16,
}

impl ShellCore {
    /// Nothing placed yet.
    #[must_use]
    pub const fn new(scale_pct: u16) -> Self {
        Self {
            model: ShellModel::new(),
            pet: None,
            monitor: None,
            open_token: 0,
            open_key: String::new(),
            measured: None,
            layout: None,
            applied: None,
            scale_pct,
        }
    }

    /// The pet's rectangle, physical pixels.
    #[must_use]
    pub const fn pet(&self) -> Option<Rect> {
        self.pet
    }

    /// The monitor the pet is on.
    #[must_use]
    pub const fn monitor(&self) -> Option<&Monitor> {
        self.monitor.as_ref()
    }

    /// The token of what is open now (measurements carry it back).
    #[must_use]
    pub const fn open_token(&self) -> u64 {
        self.open_token
    }

    /// The bubble layout, once the open bubble or menu is measured.
    #[must_use]
    pub fn layout(&self) -> Option<BubbleLayout> {
        self.layout.filter(|_| self.measured.is_some())
    }

    /// The rectangle last passed to the window-position call.
    #[must_use]
    pub const fn applied(&self) -> Option<Rect> {
        self.applied
    }

    /// Place the pet. The open bubble or menu is laid out again beside the new spot; if
    /// the monitor's scaling or work area differs from the one it was measured on, the
    /// measurement is dropped and a new token makes the page measure again. The window
    /// is positioned again on the next render even if the rectangle looks unchanged
    /// (Windows may have resized it on a scaling change).
    pub fn place(&mut self, p: Placement, monitors: &[Monitor], scale_pct: u16) {
        let monitor = monitors.get(p.monitor).cloned();
        let same_geometry = match (&self.monitor, &monitor) {
            (Some(a), Some(b)) => {
                a.work_area == b.work_area && a.safe_scale().to_bits() == b.safe_scale().to_bits()
            }
            _ => false,
        };
        self.pet = Some(p.rect);
        self.monitor = monitor;
        self.scale_pct = scale_pct;
        if !same_geometry && self.measured.is_some() {
            self.remeasure();
        }
        self.relayout();
        self.applied = None;
    }

    /// Drop the measurement and ask the page to measure again.
    fn remeasure(&mut self) {
        self.measured = None;
        self.open_token += 1;
    }

    fn relayout(&mut self) {
        self.layout = match (self.measured, self.monitor.as_ref(), self.pet) {
            (Some((w, h)), Some(m), Some(pet)) => {
                let (max_w, max_h) = bubble_limits_logical(m, self.scale_pct);
                let size = (m.to_physical(w.min(max_w)), m.to_physical(h.min(max_h)));
                Some(layout_bubble(m.work_area, pet, size, gap_physical(m)))
            }
            _ => None,
        };
    }

    /// The page measured the open bubble or menu (logical pixels) for `token`. Returns
    /// whether the measurement was taken (stale or invalid ones are ignored).
    pub fn bubble_measured(&mut self, token: u64, width: f64, height: f64) -> bool {
        if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
            return false;
        }
        self.sync_open();
        if token != self.open_token || matches!(self.model.open(), Open::None) {
            return false;
        }
        let (max_w, max_h) = self.monitor.as_ref().map_or((width, height), |m| {
            bubble_limits_logical(m, self.scale_pct)
        });
        self.measured = Some((width.min(max_w), height.min(max_h)));
        self.relayout();
        true
    }

    /// Start a new token when something new opened.
    fn sync_open(&mut self) {
        let key = open_key(self.model.open());
        if key != self.open_key {
            self.open_key = key;
            self.open_token += 1;
            self.measured = None;
            self.layout = None;
        }
    }

    /// The rectangles for a render: the window the view is laid out against, and the
    /// rectangle to pass to the window-position call (none while dragging, when the
    /// drag moves the window itself, or when it is unchanged).
    pub fn frame_for_render(&mut self, dragging: bool) -> RenderFrame {
        self.sync_open();
        let window = match (self.model.open(), self.layout(), self.pet) {
            (Open::None, _, Some(p)) | (_, None, Some(p)) => Some(p),
            (_, Some(l), _) => Some(l.window),
            _ => None,
        };
        let apply = match window {
            Some(r) if !dragging && self.applied != Some(r) => {
                self.applied = Some(r);
                Some(r)
            }
            _ => None,
        };
        RenderFrame { window, apply }
    }

    /// The display layout changed (monitor added or removed, resolution, scaling or
    /// taskbar): place the pet again from where it is (or its saved spot). Returns the
    /// placement, which is not saved (an automatic move).
    pub fn display_changed(
        &mut self,
        monitors: &[Monitor],
        saved: Option<(i32, i32)>,
        scale_pct: u16,
        edge: PetEdge,
    ) -> Option<Placement> {
        let at = self.pet.map(|r| (r.x, r.y)).or(saved);
        let p = reconcile(monitors, at, scale_pct, edge)?;
        self.place(p, monitors, scale_pct);
        Some(p)
    }

    /// "Bring pet above clock": close the menu, clear the user's hiding and place the
    /// pet at home. An open engine bubble stays open and is laid out beside the new spot.
    pub fn bring_home(
        &mut self,
        monitors: &[Monitor],
        scale_pct: u16,
    ) -> (Vec<Effect>, Option<Placement>) {
        let mut fx = self.model.close_menu();
        fx.extend(self.model.bring_back());
        let p = bring_above_clock(monitors, scale_pct);
        if let Some(p) = p {
            self.place(p, monitors, scale_pct);
        }
        (fx, p)
    }

    /// Start a drag: whatever is open closes, so the window is the pet alone.
    pub fn drag_begin_move(&mut self) -> Vec<Effect> {
        if matches!(self.model.open(), Open::None) {
            return Vec::new();
        }
        self.measured = None;
        self.layout = None;
        self.model.user_close()
    }

    /// The pet follows the pointer during a drag. Returns the rectangle the companion
    /// moves the window to (the drag moves the window directly, not through a render)
    /// and whether its size differs from the window's (so it must be resized too).
    pub fn drag_to(&mut self, x: i32, y: i32) -> Option<(Rect, bool)> {
        let r = self.pet?.at(x, y);
        let resize = self
            .applied
            .is_some_and(|a| a.width != r.width || a.height != r.height);
        self.pet = Some(r);
        self.applied = Some(r);
        Some((r, resize))
    }
}

#[cfg(test)]
mod tests;
