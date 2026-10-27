//! Where the pet window goes (architecture 4.2; wireframes P0, T1, W3 and 10.2).
//!
//! Every function here is pure: it takes the current monitors and returns a rectangle in
//! physical pixels. The companion calls them on start, after a drag, after a size change,
//! for "Bring pet above clock", and whenever the display layout changes (monitor added or
//! removed, resolution, scaling or taskbar change). Each result lies inside the work area
//! of one monitor, which the tests check for every size, scale and taskbar position.

use hy_core::schema::PetEdge;
use serde::{Deserialize, Serialize};

use crate::geometry::{Monitor, Rect, TaskbarEdge, to_physical};

/// The pet's art box at 100%, logical pixels (wireframe rule 1: about 128 px).
pub const PET_BASE_LOGICAL: f64 = 128.0;
/// Transparent room around the art for its shadow, badge and text tag, logical pixels.
pub const PET_PAD_LOGICAL: f64 = 8.0;
/// Gap between the pet window and the work-area edge at the home position, logical
/// pixels. With the padding the art sits about 12 px from the edge, as prototyped.
pub const HOME_MARGIN_LOGICAL: f64 = 4.0;
/// Gap between the pet and its bubble or panel, logical pixels.
pub const BUBBLE_GAP_LOGICAL: f64 = 6.0;
/// Widest bubble, logical pixels (wireframe section 6, scaling rule).
pub const BUBBLE_MAX_WIDTH_LOGICAL: f64 = 380.0;
/// Tallest bubble as a share of the work-area height (wireframe section 6).
pub const BUBBLE_MAX_HEIGHT_SHARE: f64 = 0.6;

/// Smallest pet size, percent (owner item 6; architecture 4.2). The approved floor was
/// 75%; the owner asked for smaller sizes after seeing the pet on an office PC, so it is
/// now 50% (the art is then 64 logical pixels, still above a 44 px touch-target minimum).
pub const SCALE_MIN: u16 = 50;
/// Largest pet size, percent.
pub const SCALE_MAX: u16 = 150;
/// The slider step, percent.
pub const SCALE_STEP: u16 = 5;
/// The default size, percent.
pub const SCALE_DEFAULT: u16 = 100;

/// A named size preset shown as S, M, L and XL beside the slider (quick picks; the
/// slider and the mouse wheel reach every size from [`SCALE_MIN`] to [`SCALE_MAX`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SizePreset {
    /// Percent.
    pub pct: u16,
    /// Short label (S, M, L, XL).
    pub short: &'static str,
    /// Full label for screen readers and tooltips.
    pub label: &'static str,
}

/// The four presets (wireframe W3 and the pet menu).
pub const SIZE_PRESETS: [SizePreset; 4] = [
    SizePreset {
        pct: 75,
        short: "S",
        label: "Small",
    },
    SizePreset {
        pct: 100,
        short: "M",
        label: "Medium",
    },
    SizePreset {
        pct: 125,
        short: "L",
        label: "Large",
    },
    SizePreset {
        pct: 150,
        short: "XL",
        label: "Extra large",
    },
];

/// Whether `pct` is an offered size: 50 to 150 in steps of 5.
#[must_use]
pub const fn is_offered_scale(pct: u16) -> bool {
    pct >= SCALE_MIN && pct <= SCALE_MAX && pct.is_multiple_of(SCALE_STEP)
}

/// The pet window's side length in logical pixels at `scale_pct`.
#[must_use]
pub fn pet_window_logical(scale_pct: u16) -> f64 {
    (PET_BASE_LOGICAL * f64::from(scale_pct) / 100.0).round() + 2.0 * PET_PAD_LOGICAL
}

/// The pet window's side length in physical pixels on `monitor`.
#[must_use]
pub fn pet_window_physical(scale_pct: u16, monitor: &Monitor) -> i32 {
    monitor.to_physical(pet_window_logical(scale_pct))
}

/// A placement: the window rectangle and the monitor it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Placement {
    /// The pet window, physical pixels.
    pub rect: Rect,
    /// Index into the monitor list passed in.
    pub monitor: usize,
    /// The taskbar side on that monitor (decides the "above or beside the clock" wording).
    pub taskbar: TaskbarEdge,
}

/// Keep `rect` inside `area`. A rectangle larger than the area is pinned to its top-left
/// corner (this cannot happen for the pet on any display Windows supports, but the rule
/// keeps the function total).
#[must_use]
pub fn clamp_into(rect: Rect, area: Rect) -> Rect {
    let x = if rect.width >= area.width {
        area.x
    } else {
        rect.x.clamp(area.x, area.right() - rect.width)
    };
    let y = if rect.height >= area.height {
        area.y
    } else {
        rect.y.clamp(area.y, area.bottom() - rect.height)
    };
    rect.at(x, y)
}

/// The primary monitor, or the first one if none is marked primary.
#[must_use]
pub fn primary_index(monitors: &[Monitor]) -> Option<usize> {
    monitors
        .iter()
        .position(|m| m.primary)
        .or(if monitors.is_empty() { None } else { Some(0) })
}

/// The monitor whose work area overlaps `rect` the most; if none overlaps, the monitor
/// whose bounds overlap it (the pet is over a taskbar); otherwise `None`.
#[must_use]
pub fn monitor_for(monitors: &[Monitor], rect: &Rect) -> Option<usize> {
    let best = |area: fn(&Monitor) -> Rect| {
        monitors
            .iter()
            .enumerate()
            .map(|(i, m)| (i, rect.intersection_area(&area(m))))
            .filter(|(_, a)| *a > 0)
            .max_by_key(|(_, a)| *a)
            .map(|(i, _)| i)
    };
    best(|m| m.work_area).or_else(|| best(|m| m.bounds))
}

/// The nearest monitor to `rect` by centre distance (used after a drop off every screen).
#[must_use]
pub fn nearest_monitor(monitors: &[Monitor], rect: &Rect) -> Option<usize> {
    monitors
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| m.work_area.center_distance_sq(rect))
        .map(|(i, _)| i)
}

/// The home position on `monitor`: the bottom-right corner of the work area. With the
/// taskbar at the bottom that is directly above the clock; with the taskbar on the right
/// it is beside the clock (owner item 7; wireframe 10.1 revision 3 notes).
#[must_use]
pub fn home_rect(monitor: &Monitor, scale_pct: u16) -> Rect {
    let side = pet_window_physical(scale_pct, monitor);
    let margin = monitor.to_physical(HOME_MARGIN_LOGICAL);
    let wa = monitor.work_area;
    clamp_into(
        Rect::new(
            wa.right() - margin - side,
            wa.bottom() - margin - side,
            side,
            side,
        ),
        wa,
    )
}

/// "Bring pet above clock" (tray, Settings and pet menu): the home position on the
/// primary monitor, where the Windows clock is, at the current size. It ignores where
/// the pet was, so it also recovers a pet that is off-screen or lost.
#[must_use]
pub fn bring_above_clock(monitors: &[Monitor], scale_pct: u16) -> Option<Placement> {
    let i = primary_index(monitors)?;
    let m = &monitors[i];
    Some(Placement {
        rect: home_rect(m, scale_pct),
        monitor: i,
        taskbar: m.taskbar_edge(),
    })
}

/// Snap `rect` to the chosen edge of `monitor`'s work area and keep it inside.
fn settle(monitor: &Monitor, rect: Rect, edge: PetEdge) -> Rect {
    let wa = monitor.work_area;
    let margin = monitor.to_physical(HOME_MARGIN_LOGICAL);
    let x = match edge {
        PetEdge::Left => wa.x + margin,
        PetEdge::Right => wa.right() - margin - rect.width,
        PetEdge::Free => rect.x,
    };
    clamp_into(rect.at(x, rect.y), wa)
}

/// Place the pet from a saved top-left corner (start-up, or after the display layout
/// changed). The size follows the scale of the monitor the pet is on.
///
/// - No saved position: the home position above the clock.
/// - The saved position is on no current monitor (it was unplugged, or the resolution
///   shrank past it): the home position above the clock (PET-04; wireframe P0).
/// - Otherwise: the same place, snapped to the chosen edge and kept inside that
///   monitor's work area.
#[must_use]
pub fn reconcile(
    monitors: &[Monitor],
    saved: Option<(i32, i32)>,
    scale_pct: u16,
    edge: PetEdge,
) -> Option<Placement> {
    let Some((x, y)) = saved else {
        return bring_above_clock(monitors, scale_pct);
    };
    // The size depends on the monitor and the monitor on the size, so test each monitor
    // with its own size and keep the one showing the most of the pet.
    let candidate = monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let side = pet_window_physical(scale_pct, m);
            let r = Rect::new(x, y, side, side);
            (
                i,
                r,
                r.intersection_area(&m.work_area),
                r.intersection_area(&m.bounds),
            )
        })
        .filter(|(_, _, _, on_bounds)| *on_bounds > 0)
        .max_by_key(|(_, _, on_work, on_bounds)| (*on_work, *on_bounds));
    match candidate {
        Some((i, r, _, _)) => {
            let m = &monitors[i];
            Some(Placement {
                rect: settle(m, r, edge),
                monitor: i,
                taskbar: m.taskbar_edge(),
            })
        }
        None => bring_above_clock(monitors, scale_pct),
    }
}

/// After a drag ends: pick the monitor under the pet (or the nearest one), resize for
/// its scale keeping the top-left corner, then snap. With edge Left or Right the pet
/// snaps to the nearer side edge and the edge setting follows it; with "where I drop
/// it" it stays where dropped. Either way it is kept inside the work area.
#[must_use]
pub fn snap_after_drop(
    monitors: &[Monitor],
    dropped: Rect,
    scale_pct: u16,
    edge: PetEdge,
) -> Option<(Placement, PetEdge)> {
    let i = monitor_for(monitors, &dropped).or_else(|| nearest_monitor(monitors, &dropped))?;
    let m = &monitors[i];
    let side = pet_window_physical(scale_pct, m);
    let r = Rect::new(dropped.x, dropped.y, side, side);
    let new_edge = match edge {
        PetEdge::Free => PetEdge::Free,
        PetEdge::Left | PetEdge::Right => {
            let (cx, _) = r.center();
            let (wcx, _) = m.work_area.center();
            if cx < wcx {
                PetEdge::Left
            } else {
                PetEdge::Right
            }
        }
    };
    Some((
        Placement {
            rect: settle(m, r, new_edge),
            monitor: i,
            taskbar: m.taskbar_edge(),
        },
        new_edge,
    ))
}

/// A size change takes effect at once without moving the pet outside the work area
/// (wireframe 10.2, item 6). The bottom edge stays put; the right edge stays put when
/// the pet is on the right half of its monitor, otherwise the left edge does.
#[must_use]
pub fn resize_anchored(
    monitors: &[Monitor],
    current: Rect,
    new_scale_pct: u16,
    edge: PetEdge,
) -> Option<Placement> {
    let Some(i) = monitor_for(monitors, &current) else {
        return bring_above_clock(monitors, new_scale_pct);
    };
    let m = &monitors[i];
    let side = pet_window_physical(new_scale_pct, m);
    let (cx, _) = current.center();
    let (wcx, _) = m.work_area.center();
    let x = if cx >= wcx {
        current.right() - side
    } else {
        current.x
    };
    let r = Rect::new(x, current.bottom() - side, side, side);
    Some(Placement {
        rect: settle(m, r, edge),
        monitor: i,
        taskbar: m.taskbar_edge(),
    })
}

/// Which side of the pet a bubble or panel opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// To the left of the pet (the pet is on the right half of the screen).
    Left,
    /// To the right of the pet.
    Right,
}

/// The largest bubble on `monitor`, logical pixels (width, height). The page caps its
/// bubble at this size and scrolls inside it.
#[must_use]
pub fn bubble_limits_logical(monitor: &Monitor, scale_pct: u16) -> (f64, f64) {
    let (ww, wh) = monitor.work_area_logical();
    let beside =
        ww - pet_window_logical(scale_pct) - BUBBLE_GAP_LOGICAL - 2.0 * HOME_MARGIN_LOGICAL;
    (
        BUBBLE_MAX_WIDTH_LOGICAL.min(beside).max(120.0),
        (wh * BUBBLE_MAX_HEIGHT_SHARE).max(80.0),
    )
}

/// Where a bubble (or the keyboard panel) goes next to the pet, inside the work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BubbleLayout {
    /// The window that holds both pet and bubble, physical pixels.
    pub window: Rect,
    /// The pet's rectangle, physical pixels (unchanged by the bubble).
    pub pet: Rect,
    /// The bubble's rectangle, physical pixels.
    pub bubble: Rect,
    /// The side the bubble opened on.
    pub side: Side,
}

/// Place a bubble of `size` (physical pixels) beside `pet` within `work_area`.
///
/// The bubble opens towards the middle of the screen, its bottom level with the pet's
/// bottom, and is then kept inside the work area. The pet does not move, so the window
/// becomes the union of the two rectangles, which is also inside the work area.
#[must_use]
pub fn layout_bubble(work_area: Rect, pet: Rect, size: (i32, i32), gap: i32) -> BubbleLayout {
    let pet = clamp_into(pet, work_area);
    let width = size.0.clamp(1, work_area.width.max(1));
    let height = size.1.clamp(1, work_area.height.max(1));
    let (pcx, _) = pet.center();
    let (wcx, _) = work_area.center();
    let side = if pcx >= wcx { Side::Left } else { Side::Right };
    let x = match side {
        Side::Left => pet.x - gap - width,
        Side::Right => pet.right() + gap,
    };
    let bubble = clamp_into(
        Rect::new(x, pet.bottom() - height, width, height),
        work_area,
    );
    BubbleLayout {
        window: pet.union(&bubble),
        pet,
        bubble,
        side,
    }
}

/// The settings window size in logical pixels: a comfortable 900 x 640, at most 90% of
/// the work area, at least 640 x 440 unless the work area itself is smaller (wireframe
/// section 6: 1366 x 768 at 150% gives a logical work area of about 910 x 480).
#[must_use]
pub fn screens_window_logical(monitor: &Monitor) -> (f64, f64) {
    let (ww, wh) = monitor.work_area_logical();
    let fit = |preferred: f64, minimum: f64, available: f64| {
        preferred.min(available * 0.9).max(minimum).min(available)
    };
    (fit(900.0, 640.0, ww), fit(640.0, 440.0, wh))
}

/// Physical pixels of a logical gap on `monitor`.
#[must_use]
pub fn gap_physical(monitor: &Monitor) -> i32 {
    to_physical(BUBBLE_GAP_LOGICAL, monitor.safe_scale())
}

#[cfg(test)]
mod tests;
