//! Placement tests: every offered size, Windows scaling at 100, 125, 150 and 200%,
//! the taskbar on each side (and auto-hidden), several resolutions, two monitors with
//! negative coordinates, and monitor removal.

use super::*;
use proptest::prelude::*;

const SCALES: [f64; 4] = [1.0, 1.25, 1.5, 2.0];
const EDGES: [TaskbarEdge; 5] = [
    TaskbarEdge::Bottom,
    TaskbarEdge::Right,
    TaskbarEdge::Top,
    TaskbarEdge::Left,
    TaskbarEdge::Hidden,
];
/// Physical resolutions seen in small offices.
const RESOLUTIONS: [(i32, i32); 4] = [(1366, 768), (1920, 1080), (2560, 1440), (3840, 2160)];

fn sizes() -> impl Iterator<Item = u16> {
    (SCALE_MIN..=SCALE_MAX).step_by(usize::from(SCALE_STEP))
}

/// A monitor at physical origin (ox, oy) with a 48-logical-pixel taskbar on `edge`.
fn monitor(
    id: &str,
    origin: (i32, i32),
    res: (i32, i32),
    scale: f64,
    edge: TaskbarEdge,
    primary: bool,
) -> Monitor {
    let bounds = Rect::new(origin.0, origin.1, res.0, res.1);
    let bar = to_physical(48.0, scale);
    let work = match edge {
        TaskbarEdge::Bottom => Rect::new(bounds.x, bounds.y, bounds.width, bounds.height - bar),
        TaskbarEdge::Top => Rect::new(bounds.x, bounds.y + bar, bounds.width, bounds.height - bar),
        TaskbarEdge::Left => Rect::new(bounds.x + bar, bounds.y, bounds.width - bar, bounds.height),
        TaskbarEdge::Right => Rect::new(bounds.x, bounds.y, bounds.width - bar, bounds.height),
        TaskbarEdge::Hidden => bounds,
    };
    Monitor {
        id: id.into(),
        bounds,
        work_area: work,
        scale,
        primary,
    }
}

fn single(res: (i32, i32), scale: f64, edge: TaskbarEdge) -> Vec<Monitor> {
    vec![monitor("\\\\.\\DISPLAY1", (0, 0), res, scale, edge, true)]
}

fn assert_inside(monitors: &[Monitor], p: &Placement) {
    let wa = monitors[p.monitor].work_area;
    assert!(
        wa.contains(&p.rect),
        "pet {:?} outside work area {:?}",
        p.rect,
        wa
    );
}

#[test]
fn offered_sizes_are_75_to_150_in_steps_of_5() {
    assert!(is_offered_scale(75) && is_offered_scale(100) && is_offered_scale(150));
    assert!(!is_offered_scale(70) && !is_offered_scale(155) && !is_offered_scale(101));
    assert_eq!(sizes().count(), 16);
    assert!(SIZE_PRESETS.iter().all(|p| is_offered_scale(p.pct)));
    assert_eq!(
        SIZE_PRESETS.map(|p| p.short),
        ["S", "M", "L", "XL"],
        "presets in order"
    );
}

#[test]
fn pet_window_size_follows_size_and_dpi() {
    assert!((pet_window_logical(100) - 144.0).abs() < f64::EPSILON);
    assert!((pet_window_logical(75) - 112.0).abs() < f64::EPSILON);
    assert!((pet_window_logical(150) - 208.0).abs() < f64::EPSILON);
    let m = |s| monitor("m", (0, 0), (1920, 1080), s, TaskbarEdge::Bottom, true);
    assert_eq!(pet_window_physical(100, &m(1.0)), 144);
    assert_eq!(pet_window_physical(100, &m(1.25)), 180);
    assert_eq!(pet_window_physical(100, &m(1.5)), 216);
    assert_eq!(pet_window_physical(100, &m(2.0)), 288);
    assert_eq!(pet_window_physical(150, &m(2.0)), 416);
}

/// Owner item 7 at every size, scale, taskbar side and resolution.
#[test]
fn bring_above_clock_is_bottom_right_inside_the_work_area_everywhere() {
    for res in RESOLUTIONS {
        for scale in SCALES {
            for edge in EDGES {
                let ms = single(res, scale, edge);
                let m = &ms[0];
                let margin = m.to_physical(HOME_MARGIN_LOGICAL);
                for pct in sizes() {
                    let p = bring_above_clock(&ms, pct).expect("a monitor");
                    assert_inside(&ms, &p);
                    assert_eq!(p.taskbar, edge);
                    let side = pet_window_physical(pct, m);
                    assert_eq!((p.rect.width, p.rect.height), (side, side));
                    assert_eq!(p.rect.right(), m.work_area.right() - margin);
                    assert_eq!(p.rect.bottom(), m.work_area.bottom() - margin);
                    match edge {
                        // Directly above the clock: touching the taskbar's top edge
                        // region and at the right of the screen.
                        TaskbarEdge::Bottom => {
                            assert!(p.rect.bottom() <= m.work_area.bottom());
                            assert!(m.bounds.right() - p.rect.right() <= margin);
                        }
                        // Beside the clock, which sits at the bottom of a right taskbar.
                        TaskbarEdge::Right => {
                            assert!(p.rect.right() <= m.work_area.right());
                            assert!(m.bounds.bottom() - p.rect.bottom() <= margin);
                        }
                        TaskbarEdge::Top | TaskbarEdge::Left | TaskbarEdge::Hidden => {}
                    }
                }
            }
        }
    }
}

#[test]
fn bring_above_clock_uses_the_primary_monitor_even_when_listed_second() {
    let left = monitor(
        "\\\\.\\DISPLAY2",
        (-1920, 0),
        (1920, 1080),
        1.0,
        TaskbarEdge::Bottom,
        false,
    );
    let primary = monitor(
        "\\\\.\\DISPLAY1",
        (0, 0),
        (2560, 1440),
        1.5,
        TaskbarEdge::Bottom,
        true,
    );
    let ms = vec![left, primary];
    let p = bring_above_clock(&ms, 125).expect("monitor");
    assert_eq!(p.monitor, 1);
    assert_inside(&ms, &p);
    // No monitor marked primary: the first one is used.
    let mut none = ms.clone();
    none.iter_mut().for_each(|m| m.primary = false);
    assert_eq!(bring_above_clock(&none, 100).expect("monitor").monitor, 0);
    assert!(bring_above_clock(&[], 100).is_none());
}

#[test]
fn start_without_a_saved_position_goes_above_the_clock() {
    let ms = single((1920, 1080), 1.25, TaskbarEdge::Bottom);
    assert_eq!(
        reconcile(&ms, None, 100, PetEdge::Right),
        bring_above_clock(&ms, 100)
    );
}

#[test]
fn saved_position_is_kept_and_snapped_to_the_edge() {
    let ms = single((1920, 1080), 1.0, TaskbarEdge::Bottom);
    let free = reconcile(&ms, Some((700, 300)), 100, PetEdge::Free).expect("placed");
    assert_eq!((free.rect.x, free.rect.y), (700, 300));
    let right = reconcile(&ms, Some((700, 300)), 100, PetEdge::Right).expect("placed");
    assert_eq!(right.rect.right(), 1920 - 4);
    assert_eq!(right.rect.y, 300);
    let left = reconcile(&ms, Some((700, 300)), 100, PetEdge::Left).expect("placed");
    assert_eq!(left.rect.x, 4);
}

/// PET-04: the monitor the pet was on is unplugged.
#[test]
fn monitor_removed_returns_the_pet_above_the_clock_on_the_remaining_monitor() {
    for scale in SCALES {
        for pct in sizes() {
            let primary = monitor("P", (0, 0), (1920, 1080), scale, TaskbarEdge::Bottom, true);
            let second = monitor(
                "S",
                (1920, 0),
                (1920, 1080),
                1.0,
                TaskbarEdge::Bottom,
                false,
            );
            let both = vec![primary.clone(), second];
            let on_second = reconcile(&both, Some((2500, 400)), pct, PetEdge::Free).expect("p");
            assert_eq!(on_second.monitor, 1);
            let saved = (on_second.rect.x, on_second.rect.y);
            let only = vec![primary];
            let after = reconcile(&only, Some(saved), pct, PetEdge::Free).expect("p");
            assert_eq!(Some(after), bring_above_clock(&only, pct));
            assert_inside(&only, &after);
        }
    }
}

#[test]
fn monitor_on_the_left_with_negative_coordinates_removed() {
    let left = monitor(
        "L",
        (-1366, 200),
        (1366, 768),
        1.0,
        TaskbarEdge::Bottom,
        false,
    );
    let primary = monitor("P", (0, 0), (1920, 1080), 1.25, TaskbarEdge::Bottom, true);
    let both = vec![left, primary.clone()];
    let p = reconcile(&both, Some((-300, 500)), 100, PetEdge::Free).expect("p");
    assert_eq!(p.monitor, 0);
    assert_inside(&both, &p);
    let only = vec![primary];
    let after = reconcile(&only, Some((p.rect.x, p.rect.y)), 100, PetEdge::Free).expect("p");
    assert_eq!(Some(after), bring_above_clock(&only, 100));
}

#[test]
fn resolution_shrink_clamps_a_partly_visible_pet_and_rehomes_an_invisible_one() {
    let small = single((1366, 768), 1.0, TaskbarEdge::Bottom);
    // Partly visible after the change: pulled inside.
    let p = reconcile(&small, Some((1300, 700)), 100, PetEdge::Free).expect("p");
    assert_inside(&small, &p);
    assert_eq!(p.rect.right(), 1366);
    // Wholly beyond the new bounds: home above the clock.
    let q = reconcile(&small, Some((1800, 900)), 100, PetEdge::Free).expect("p");
    assert_eq!(Some(q), bring_above_clock(&small, 100));
}

#[test]
fn scaling_change_on_the_same_monitor_resizes_and_stays_inside() {
    for edge in EDGES {
        for pct in sizes() {
            let before = single((1920, 1080), 1.0, edge);
            let home = bring_above_clock(&before, pct).expect("p");
            for scale in SCALES {
                let after = single((1920, 1080), scale, edge);
                let p = reconcile(
                    &after,
                    Some((home.rect.x, home.rect.y)),
                    pct,
                    PetEdge::Right,
                )
                .expect("p");
                assert_inside(&after, &p);
                assert_eq!(p.rect.width, pet_window_physical(pct, &after[0]));
            }
        }
    }
}

#[test]
fn taskbar_moved_keeps_the_pet_inside_the_new_work_area() {
    for from in EDGES {
        for to in EDGES {
            for scale in SCALES {
                let before = single((1920, 1080), scale, from);
                let home = bring_above_clock(&before, 150).expect("p");
                let after = single((1920, 1080), scale, to);
                for edge in [PetEdge::Left, PetEdge::Right, PetEdge::Free] {
                    let p =
                        reconcile(&after, Some((home.rect.x, home.rect.y)), 150, edge).expect("p");
                    assert_inside(&after, &p);
                }
            }
        }
    }
}

#[test]
fn a_pet_left_over_the_taskbar_is_pulled_into_the_work_area() {
    let ms = single((1920, 1080), 1.0, TaskbarEdge::Bottom);
    // Entirely over the 48 px taskbar strip.
    let p = reconcile(&ms, Some((800, 1040)), 75, PetEdge::Free).expect("p");
    assert_eq!(p.monitor, 0);
    assert_inside(&ms, &p);
}

#[test]
fn drop_snaps_to_the_nearer_side_edge_or_stays_free() {
    let ms = single((1920, 1080), 1.5, TaskbarEdge::Bottom);
    let side = pet_window_physical(100, &ms[0]);
    let near_left = Rect::new(500, 300, side, side);
    let (p, e) = snap_after_drop(&ms, near_left, 100, PetEdge::Right).expect("p");
    assert_eq!(e, PetEdge::Left);
    assert_eq!(p.rect.x, ms[0].to_physical(HOME_MARGIN_LOGICAL));
    assert_eq!(p.rect.y, 300);
    let near_right = Rect::new(1400, 300, side, side);
    let (p, e) = snap_after_drop(&ms, near_right, 100, PetEdge::Left).expect("p");
    assert_eq!(e, PetEdge::Right);
    assert_inside(&ms, &p);
    let (p, e) = snap_after_drop(&ms, near_left, 100, PetEdge::Free).expect("p");
    assert_eq!(e, PetEdge::Free);
    assert_eq!((p.rect.x, p.rect.y), (500, 300));
}

#[test]
fn drop_off_every_screen_lands_on_the_nearest_monitor() {
    let ms = vec![
        monitor("P", (0, 0), (1920, 1080), 1.0, TaskbarEdge::Bottom, true),
        monitor(
            "S",
            (1920, 0),
            (1366, 768),
            1.25,
            TaskbarEdge::Bottom,
            false,
        ),
    ];
    let way_right = Rect::new(5000, 300, 144, 144);
    let (p, _) = snap_after_drop(&ms, way_right, 100, PetEdge::Free).expect("p");
    assert_eq!(p.monitor, 1);
    assert_inside(&ms, &p);
    assert_eq!(p.rect.width, pet_window_physical(100, &ms[1]));
    let below = Rect::new(-400, 3000, 144, 144);
    let (p, _) = snap_after_drop(&ms, below, 100, PetEdge::Free).expect("p");
    assert_eq!(p.monitor, 0);
    assert_inside(&ms, &p);
}

#[test]
fn drop_across_monitors_with_different_scaling_takes_the_target_size() {
    let ms = vec![
        monitor("P", (0, 0), (1920, 1080), 1.0, TaskbarEdge::Bottom, true),
        monitor(
            "S",
            (1920, 0),
            (3840, 2160),
            2.0,
            TaskbarEdge::Bottom,
            false,
        ),
    ];
    // Mostly on the 200% monitor.
    let r = Rect::new(1900, 500, 144, 144);
    let (p, _) = snap_after_drop(&ms, r, 125, PetEdge::Free).expect("p");
    assert_eq!(p.monitor, 1);
    assert_eq!(p.rect.width, pet_window_physical(125, &ms[1]));
    assert_inside(&ms, &p);
}

/// Owner item 6, criterion 4: a size change takes effect at once and never moves the pet
/// outside the work area.
#[test]
fn resize_keeps_the_pet_inside_and_anchored() {
    for scale in SCALES {
        for edge in EDGES {
            let ms = single((1366, 768), scale, edge);
            for from in sizes() {
                let home = bring_above_clock(&ms, from).expect("p");
                for to in sizes() {
                    let p = resize_anchored(&ms, home.rect, to, PetEdge::Right).expect("p");
                    assert_inside(&ms, &p);
                    assert_eq!(p.rect.width, pet_window_physical(to, &ms[0]));
                    assert_eq!(p.rect.bottom(), home.rect.bottom(), "bottom edge kept");
                    assert_eq!(p.rect.right(), home.rect.right(), "right edge kept");
                }
            }
        }
    }
}

#[test]
fn resize_on_the_left_keeps_the_left_edge() {
    let ms = single((1920, 1080), 1.0, TaskbarEdge::Bottom);
    let r = Rect::new(4, 500, 144, 144);
    let p = resize_anchored(&ms, r, 150, PetEdge::Left).expect("p");
    assert_eq!(p.rect.x, 4);
    assert_eq!(p.rect.bottom(), r.bottom());
    // Growing at the top of the screen pushes down rather than off-screen.
    let top = Rect::new(800, 0, 112, 112);
    let p = resize_anchored(&ms, top, 150, PetEdge::Free).expect("p");
    assert_inside(&ms, &p);
    assert_eq!(p.rect.y, 0);
}

#[test]
fn resize_of_a_lost_pet_brings_it_home() {
    let ms = single((1920, 1080), 1.0, TaskbarEdge::Bottom);
    let lost = Rect::new(-5000, -5000, 144, 144);
    assert_eq!(
        resize_anchored(&ms, lost, 125, PetEdge::Right),
        bring_above_clock(&ms, 125)
    );
}

/// Wireframe P0: bubbles stay inside the work area at every size.
#[test]
fn bubble_stays_inside_the_work_area_beside_the_pet() {
    for res in RESOLUTIONS {
        for scale in SCALES {
            for edge in EDGES {
                let ms = single(res, scale, edge);
                let m = &ms[0];
                for pct in sizes() {
                    let (lw, lh) = bubble_limits_logical(m, pct);
                    let size = (m.to_physical(lw), m.to_physical(lh));
                    for pet in [
                        bring_above_clock(&ms, pct).expect("p").rect,
                        reconcile(&ms, Some((0, 0)), pct, PetEdge::Left)
                            .expect("p")
                            .rect,
                    ] {
                        let l = layout_bubble(m.work_area, pet, size, gap_physical(m));
                        assert!(
                            m.work_area.contains(&l.window),
                            "{l:?} in {:?}",
                            m.work_area
                        );
                        assert!(m.work_area.contains(&l.bubble));
                        assert_eq!(l.pet, pet, "the pet does not move");
                        assert!(l.window.contains(&l.pet) && l.window.contains(&l.bubble));
                        assert_eq!(
                            l.bubble.intersection_area(&l.pet),
                            0,
                            "bubble beside the pet, not over it"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn bubble_opens_towards_the_middle() {
    let ms = single((1920, 1080), 1.0, TaskbarEdge::Bottom);
    let wa = ms[0].work_area;
    let home = bring_above_clock(&ms, 100).expect("p").rect;
    let l = layout_bubble(wa, home, (300, 200), 6);
    assert_eq!(l.side, Side::Left);
    assert_eq!(l.bubble.right(), home.x - 6);
    assert_eq!(l.bubble.bottom(), home.bottom());
    let left = Rect::new(4, 100, 144, 144);
    let l = layout_bubble(wa, left, (300, 400), 6);
    assert_eq!(l.side, Side::Right);
    assert_eq!(l.bubble.x, left.right() + 6);
    // Taller than the space above the pet's bottom: pushed down, still inside.
    assert_eq!(l.bubble.y, wa.y);
}

#[test]
fn bubble_limits_respect_the_wireframe_rule() {
    // 1366 x 768 at 150%: the logical work area is about 910 x 480.
    let ms = single((1366, 768), 1.5, TaskbarEdge::Bottom);
    let (w, h) = bubble_limits_logical(&ms[0], 150);
    assert!(w <= BUBBLE_MAX_WIDTH_LOGICAL);
    let (_, wh) = ms[0].work_area_logical();
    assert!(h <= wh * BUBBLE_MAX_HEIGHT_SHARE + 0.01);
}

#[test]
fn settings_window_fits_small_and_large_displays() {
    let small = single((1366, 768), 1.5, TaskbarEdge::Bottom);
    let (w, h) = screens_window_logical(&small[0]);
    let (ww, wh) = small[0].work_area_logical();
    assert!(w <= ww && h <= wh, "{w}x{h} in {ww}x{wh}");
    assert!(w >= 640.0);
    let big = single((2560, 1440), 1.0, TaskbarEdge::Bottom);
    assert_eq!(screens_window_logical(&big[0]), (900.0, 640.0));
    let tiny = single((800, 600), 2.0, TaskbarEdge::Bottom);
    let (w, h) = screens_window_logical(&tiny[0]);
    let (ww, wh) = tiny[0].work_area_logical();
    assert!(w <= ww && h <= wh);
}

#[test]
fn clamp_into_handles_every_side_and_oversize() {
    let area = Rect::new(10, 20, 100, 50);
    assert_eq!(
        clamp_into(Rect::new(0, 0, 10, 10), area),
        Rect::new(10, 20, 10, 10)
    );
    assert_eq!(
        clamp_into(Rect::new(200, 200, 10, 10), area),
        Rect::new(100, 60, 10, 10)
    );
    assert_eq!(
        clamp_into(Rect::new(50, 30, 200, 10), area),
        Rect::new(10, 30, 200, 10)
    );
}

#[test]
fn no_monitors_means_no_placement() {
    assert!(reconcile(&[], Some((0, 0)), 100, PetEdge::Right).is_none());
    assert!(snap_after_drop(&[], Rect::new(0, 0, 1, 1), 100, PetEdge::Right).is_none());
    assert!(resize_anchored(&[], Rect::new(0, 0, 1, 1), 100, PetEdge::Right).is_none());
}

fn arb_monitors() -> impl Strategy<Value = Vec<Monitor>> {
    let one = (
        prop::sample::select(RESOLUTIONS.to_vec()),
        prop::sample::select(SCALES.to_vec()),
        prop::sample::select(EDGES.to_vec()),
    );
    (prop::collection::vec(one, 1..=3), 0usize..3).prop_map(|(specs, primary)| {
        let mut x = 0;
        let n = specs.len();
        specs
            .into_iter()
            .enumerate()
            .map(|(i, (res, scale, edge))| {
                let m = monitor(&format!("D{i}"), (x, 0), res, scale, edge, i == primary % n);
                x += res.0;
                m
            })
            .collect()
    })
}

fn arb_edge() -> impl Strategy<Value = PetEdge> {
    prop::sample::select(vec![PetEdge::Left, PetEdge::Right, PetEdge::Free])
}

fn arb_pct() -> impl Strategy<Value = u16> {
    (0u16..=15).prop_map(|k| SCALE_MIN + k * SCALE_STEP)
}

proptest! {
    #[test]
    fn any_saved_position_ends_inside_a_work_area(
        ms in arb_monitors(),
        x in -12_000i32..12_000,
        y in -6_000i32..6_000,
        pct in arb_pct(),
        edge in arb_edge(),
    ) {
        let p = reconcile(&ms, Some((x, y)), pct, edge).expect("placed");
        prop_assert!(ms[p.monitor].work_area.contains(&p.rect));
        prop_assert_eq!(p.rect.width, pet_window_physical(pct, &ms[p.monitor]));
    }

    #[test]
    fn any_drop_ends_inside_a_work_area(
        ms in arb_monitors(),
        x in -12_000i32..12_000,
        y in -6_000i32..6_000,
        pct in arb_pct(),
        edge in arb_edge(),
    ) {
        let (p, e) = snap_after_drop(&ms, Rect::new(x, y, 144, 144), pct, edge).expect("placed");
        prop_assert!(ms[p.monitor].work_area.contains(&p.rect));
        prop_assert_eq!(edge == PetEdge::Free, e == PetEdge::Free);
    }

    #[test]
    fn any_resize_and_bubble_stay_inside(
        ms in arb_monitors(),
        x in -12_000i32..12_000,
        y in -6_000i32..6_000,
        from in arb_pct(),
        to in arb_pct(),
        bw in 1i32..2_000,
        bh in 1i32..2_000,
    ) {
        let p = reconcile(&ms, Some((x, y)), from, PetEdge::Free).expect("placed");
        let q = resize_anchored(&ms, p.rect, to, PetEdge::Free).expect("placed");
        let m = &ms[q.monitor];
        prop_assert!(m.work_area.contains(&q.rect));
        let l = layout_bubble(m.work_area, q.rect, (bw, bh), gap_physical(m));
        prop_assert!(m.work_area.contains(&l.window));
        prop_assert_eq!(l.pet, q.rect);
    }
}
