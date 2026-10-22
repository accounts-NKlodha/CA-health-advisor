//! The pet shell: the pet window, its bubble, the keyboard panel and the settings window,
//! driven by the pure rules in `hy-pet`.
//!
//! Threading: every method of [`Shell`] runs on Tauri's main (UI) thread: synchronous
//! commands, tray and window events run there already, and everything else is posted
//! with `run_on_main_thread`. [`ShellHandle`], the engine's [`PetShell`], posts its calls
//! the same way, so the engine can call it from any thread. The `inner` lock is never
//! held while calling the engine's sink, and the store lock is only ever taken inside
//! `inner` (never the other way round), so there is no lock-order deadlock.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hy_core::schema::PetEdge;
use hy_core::store::{PetProfile, Store};
use hy_pet::geometry::{Monitor, Rect, layout_fingerprint};
use hy_pet::hotkey::Hotkey;
use hy_pet::messages::{MessageId, bring_back_message, message};
use hy_pet::pack::{Library, Pack};
use hy_pet::placement::{
    BubbleLayout, Placement, Side, bring_above_clock, bubble_limits_logical, gap_physical,
    layout_bubble, pet_window_logical, reconcile, resize_anchored, snap_after_drop,
};
use hy_pet::profile::{self, PetEdit};
use hy_pet::shell::{
    Bubble, BubbleOutcome, CloseCause, Effect, Handled, HideReason, MenuAction, Open, PetShell,
    ShellEvent, ShellEventSink, ShellModel, TrayView,
};
use hy_pet::state::{PetState, StateLook};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::view::{
    LayoutView, MenuView, PanelView, PetView, SettingsView, menu_view, open_view, pose_for,
    settings_view,
};
use crate::{tray, windows};

/// The pet window's label.
pub const PET: &str = "pet";
/// The keyboard panel's label.
pub const PANEL: &str = "panel";
/// The settings window's label.
pub const SCREENS: &str = "screens";

/// How long the "Your pet is back above the clock" notice stays.
const NOTICE_MS: u64 = 2600;
/// The safety-net check of the display layout, in case a change message is missed.
const LAYOUT_POLL: Duration = Duration::from_secs(3);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    cursor: (f64, f64),
    origin: (i32, i32),
}

#[derive(Debug)]
struct Inner {
    model: ShellModel,
    profile: PetProfile,
    /// The pet's rectangle, physical pixels.
    pet: Option<Rect>,
    /// The monitor the pet is on.
    monitor: Option<Monitor>,
    fingerprint: String,
    /// Changes whenever what is open changes.
    open_token: u64,
    open_key: String,
    /// The measured bubble size for `open_token`, logical pixels.
    bubble_logical: Option<(f64, f64)>,
    layout: Option<BubbleLayout>,
    /// The window rectangle last applied.
    applied: Option<Rect>,
    drag: Option<Drag>,
    hotkey: Option<Hotkey>,
    hotkey_registered: bool,
    panel_prev_foreground: isize,
    notice_seq: u64,
    page_ready: bool,
}

/// The pet shell.
pub struct Shell {
    app: AppHandle,
    store: Mutex<Store>,
    library: Library,
    inner: Mutex<Inner>,
    sink: Mutex<Option<Arc<dyn ShellEventSink>>>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").finish_non_exhaustive()
    }
}

fn open_key(open: &Open) -> String {
    match open {
        Open::None => String::new(),
        Open::Bubble { bubble } => format!("bubble:{}", bubble.id),
        Open::Notice { bubble } => format!("notice:{}", bubble.id),
        Open::Menu => "menu".to_owned(),
    }
}

/// The monitors as Windows reports them.
pub fn current_monitors(app: &AppHandle) -> Vec<Monitor> {
    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.name().cloned(), *m.position()));
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let pos = m.position();
            let size = m.size();
            let wa = m.work_area();
            let id = m.name().cloned().unwrap_or_else(|| format!("monitor-{i}"));
            let is_primary = match &primary {
                Some((Some(name), _)) => m.name() == Some(name),
                Some((None, p)) => p == pos,
                // Windows puts the primary monitor's top-left at (0, 0).
                None => pos.x == 0 && pos.y == 0,
            };
            Monitor {
                id,
                bounds: Rect::new(
                    pos.x,
                    pos.y,
                    i32::try_from(size.width).unwrap_or(i32::MAX),
                    i32::try_from(size.height).unwrap_or(i32::MAX),
                ),
                work_area: Rect::new(
                    wa.position.x,
                    wa.position.y,
                    i32::try_from(wa.size.width).unwrap_or(i32::MAX),
                    i32::try_from(wa.size.height).unwrap_or(i32::MAX),
                ),
                scale: m.scale_factor(),
                primary: is_primary,
            }
        })
        .collect()
}

/// Move and size a window in physical pixels. Position, size, then position again: a
/// move onto a monitor with a different scale makes Windows resize the window, so the
/// size is set after the move and the position re-applied.
pub fn set_window_rect(w: &WebviewWindow, r: Rect) {
    let pos = PhysicalPosition::new(r.x, r.y);
    let size = PhysicalSize::new(
        u32::try_from(r.width.max(1)).unwrap_or(1),
        u32::try_from(r.height.max(1)).unwrap_or(1),
    );
    let _ = w.set_position(pos);
    let _ = w.set_size(size);
    let _ = w.set_position(pos);
}

/// The window handle as an integer (Windows).
#[cfg(windows)]
pub fn hwnd_of(w: &WebviewWindow) -> Option<isize> {
    w.hwnd().ok().map(|h| h.0 as isize)
}

impl Shell {
    /// Create the shell from the opened store and loaded library.
    ///
    /// # Errors
    /// A store read error.
    pub fn new(
        app: AppHandle,
        store: Store,
        library: Library,
    ) -> Result<Arc<Self>, hy_core::error::Error> {
        let profile = profile::current_profile(&store, &library)?;
        let hotkey = profile::panel_hotkey(&store)?;
        Ok(Arc::new(Self {
            app,
            store: Mutex::new(store),
            library,
            inner: Mutex::new(Inner {
                model: ShellModel::new(),
                profile,
                pet: None,
                monitor: None,
                fingerprint: String::new(),
                open_token: 0,
                open_key: String::new(),
                bubble_logical: None,
                layout: None,
                applied: None,
                drag: None,
                hotkey,
                hotkey_registered: false,
                panel_prev_foreground: 0,
                notice_seq: 0,
                page_ready: false,
            }),
            sink: Mutex::new(None),
        }))
    }

    /// The engine's handle.
    pub fn handle(self: &Arc<Self>) -> ShellHandle {
        ShellHandle(Arc::clone(self))
    }

    /// The app.
    pub const fn app(&self) -> &AppHandle {
        &self.app
    }

    fn pet_window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(PET)
    }

    fn pack<'a>(&'a self, profile: &PetProfile) -> &'a Pack {
        self.library
            .selectable(&profile.pet_id)
            .unwrap_or_else(|| self.library.default_pack())
    }

    /// Post work to the main thread.
    pub fn post(self: &Arc<Self>, f: impl FnOnce(&Arc<Self>) + Send + 'static) {
        let me = Arc::clone(self);
        let _ = self.app.run_on_main_thread(move || f(&me));
    }

    // ------------------------------------------------------------- start-up

    /// Place and show the pet, start listening for display changes and the hotkey.
    /// Called once, on the main thread, after the pet window exists.
    pub fn start(self: &Arc<Self>) {
        let monitors = current_monitors(&self.app);
        {
            let mut i = lock(&self.inner);
            i.fingerprint = layout_fingerprint(&monitors);
            let p = reconcile(
                &monitors,
                i.profile.position,
                i.profile.scale_pct,
                i.profile.edge,
            );
            if let Some(p) = p {
                place(&mut i, p, &monitors);
            }
        }
        self.render();
        self.show_window();
        self.install_signals();
        let hk = lock(&self.inner).hotkey;
        self.apply_hotkey(hk);
        let me = Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("hy-layout-poll".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(LAYOUT_POLL);
                    let Some(shell) = me.upgrade() else { break };
                    shell.post(|s| s.on_display_change(false));
                }
            });
        if let Some(w) = self.pet_window() {
            let me = Arc::downgrade(self);
            w.on_window_event(move |e| {
                if let tauri::WindowEvent::ScaleFactorChanged { .. } = e
                    && let Some(shell) = me.upgrade()
                {
                    shell.post(|s| s.on_display_change(true));
                }
            });
        }
    }

    #[cfg(windows)]
    fn install_signals(self: &Arc<Self>) {
        let Some(h) = self.pet_window().as_ref().and_then(hwnd_of) else {
            return;
        };
        let me = Arc::downgrade(self);
        crate::win32::install_signals(
            h,
            Box::new(move |signal| {
                if let Some(shell) = me.upgrade() {
                    match signal {
                        crate::win32::Signal::DisplayChanged => {
                            shell.post(|s| s.on_display_change(true));
                        }
                        crate::win32::Signal::Hotkey => shell.post(|s| s.open_panel()),
                    }
                }
            }),
        );
    }

    #[cfg(not(windows))]
    #[allow(clippy::unused_self)]
    fn install_signals(self: &Arc<Self>) {}

    /// Register (or clear) the panel hotkey. Returns whether Windows accepted it.
    pub fn apply_hotkey(&self, hotkey: Option<Hotkey>) -> bool {
        #[cfg(windows)]
        let ok = match (hotkey, self.pet_window().as_ref().and_then(hwnd_of)) {
            (Some(hk), Some(h)) => {
                crate::win32::register_hotkey(h, hk.win32_modifiers(), hk.virtual_key())
            }
            (None, Some(h)) => {
                crate::win32::unregister_hotkey(h);
                false
            }
            _ => false,
        };
        #[cfg(not(windows))]
        let ok = false;
        let mut i = lock(&self.inner);
        i.hotkey = hotkey;
        i.hotkey_registered = ok;
        ok
    }

    // --------------------------------------------------------- effect plumbing

    fn run(self: &Arc<Self>, f: impl FnOnce(&mut Inner, &Self) -> Vec<Effect>) {
        let fx = {
            let mut i = lock(&self.inner);
            f(&mut i, self)
        };
        self.apply(fx);
    }

    fn apply(self: &Arc<Self>, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::ShowWindow => self.show_window(),
                Effect::HideWindow => self.hide_window(),
                Effect::Render => self.render(),
                Effect::UpdateTray => tray::refresh(&self.app),
                Effect::Emit(ev) => self.emit_event(&ev),
            }
        }
    }

    fn emit_event(self: &Arc<Self>, ev: &ShellEvent) {
        let sink = lock(&self.sink).clone();
        let handled = sink.map_or(Handled::No, |s| s.on_event(ev));
        if let (ShellEvent::PetClicked { .. }, Handled::No) = (ev, handled) {
            self.run(|i, _| i.model.open_menu());
        }
    }

    fn show_window(&self) {
        let Some(w) = self.pet_window() else { return };
        #[cfg(windows)]
        if let Some(h) = hwnd_of(&w) {
            crate::win32::show_no_activate(h);
            crate::win32::raise_topmost(h);
            return;
        }
        let _ = w.show();
    }

    fn hide_window(&self) {
        let Some(w) = self.pet_window() else { return };
        #[cfg(windows)]
        if let Some(h) = hwnd_of(&w) {
            crate::win32::hide(h);
            return;
        }
        let _ = w.hide();
    }

    /// The tray's labels.
    pub fn tray_view(&self) -> TrayView {
        lock(&self.inner).model.tray()
    }

    /// The hotkey label for menus.
    pub fn hotkey_label(&self) -> Option<String> {
        let i = lock(&self.inner);
        i.hotkey
            .filter(|_| i.hotkey_registered)
            .map(|h| h.to_string())
    }

    // ------------------------------------------------------------------ render

    fn menu_for(&self, i: &Inner) -> MenuView {
        let hk = i
            .hotkey
            .filter(|_| i.hotkey_registered)
            .map(|h| h.to_string());
        menu_view(&i.profile, i.model.menu_actions(), hk)
    }

    /// Lay the window out and send the pet page (and the panel, if open) a fresh view.
    pub fn render(&self) {
        let (view, panel) = {
            let mut i = lock(&self.inner);
            let key = open_key(i.model.open());
            if key != i.open_key {
                i.open_key = key;
                i.open_token += 1;
                i.bubble_logical = None;
                i.layout = None;
            }
            let window_rect = match (i.model.open(), i.layout, i.pet) {
                (Open::None, _, Some(p)) | (_, None, Some(p)) => Some(p),
                (_, Some(l), _) => Some(l.window),
                _ => None,
            };
            if let (Some(r), Some(w)) = (window_rect, self.pet_window())
                && i.applied != Some(r)
                && i.drag.is_none()
            {
                set_window_rect(&w, r);
                i.applied = Some(r);
            }
            let view = self.pet_view(&i, window_rect);
            let panel = self
                .app
                .get_webview_window(PANEL)
                .is_some()
                .then(|| self.panel_view_locked(&i));
            (view, panel)
        };
        let _ = self.app.emit_to(PET, "pet://view", &view);
        if let Some(p) = panel {
            let _ = self.app.emit_to(PANEL, "panel://view", &p);
        }
    }

    fn pet_view(&self, i: &Inner, window: Option<Rect>) -> PetView {
        let pack = self.pack(&i.profile);
        let (state, tag) = i.model.state();
        let look = StateLook::of(state, tag);
        let scale = i.monitor.as_ref().map_or(1.0, Monitor::safe_scale);
        let (max_w, max_h) = i.monitor.as_ref().map_or((320.0, 400.0), |m| {
            bubble_limits_logical(m, i.profile.scale_pct)
        });
        let to_css = |v: i32| f64::from(v) / scale;
        let layout = match (window, i.pet, i.layout) {
            (Some(w), Some(p), l) => {
                let measured = l.filter(|_| i.bubble_logical.is_some());
                LayoutView {
                    pet_x: to_css(p.x - w.x),
                    pet_y: to_css(p.y - w.y),
                    bubble_x: measured.map_or(0.0, |l| to_css(l.bubble.x - w.x)),
                    bubble_y: measured.map_or(0.0, |l| to_css(l.bubble.y - w.y)),
                    measured_token: if measured.is_some() { i.open_token } else { 0 },
                    side: measured.map_or(Side::Left, |l| l.side),
                    bubble_max_w: max_w,
                    bubble_max_h: max_h,
                }
            }
            _ => LayoutView {
                pet_x: 0.0,
                pet_y: 0.0,
                bubble_x: 0.0,
                bubble_y: 0.0,
                measured_token: 0,
                side: Side::Left,
                bubble_max_w: max_w,
                bubble_max_h: max_h,
            },
        };
        PetView {
            name: i.profile.display_name.clone(),
            pack_key: format!("{}@{}", pack.id(), pack.manifest.version),
            art_svg: pack.art_svg.clone(),
            colours: pack.colours(i.profile.colour_variant).clone(),
            hitbox: pack.manifest.hitbox,
            pose: pose_for(pack, look.base),
            accessible_name: look.accessible_name(&i.profile.display_name),
            look,
            motion: i.profile.motion,
            pet_px: pet_window_logical(i.profile.scale_pct),
            open: open_view(i.model.open(), i.open_token, || self.menu_for(i)),
            layout,
        }
    }

    /// The pet page's current view (for its first load).
    pub fn current_pet_view(&self) -> PetView {
        let i = lock(&self.inner);
        let window = i.applied.or(i.pet);
        self.pet_view(&i, window)
    }

    /// The pet page has loaded and drawn its first view.
    pub fn mark_page_ready(&self) {
        lock(&self.inner).page_ready = true;
    }

    /// Whether the pet page has loaded (for the self-test).
    pub fn page_ready(&self) -> bool {
        lock(&self.inner).page_ready
    }

    // ---------------------------------------------------------------- geometry

    fn relayout(i: &mut Inner) {
        if let (Some((w, h)), Some(m), Some(pet)) = (i.bubble_logical, i.monitor.as_ref(), i.pet) {
            let size = (m.to_physical(w), m.to_physical(h));
            i.layout = Some(layout_bubble(m.work_area, pet, size, gap_physical(m)));
        } else {
            i.layout = None;
        }
    }

    /// The pet page measured the open bubble (logical pixels).
    pub fn bubble_measured(self: &Arc<Self>, token: u64, width: f64, height: f64) {
        if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
            return;
        }
        self.run(|i, _| {
            if token != i.open_token || matches!(i.model.open(), Open::None) {
                return Vec::new();
            }
            let (max_w, max_h) = i.monitor.as_ref().map_or((width, height), |m| {
                bubble_limits_logical(m, i.profile.scale_pct)
            });
            i.bubble_logical = Some((width.min(max_w), height.min(max_h)));
            Self::relayout(i);
            vec![Effect::Render]
        });
    }

    /// The display layout may have changed: place the pet again inside the work area
    /// (monitor removed, resolution, scaling or taskbar change). With `force` false the
    /// pet is only moved if the layout fingerprint changed. Automatic moves are not
    /// saved, so the saved spot is used again if the monitor comes back after a restart.
    pub fn on_display_change(self: &Arc<Self>, force: bool) {
        let monitors = current_monitors(&self.app);
        if monitors.is_empty() {
            return;
        }
        self.run(|i, _| {
            let fp = layout_fingerprint(&monitors);
            if !force && fp == i.fingerprint {
                return Vec::new();
            }
            i.fingerprint = fp;
            let at = i.pet.map(|r| (r.x, r.y)).or(i.profile.position);
            if let Some(p) = reconcile(&monitors, at, i.profile.scale_pct, i.profile.edge) {
                place(i, p, &monitors);
                i.applied = None;
            }
            vec![Effect::Render]
        });
    }

    /// The pet's rectangle and monitor (for the self-test).
    pub fn pet_geometry(&self) -> (Option<Rect>, Option<Monitor>, Option<Rect>) {
        let i = lock(&self.inner);
        (i.pet, i.monitor.clone(), i.layout.map(|l| l.window))
    }

    // -------------------------------------------------------------------- drag

    /// The pointer went down on the pet.
    pub fn drag_start(self: &Arc<Self>) {
        let Ok(c) = self.app.cursor_position() else {
            return;
        };
        let mut i = lock(&self.inner);
        if let Some(p) = i.pet {
            i.drag = Some(Drag {
                cursor: (c.x, c.y),
                origin: (p.x, p.y),
            });
        }
    }

    /// The pointer moved during a drag: follow it. Anything open closes first.
    pub fn drag_move(self: &Arc<Self>) {
        let Ok(c) = self.app.cursor_position() else {
            return;
        };
        let fx = {
            let mut i = lock(&self.inner);
            let Some(d) = i.drag else { return };
            let fx = if matches!(i.model.open(), Open::None) {
                Vec::new()
            } else {
                // Close what is open before moving, so the window is the pet alone.
                i.layout = None;
                i.bubble_logical = None;
                i.model.user_close()
            };
            #[allow(clippy::cast_possible_truncation)]
            let (dx, dy) = (
                (c.x - d.cursor.0).round() as i32,
                (c.y - d.cursor.1).round() as i32,
            );
            if let Some(p) = i.pet {
                let r = p.at(d.origin.0 + dx, d.origin.1 + dy);
                i.pet = Some(r);
                if let Some(w) = self.pet_window() {
                    if i.applied
                        .is_some_and(|a| a.width != r.width || a.height != r.height)
                    {
                        set_window_rect(&w, r);
                    } else {
                        let _ = w.set_position(PhysicalPosition::new(r.x, r.y));
                    }
                }
                i.applied = Some(r);
            }
            fx
        };
        self.apply(fx);
    }

    /// The pointer came up. Returns true if the pet moved (a drag), false for a click.
    pub fn drag_end(self: &Arc<Self>, moved: bool) -> bool {
        let Some(d) = lock(&self.inner).drag.take() else {
            return false;
        };
        let moved_px = lock(&self.inner)
            .pet
            .is_some_and(|p| (p.x, p.y) != d.origin);
        if !moved && !moved_px {
            return false;
        }
        let monitors = current_monitors(&self.app);
        self.run(|i, shell| {
            let Some(pet) = i.pet else { return Vec::new() };
            if let Some((p, edge)) =
                snap_after_drop(&monitors, pet, i.profile.scale_pct, i.profile.edge)
            {
                place(i, p, &monitors);
                i.applied = None;
                shell.save(
                    i,
                    &PetEdit::Position {
                        x: p.rect.x,
                        y: p.rect.y,
                    },
                );
                if edge != i.profile.edge {
                    shell.save(i, &PetEdit::Edge { edge });
                }
            }
            vec![Effect::Render]
        });
        true
    }

    fn save(&self, i: &mut Inner, edit: &PetEdit) {
        let store = lock(&self.store);
        match profile::apply_edit(&store, &self.library, edit) {
            Ok(p) => i.profile = p,
            Err(e) => eprintln!("HealthierYou: could not save pet edit: {e}"),
        }
    }

    // ------------------------------------------------------------------ actions

    /// A click on the pet.
    pub fn pet_click(self: &Arc<Self>) {
        self.run(|i, _| i.model.pet_clicked());
    }

    /// The close button of the open bubble, menu or notice.
    pub fn bubble_close(self: &Arc<Self>) {
        self.run(|i, _| i.model.user_close());
    }

    /// An action in the engine's bubble.
    pub fn bubble_action(self: &Arc<Self>, bubble_id: &str, action_id: &str) {
        self.run(|i, _| i.model.bubble_action(bubble_id, action_id));
    }

    /// An engine action from the pet menu, panel or tray.
    pub fn menu_action(self: &Arc<Self>, action_id: &str) {
        self.run(|i, _| i.model.menu_action(action_id));
    }

    /// "Hide pet".
    pub fn hide_pet(self: &Arc<Self>) {
        self.run(|i, _| i.model.hide(HideReason::User));
    }

    /// The tray's "Hide pet" / "Show pet".
    pub fn toggle_from_tray(self: &Arc<Self>) {
        self.run(|i, _| {
            if i.model.is_visible() {
                i.model.hide(HideReason::User)
            } else {
                i.model.user_show()
            }
        });
    }

    /// "Bring pet above clock" (tray, Settings, pet menu, panel). Works when the pet is
    /// hidden by the user, off-screen or behind other windows; it never takes focus.
    pub fn bring_above_clock(self: &Arc<Self>) {
        let monitors = current_monitors(&self.app);
        let mut notice_id = None;
        self.run(|i, shell| {
            let mut fx = i.model.close_menu();
            fx.extend(i.model.bring_back());
            let Some(p) = bring_above_clock(&monitors, i.profile.scale_pct) else {
                return fx;
            };
            place(i, p, &monitors);
            i.applied = None;
            if i.profile.edge == PetEdge::Left {
                shell.save(
                    i,
                    &PetEdit::Edge {
                        edge: PetEdge::Right,
                    },
                );
            }
            shell.save(
                i,
                &PetEdit::Position {
                    x: p.rect.x,
                    y: p.rect.y,
                },
            );
            i.notice_seq += 1;
            let id = format!("back-{}", i.notice_seq);
            let text = message(
                bring_back_message(p.taskbar),
                i.profile.tone,
                &i.profile.display_name,
            );
            let shown = i.model.show_notice(Bubble::notice(&id, &text));
            if !shown.is_empty() {
                notice_id = Some(id);
            }
            fx.extend(shown);
            fx.push(Effect::Render);
            fx
        });
        #[cfg(windows)]
        if let Some(h) = self.pet_window().as_ref().and_then(hwnd_of) {
            crate::win32::raise_topmost(h);
        }
        if let Some(id) = notice_id {
            let me = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("hy-notice".into())
                .spawn(move || {
                    std::thread::sleep(Duration::from_millis(NOTICE_MS));
                    me.post(move |s| s.run(|i, _| i.model.notice_expired(&id)));
                });
        }
    }

    /// Apply a pet edit from Settings, the pet menu or the panel, at once.
    ///
    /// # Errors
    /// The refusal message, shown to the user.
    pub fn edit(self: &Arc<Self>, edit: &PetEdit) -> Result<SettingsView, String> {
        let saved = {
            let store = lock(&self.store);
            profile::apply_edit(&store, &self.library, edit).map_err(|e| e.to_string())?
        };
        let monitors = current_monitors(&self.app);
        self.run(|i, shell| {
            i.profile = saved;
            let placement: Option<Placement> = match edit {
                PetEdit::Size { pct } => i
                    .pet
                    .and_then(|r| resize_anchored(&monitors, r, *pct, i.profile.edge)),
                PetEdit::Edge { .. } | PetEdit::Position { .. } | PetEdit::ClearPosition => {
                    reconcile(
                        &monitors,
                        i.profile.position.or(i.pet.map(|r| (r.x, r.y))),
                        i.profile.scale_pct,
                        i.profile.edge,
                    )
                }
                PetEdit::ResetDefaults => bring_above_clock(&monitors, i.profile.scale_pct),
                PetEdit::ChoosePet { .. }
                | PetEdit::Rename { .. }
                | PetEdit::Colour { .. }
                | PetEdit::Motion { .. }
                | PetEdit::Tone { .. } => None,
            };
            if let Some(p) = placement {
                place(i, p, &monitors);
                Self::relayout(i);
                i.applied = None;
                // A resize or edge change moves the pet: keep the new spot. Reset and
                // "forget position" leave no saved spot (home above the clock).
                if matches!(edit, PetEdit::Size { .. } | PetEdit::Edge { .. }) {
                    shell.save(
                        i,
                        &PetEdit::Position {
                            x: p.rect.x,
                            y: p.rect.y,
                        },
                    );
                }
            }
            vec![Effect::Render]
        });
        let view = self.settings_view();
        let _ = self.app.emit_to(SCREENS, "settings://changed", &view);
        Ok(view)
    }

    /// Change the panel hotkey (`None` turns it off).
    ///
    /// # Errors
    /// The refusal message.
    pub fn set_hotkey(self: &Arc<Self>, text: Option<&str>) -> Result<SettingsView, String> {
        let hk = text
            .map(str::parse::<Hotkey>)
            .transpose()
            .map_err(|e| e.to_string())?;
        profile::set_panel_hotkey(&lock(&self.store), hk).map_err(|e| e.to_string())?;
        self.apply_hotkey(hk);
        self.render();
        tray::refresh(&self.app);
        Ok(self.settings_view())
    }

    /// The Settings page's view.
    pub fn settings_view(&self) -> SettingsView {
        let i = lock(&self.inner);
        settings_view(
            &i.profile,
            &self.library,
            i.hotkey.map(|h| h.to_string()),
            i.hotkey_registered,
        )
    }

    // ------------------------------------------------------------------- panel

    fn panel_view_locked(&self, i: &Inner) -> PanelView {
        PanelView {
            menu: self.menu_for(i),
            panel_hint: message(
                MessageId::PanelHint,
                i.profile.tone,
                &i.profile.display_name,
            ),
            accessible_name: StateLook::of(i.model.state().0, i.model.state().1)
                .accessible_name(&i.profile.display_name),
        }
    }

    /// The panel page's view.
    pub fn panel_view(&self) -> PanelView {
        let i = lock(&self.inner);
        self.panel_view_locked(&i)
    }

    /// Open the keyboard panel (hotkey or tray). It takes focus on purpose; the window
    /// that had focus is remembered so Escape can hand focus back.
    pub fn open_panel(self: &Arc<Self>) {
        if let Some(w) = self.app.get_webview_window(PANEL) {
            let _ = w.set_focus();
            return;
        }
        #[cfg(windows)]
        let prev = {
            let f = crate::win32::foreground();
            if f != 0 && !crate::win32::is_own_window(f) {
                f
            } else {
                0
            }
        };
        #[cfg(not(windows))]
        let prev = 0;
        let rect = {
            let mut i = lock(&self.inner);
            i.panel_prev_foreground = prev;
            let fx = i.model.close_menu();
            drop(i);
            self.apply(fx);
            let i = lock(&self.inner);
            match (i.pet, i.monitor.as_ref()) {
                (Some(pet), Some(m)) => {
                    let size = (
                        m.to_physical(windows::PANEL_LOGICAL.0),
                        m.to_physical(windows::PANEL_LOGICAL.1),
                    );
                    Some(layout_bubble(m.work_area, pet, size, gap_physical(m)).bubble)
                }
                _ => None,
            }
        };
        if let Err(e) = windows::open_panel(self, rect) {
            eprintln!("HealthierYou: could not open the pet panel: {e}");
        }
    }

    /// Close the keyboard panel. With `restore`, focus goes back to the window that had
    /// it before (done before closing, while this app may still set the foreground).
    pub fn close_panel(&self, restore: bool) {
        let prev = std::mem::take(&mut lock(&self.inner).panel_prev_foreground);
        #[cfg(windows)]
        if restore {
            crate::win32::restore_foreground(prev);
        }
        #[cfg(not(windows))]
        let _ = (restore, prev);
        if let Some(w) = self.app.get_webview_window(PANEL) {
            let _ = w.close();
        }
    }

    /// The window that had focus before the panel opened (for the self-test).
    pub fn panel_prev_foreground(&self) -> isize {
        lock(&self.inner).panel_prev_foreground
    }

    /// Open the settings window (or focus it).
    pub fn open_settings(self: &Arc<Self>) {
        let monitor = lock(&self.inner).monitor.clone();
        if let Err(e) = windows::open_settings(&self.app, monitor.as_ref()) {
            eprintln!("HealthierYou: could not open settings: {e}");
        }
    }

    // ---------------------------------------------------------- engine plumbing

    fn set_sink(&self, sink: Arc<dyn ShellEventSink>) {
        *lock(&self.sink) = Some(sink);
    }

    fn visible(&self) -> bool {
        lock(&self.inner).model.is_visible()
    }
}

fn place(i: &mut Inner, p: Placement, monitors: &[Monitor]) {
    i.pet = Some(p.rect);
    i.monitor = monitors.get(p.monitor).cloned();
}

/// The engine's handle on the pet shell. Cheap to clone; callable from any thread.
#[derive(Debug, Clone)]
pub struct ShellHandle(Arc<Shell>);

impl PetShell for ShellHandle {
    fn set_state(&self, state: PetState, tag: Option<String>) {
        self.0
            .post(move |s| s.run(|i, _| i.model.set_state(state, tag)));
    }

    fn show_bubble(&self, bubble: Bubble) -> BubbleOutcome {
        if bubble.validate().is_err() {
            return BubbleOutcome::Invalid;
        }
        if !self.0.visible() {
            return BubbleOutcome::Suppressed;
        }
        self.0.post(move |s| {
            s.run(|i, _| {
                let id = bubble.id.clone();
                let (outcome, mut fx) = i.model.show_bubble(bubble);
                if outcome == BubbleOutcome::Suppressed {
                    // Hidden between the call and now: tell the engine it closed.
                    fx.push(Effect::Emit(ShellEvent::BubbleClosed {
                        bubble_id: id,
                        cause: CloseCause::Hidden,
                    }));
                }
                fx
            });
        });
        BubbleOutcome::Shown
    }

    fn close_bubble(&self, bubble_id: &str) {
        let id = bubble_id.to_owned();
        self.0
            .post(move |s| s.run(|i, _| i.model.close_bubble(&id)));
    }

    fn hide(&self, reason: HideReason) {
        self.0.post(move |s| s.run(|i, _| i.model.hide(reason)));
    }

    fn unhide(&self, reason: HideReason) {
        self.0.post(move |s| s.run(|i, _| i.model.unhide(reason)));
    }

    fn is_visible(&self) -> bool {
        self.0.visible()
    }

    fn set_menu_actions(&self, actions: Vec<MenuAction>) {
        self.0
            .post(move |s| s.run(|i, _| i.model.set_menu_actions(actions)));
    }

    fn set_event_sink(&self, sink: Arc<dyn ShellEventSink>) {
        self.0.set_sink(sink);
    }
}
