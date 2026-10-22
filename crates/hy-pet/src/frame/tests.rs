//! Frame regressions: the rectangle handed to the window-position call stays inside the
//! current work area when the pet is moved with a menu or bubble open (monitor removed,
//! scaling changed, "Bring pet above clock" with the engine's bubble open), at every
//! size, scaling and taskbar side.

use super::*;
use crate::geometry::{TaskbarEdge, to_physical};
use crate::placement::{BUBBLE_MAX_WIDTH_LOGICAL, SCALE_MAX, SCALE_MIN, SCALE_STEP};
use crate::shell::{ActionStyle, Bubble, BubbleAction, BubbleOutcome};

const SCALES: [f64; 4] = [1.0, 1.25, 1.5, 2.0];
const EDGES: [TaskbarEdge; 4] = [
    TaskbarEdge::Bottom,
    TaskbarEdge::Right,
    TaskbarEdge::Top,
    TaskbarEdge::Left,
];

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

/// A primary 1920 x 1080 monitor and a second one to its right.
fn two(scale: f64, edge: TaskbarEdge) -> Vec<Monitor> {
    vec![
        monitor("P", (0, 0), (1920, 1080), scale, edge, true),
        monitor(
            "S",
            (1920, 0),
            (2560, 1440),
            1.0,
            TaskbarEdge::Hidden,
            false,
        ),
    ]
}

fn engine_bubble() -> Bubble {
    Bubble {
        id: "drink-1".into(),
        title: "Time for some water".into(),
        lines: vec!["A glass now keeps the afternoon easy.".into()],
        chips: vec![],
        actions: vec![BubbleAction {
            id: "done".into(),
            label: "Done".into(),
            style: ActionStyle::Primary,
        }],
        ..Bubble::default()
    }
}

/// Render as the companion does and return the rectangle passed to the window, checking
/// it lies inside the current monitor's work area.
fn render_inside(core: &mut ShellCore) -> Rect {
    let f = core.frame_for_render(false);
    let window = f.window.expect("a window rectangle");
    let wa = core.monitor().expect("a monitor").work_area;
    if let Some(applied) = f.apply {
        assert_eq!(applied, window);
    }
    let applied = core.applied().expect("applied");
    assert_eq!(
        applied, window,
        "the applied rectangle is the window rectangle"
    );
    assert!(
        wa.contains(&applied),
        "window {applied:?} outside work area {wa:?}"
    );
    applied
}

/// The page measures what is open (the largest bubble allowed, the worst case).
fn measure_max(core: &mut ShellCore) {
    let token = core.open_token();
    assert!(core.bubble_measured(token, BUBBLE_MAX_WIDTH_LOGICAL, 2000.0));
}

fn start_on_secondary(monitors: &[Monitor], pct: u16) -> ShellCore {
    let mut core = ShellCore::new(pct);
    // The saved spot is near the far corner of the second monitor.
    let s = &monitors[1].work_area;
    let p = reconcile(monitors, Some((s.right() - 10, s.y)), pct, PetEdge::Free).expect("placed");
    assert_eq!(p.monitor, 1);
    core.place(p, monitors, pct);
    render_inside(&mut core);
    core
}

#[test]
fn monitor_removal_with_the_menu_open_keeps_the_window_inside() {
    let monitors = two(1.25, TaskbarEdge::Bottom);
    let mut core = start_on_secondary(&monitors, 100);
    core.model.open_menu();
    render_inside(&mut core);
    measure_max(&mut core);
    let before = render_inside(&mut core);
    assert!(monitors[1].work_area.contains(&before));
    let old_token = core.open_token();

    // The second monitor is unplugged.
    let left = vec![monitors[0].clone()];
    core.display_changed(&left, None, 100, PetEdge::Free)
        .expect("placed");
    let after = render_inside(&mut core);
    assert!(left[0].work_area.contains(&after));
    // The scaling and work area differ, so the page measures again; until then the
    // window is the pet alone, and a late measurement for the old token is ignored.
    assert_ne!(core.open_token(), old_token);
    assert_eq!(after, core.pet().expect("pet"));
    assert!(!core.bubble_measured(old_token, 300.0, 200.0));
    measure_max(&mut core);
    let laid = render_inside(&mut core);
    let l = core.layout().expect("laid out");
    assert!(laid.contains(&l.bubble) && laid.contains(&l.pet));
    assert!(left[0].work_area.contains(&l.bubble));
}

#[test]
fn bring_home_with_the_engine_bubble_open_moves_the_bubble_too() {
    let monitors = vec![monitor(
        "P",
        (0, 0),
        (1920, 1080),
        1.0,
        TaskbarEdge::Bottom,
        true,
    )];
    let mut core = ShellCore::new(100);
    // The pet sits at the top-left, far from home.
    let p = reconcile(&monitors, Some((0, 0)), 100, PetEdge::Free).expect("placed");
    core.place(p, &monitors, 100);
    let (outcome, _) = core.model.show_bubble(engine_bubble());
    assert_eq!(outcome, BubbleOutcome::Shown);
    render_inside(&mut core);
    measure_max(&mut core);
    let before = render_inside(&mut core);
    let token = core.open_token();

    let (_, home) = core.bring_home(&monitors, 100);
    let home = home.expect("home");
    // The notice never replaces the engine's bubble, so what is open is unchanged...
    assert!(
        core.model
            .show_notice(Bubble::notice("back-1", "Back"))
            .is_empty()
    );
    let after = render_inside(&mut core);
    // ...but the window and the bubble moved with the pet, inside the work area.
    assert_ne!(after, before);
    assert_eq!(
        core.open_token(),
        token,
        "same monitor: no new measurement needed"
    );
    let l = core.layout().expect("still laid out");
    assert_eq!(l.pet, home.rect);
    assert!(after.contains(&home.rect) && after.contains(&l.bubble));
    assert!(monitors[0].work_area.contains(&l.bubble));
}

#[test]
fn scaling_change_with_a_bubble_open_measures_again() {
    let at = |scale| {
        vec![monitor(
            "P",
            (0, 0),
            (2560, 1440),
            scale,
            TaskbarEdge::Bottom,
            true,
        )]
    };
    let m100 = at(1.0);
    let mut core = ShellCore::new(125);
    let (_, p) = core.bring_home(&m100, 125);
    assert!(p.is_some());
    core.model.show_bubble(engine_bubble());
    render_inside(&mut core);
    assert!(core.bubble_measured(core.open_token(), 300.0, 160.0));
    let w100 = render_inside(&mut core);
    let old_token = core.open_token();

    // Windows scaling goes to 150%: the pet grows and moves; the bubble is measured again.
    let m150 = at(1.5);
    core.display_changed(&m150, None, 125, PetEdge::Right)
        .expect("placed");
    let pet_only = render_inside(&mut core);
    assert_eq!(pet_only, core.pet().expect("pet"));
    assert_ne!(core.open_token(), old_token);
    assert!(core.bubble_measured(core.open_token(), 300.0, 160.0));
    let w150 = render_inside(&mut core);
    let l = core.layout().expect("laid out");
    assert_eq!(l.bubble.width, to_physical(300.0, 1.5));
    assert!(w150.width > w100.width);
}

#[test]
fn the_window_stays_inside_for_every_size_scaling_and_taskbar_side() {
    for scale in SCALES {
        for edge in EDGES {
            let mut pct = SCALE_MIN;
            while pct <= SCALE_MAX {
                let monitors = two(scale, edge);
                // Monitor removal with the menu open.
                let mut core = start_on_secondary(&monitors, pct);
                core.model.open_menu();
                render_inside(&mut core);
                measure_max(&mut core);
                render_inside(&mut core);
                let left = vec![monitors[0].clone()];
                core.display_changed(&left, None, pct, PetEdge::Free)
                    .expect("placed");
                render_inside(&mut core);
                measure_max(&mut core);
                render_inside(&mut core);

                // Bring home from the second monitor with the engine's bubble open.
                let mut core = start_on_secondary(&monitors, pct);
                core.model.show_bubble(engine_bubble());
                render_inside(&mut core);
                measure_max(&mut core);
                render_inside(&mut core);
                let (_, home) = core.bring_home(&monitors, pct);
                let home = home.expect("home");
                let w = render_inside(&mut core);
                assert!(monitors[home.monitor].work_area.contains(&w));
                measure_max(&mut core);
                let w = render_inside(&mut core);
                assert!(
                    w.contains(&home.rect),
                    "{scale} {edge:?} {pct}%: window {w:?} lost the pet {:?}",
                    home.rect
                );
                pct += SCALE_STEP;
            }
        }
    }
}

#[test]
fn bring_home_with_the_menu_open_closes_it_and_opens_the_notice() {
    let monitors = two(1.0, TaskbarEdge::Bottom);
    let mut core = start_on_secondary(&monitors, 100);
    core.model.open_menu();
    render_inside(&mut core);
    measure_max(&mut core);
    render_inside(&mut core);
    let token = core.open_token();
    let (_, home) = core.bring_home(&monitors, 100);
    assert!(home.is_some());
    assert!(
        !core
            .model
            .show_notice(Bubble::notice("back-1", "Back"))
            .is_empty()
    );
    let w = render_inside(&mut core);
    assert_eq!(
        w,
        home.expect("home").rect,
        "notice not measured yet: pet alone"
    );
    assert!(core.open_token() > token);
    assert!(core.layout().is_none());
}

#[test]
fn a_drag_closes_what_is_open_and_moves_the_window_itself() {
    let monitors = two(1.0, TaskbarEdge::Bottom);
    let mut core = start_on_secondary(&monitors, 100);
    core.model.open_menu();
    render_inside(&mut core);
    measure_max(&mut core);
    render_inside(&mut core);
    let fx = core.drag_begin_move();
    assert!(!fx.is_empty());
    assert!(core.layout().is_none());
    let pet = core.pet().expect("pet");
    let (r, resize) = core.drag_to(pet.x - 50, pet.y + 20).expect("moved");
    assert!(resize, "the window was pet plus menu, now the pet alone");
    assert_eq!(core.applied(), Some(r));
    // While dragging, a render never positions the window.
    assert_eq!(core.frame_for_render(true).apply, None);
}

#[test]
fn an_unchanged_frame_is_not_applied_twice_but_a_placement_is() {
    let monitors = two(1.0, TaskbarEdge::Bottom);
    let mut core = start_on_secondary(&monitors, 100);
    assert_eq!(core.frame_for_render(false).apply, None);
    let p = reconcile(
        &monitors,
        core.pet().map(|r| (r.x, r.y)),
        100,
        PetEdge::Free,
    )
    .expect("placed");
    core.place(p, &monitors, 100);
    // Same rectangle, but placed again (a scaling change may have resized the window).
    assert_eq!(core.frame_for_render(false).apply, core.pet());
}
