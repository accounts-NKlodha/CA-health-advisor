//! Desktop geometry in physical pixels (architecture 4.2: per-monitor DPI).
//!
//! Windows reports monitor bounds and work areas in physical pixels on one virtual
//! desktop, where a secondary monitor can have negative coordinates. Each monitor has
//! its own scale factor (1.0 = 100%, 1.25 = 125%, 1.5 = 150%, 2.0 = 200%). The pet's
//! size is defined in logical pixels and converted with the scale of the monitor it is
//! on, so it looks the same size on every monitor.

use serde::{Deserialize, Serialize};

/// A rectangle in physical pixels. `width` and `height` are never negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

impl Rect {
    /// A rectangle; negative sizes become zero.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: if width < 0 { 0 } else { width },
            height: if height < 0 { 0 } else { height },
        }
    }

    /// One past the right edge.
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    /// One past the bottom edge.
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    /// The centre, rounded down.
    #[must_use]
    pub const fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    /// The overlapping area with `other` in square pixels (0 when they do not overlap).
    #[must_use]
    pub fn intersection_area(&self, other: &Self) -> i64 {
        let w = i64::from(self.right().min(other.right())) - i64::from(self.x.max(other.x));
        let h = i64::from(self.bottom().min(other.bottom())) - i64::from(self.y.max(other.y));
        if w <= 0 || h <= 0 { 0 } else { w * h }
    }

    /// Whether `other` lies completely inside this rectangle.
    #[must_use]
    pub const fn contains(&self, other: &Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// Whether the point lies inside this rectangle.
    #[must_use]
    pub const fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The smallest rectangle holding both.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    /// The same size at a new top-left corner.
    #[must_use]
    pub const fn at(&self, x: i32, y: i32) -> Self {
        Self::new(x, y, self.width, self.height)
    }

    /// Squared distance between the centres (used to find the nearest monitor).
    #[must_use]
    pub fn center_distance_sq(&self, other: &Self) -> i64 {
        let (ax, ay) = self.center();
        let (bx, by) = other.center();
        let dx = i64::from(ax) - i64::from(bx);
        let dy = i64::from(ay) - i64::from(by);
        dx * dx + dy * dy
    }
}

/// Where the Windows taskbar sits on a monitor, worked out from the gap between the
/// monitor's bounds and its work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskbarEdge {
    /// Along the bottom (the Windows default; the clock is at the bottom-right).
    Bottom,
    /// Along the right edge (the clock is at the bottom of the bar).
    Right,
    /// Along the top (the clock is at the top-right).
    Top,
    /// Along the left edge (the clock is at the bottom of the bar).
    Left,
    /// No taskbar takes space here: auto-hide is on, or the taskbar is on another monitor.
    Hidden,
}

/// One display as Windows reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Monitor {
    /// The device name (for example `\\.\DISPLAY1`). Used only to notice layout changes.
    pub id: String,
    /// The whole monitor, physical pixels.
    pub bounds: Rect,
    /// The monitor minus the taskbar and docked bars, physical pixels.
    pub work_area: Rect,
    /// The scale factor (1.0 = 100%).
    pub scale: f64,
    /// Whether this is the primary monitor (the one with the main taskbar and clock).
    pub primary: bool,
}

/// A work-area gap at or below this many physical pixels is not a taskbar (auto-hide
/// taskbars can keep a sliver of 1 or 2 pixels).
const TASKBAR_MIN_GAP: i32 = 4;

impl Monitor {
    /// The side the taskbar is on.
    #[must_use]
    pub fn taskbar_edge(&self) -> TaskbarEdge {
        let b = &self.bounds;
        let w = &self.work_area;
        let gaps = [
            (b.bottom() - w.bottom(), TaskbarEdge::Bottom),
            (b.right() - w.right(), TaskbarEdge::Right),
            (w.y - b.y, TaskbarEdge::Top),
            (w.x - b.x, TaskbarEdge::Left),
        ];
        // `max_by_key` keeps the last maximum; iterate in reverse so a tie prefers the
        // order above (bottom first).
        gaps.iter()
            .rev()
            .filter(|(gap, _)| *gap > TASKBAR_MIN_GAP)
            .max_by_key(|(gap, _)| *gap)
            .map_or(TaskbarEdge::Hidden, |(_, edge)| *edge)
    }

    /// A usable scale factor: Windows supports 100% to 500%; anything else is treated
    /// as 100% so a bad report can never produce a zero or huge pet.
    #[must_use]
    pub fn safe_scale(&self) -> f64 {
        if self.scale.is_finite() && (0.5..=5.0).contains(&self.scale) {
            self.scale
        } else {
            1.0
        }
    }

    /// Logical pixels to physical pixels on this monitor, rounded.
    #[must_use]
    pub fn to_physical(&self, logical: f64) -> i32 {
        to_physical(logical, self.safe_scale())
    }

    /// The work area in logical pixels (width, height).
    #[must_use]
    pub fn work_area_logical(&self) -> (f64, f64) {
        let s = self.safe_scale();
        (
            f64::from(self.work_area.width) / s,
            f64::from(self.work_area.height) / s,
        )
    }
}

/// Logical pixels to physical pixels at `scale`, rounded to the nearest pixel.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // pet and window sizes are far below i32::MAX
pub fn to_physical(logical: f64, scale: f64) -> i32 {
    (logical * scale).round() as i32
}

/// A fingerprint of the monitor layout. When it changes (a monitor is added or removed,
/// the resolution, scaling or taskbar changes), the pet is placed again.
#[must_use]
pub fn layout_fingerprint(monitors: &[Monitor]) -> String {
    let mut parts: Vec<String> = monitors
        .iter()
        .map(|m| {
            format!(
                "{}:{:?}:{:?}:{:.3}:{}",
                m.id, m.bounds, m.work_area, m.scale, m.primary
            )
        })
        .collect();
    parts.sort();
    parts.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(bounds: Rect, work: Rect) -> Monitor {
        Monitor {
            id: "m".into(),
            bounds,
            work_area: work,
            scale: 1.0,
            primary: true,
        }
    }

    #[test]
    fn rect_basics() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersection_area(&b), 25);
        assert_eq!(a.intersection_area(&Rect::new(10, 0, 5, 5)), 0);
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert!(a.contains(&Rect::new(1, 1, 9, 9)));
        assert!(!a.contains(&b));
        assert!(a.contains_point(9, 9));
        assert!(!a.contains_point(10, 9));
        assert_eq!(Rect::new(0, 0, -3, 4).width, 0);
    }

    #[test]
    fn taskbar_edges() {
        let b = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            mon(b, Rect::new(0, 0, 1920, 1032)).taskbar_edge(),
            TaskbarEdge::Bottom
        );
        assert_eq!(
            mon(b, Rect::new(0, 0, 1858, 1080)).taskbar_edge(),
            TaskbarEdge::Right
        );
        assert_eq!(
            mon(b, Rect::new(0, 48, 1920, 1032)).taskbar_edge(),
            TaskbarEdge::Top
        );
        assert_eq!(
            mon(b, Rect::new(62, 0, 1858, 1080)).taskbar_edge(),
            TaskbarEdge::Left
        );
        assert_eq!(mon(b, b).taskbar_edge(), TaskbarEdge::Hidden);
        // Auto-hide sliver.
        assert_eq!(
            mon(b, Rect::new(0, 0, 1920, 1078)).taskbar_edge(),
            TaskbarEdge::Hidden
        );
    }

    #[test]
    fn taskbar_edge_on_a_monitor_left_of_the_primary() {
        let b = Rect::new(-1920, 0, 1920, 1080);
        assert_eq!(
            mon(b, Rect::new(-1920, 0, 1920, 1032)).taskbar_edge(),
            TaskbarEdge::Bottom
        );
        assert_eq!(
            mon(b, Rect::new(-1920, 0, 1858, 1080)).taskbar_edge(),
            TaskbarEdge::Right
        );
    }

    #[test]
    fn bad_scale_falls_back_to_100_percent() {
        let mut m = mon(Rect::new(0, 0, 10, 10), Rect::new(0, 0, 10, 10));
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 9.0] {
            m.scale = bad;
            assert!((m.safe_scale() - 1.0).abs() < f64::EPSILON);
        }
        m.scale = 1.25;
        assert_eq!(m.to_physical(128.0), 160);
    }

    #[test]
    fn fingerprint_ignores_order_and_sees_changes() {
        let a = mon(Rect::new(0, 0, 10, 10), Rect::new(0, 0, 10, 8));
        let mut b = a.clone();
        b.id = "n".into();
        b.primary = false;
        assert_eq!(
            layout_fingerprint(&[a.clone(), b.clone()]),
            layout_fingerprint(&[b.clone(), a.clone()])
        );
        let mut a2 = a.clone();
        a2.work_area = Rect::new(0, 0, 10, 9);
        assert_ne!(layout_fingerprint(&[a]), layout_fingerprint(&[a2]));
    }
}
