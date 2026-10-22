// Each app command gets an `allow-<command>` permission; the capability files in
// `capabilities/` grant each window only the commands it needs (architecture 9:
// Tauri capabilities per window).
const COMMANDS: &[&str] = &[
    "pet_bootstrap",
    "pet_ready",
    "pet_click",
    "bubble_measured",
    "bubble_close",
    "bubble_action",
    "menu_action",
    "drag_start",
    "drag_move",
    "drag_end",
    "pet_edit",
    "bring_pet",
    "hide_pet",
    "open_settings",
    "panel_bootstrap",
    "panel_close",
    "settings_bootstrap",
    "set_hotkey",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build script");
}
