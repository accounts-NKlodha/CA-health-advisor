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
    "open_review",
    "review_week",
    "review_month",
    "review_phone_label",
    "review_suggestion",
    "review_target",
    "settings_page",
    "save_coach_settings",
    "save_water_settings",
    "save_workweek",
    "save_accessibility",
    "plan_page",
    "plan_save",
    "plan_cancel",
    "open_setup",
    "setup_page",
    "setup_sample",
    "setup_finish",
    "backup_export",
    "backup_stage",
    "backup_import",
    "server_status",
    "server_pair",
    "server_refresh",
    "server_unpair",
    "server_notice_seen",
    "server_set_address",
    "server_share_set",
    "server_notice_dismiss",
    "server_share_preview",
    "server_restore",
    "app_page",
    "update_choose",
    "update_install",
    "set_autostart",
    "camera_open",
    "camera_off",
    "camera_on",
    "camera_page",
    "camera_consent",
    "camera_try_again",
    "camera_withdraw",
    "camera_report",
    "camera_frame",
];

/// The update public keys this build trusts (slice V1-6), passed to the program as
/// `HY_UPDATE_KEYS`: the `HY_UPDATE_PUBKEY` environment variable if it is set (the CI
/// lifecycle test sets a key it makes for the run), otherwise the key lines of
/// `release/update-keys.txt` (the owner's public key, committed once; never a secret
/// key). With neither, the build has no key and refuses every update file.
fn update_keys() -> String {
    println!("cargo:rerun-if-env-changed=HY_UPDATE_PUBKEY");
    if let Ok(k) = std::env::var("HY_UPDATE_PUBKEY") {
        return k.split_whitespace().collect::<Vec<_>>().join(";");
    }
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../release/update-keys.txt");
    println!("cargo:rerun-if-changed={}", file.display());
    std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join(";")
}

fn main() {
    println!("cargo:rustc-env=HY_UPDATE_KEYS={}", update_keys());
    // The Windows application manifest: Tauri's default (common controls v6) plus
    // per-monitor v2 DPI awareness from the first instruction, so Windows never
    // bitmap-stretches the pet or the screens at 125 to 200% scaling (slice V1-6).
    println!("cargo:rerun-if-changed=windows/app.manifest");
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(windows)
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build script");
}
