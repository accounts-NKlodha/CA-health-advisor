//! The tray icon and menu (wireframe T1). The tray stays when the pet is hidden, so
//! "Show pet" and "Bring pet above clock" always work without the pet.

use std::sync::Arc;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::shell::Shell;

const TRAY_ID: &str = "hy-tray";
const ENGINE_PREFIX: &str = "engine:";

fn build_menu(app: &AppHandle, shell: &Shell) -> tauri::Result<Menu<Wry>> {
    let view = shell.tray_view();
    let panel_label = shell.hotkey_label().map_or_else(
        || "Open pet panel".to_owned(),
        |hk| format!("Open pet panel ({hk})"),
    );
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "toggle",
        view.toggle_label,
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "bring",
        "Bring pet above clock",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "panel",
        panel_label,
        true,
        None::<&str>,
    )?)?;
    for a in &view.actions {
        let id = format!("{ENGINE_PREFIX}{}", a.id);
        menu.append(&MenuItem::with_id(app, id, &a.label, true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "review",
        "Weekly review",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "settings",
        "Settings",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit HealthierYou",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn on_menu(app: &AppHandle, id: &str) {
    let Some(shell) = app.try_state::<Arc<Shell>>() else {
        return;
    };
    let shell = Arc::clone(&shell);
    match id {
        "toggle" => shell.toggle_from_tray(),
        "bring" => shell.bring_above_clock(),
        "panel" => shell.open_panel(),
        "review" => shell.open_review(),
        "settings" => shell.open_settings(),
        "quit" => app.exit(0),
        other => {
            if let Some(action) = other.strip_prefix(ENGINE_PREFIX) {
                shell.menu_action(action);
            }
        }
    }
}

/// Create the tray icon.
///
/// # Errors
/// A Tauri tray or menu error.
pub fn create(app: &AppHandle, shell: &Shell) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(shell.tray_view().tooltip)
        .menu(&build_menu(app, shell)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuild the tray menu and tooltip after a change (hidden or shown, hotkey, engine
/// actions).
pub fn refresh(app: &AppHandle) {
    let (Some(shell), Some(tray)) = (app.try_state::<Arc<Shell>>(), app.tray_by_id(TRAY_ID)) else {
        return;
    };
    if let Ok(menu) = build_menu(app, &shell) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_tooltip(Some(shell.tray_view().tooltip));
}
