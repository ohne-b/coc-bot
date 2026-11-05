fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "screenshot",
            "save_settings",
            "discover",
            "start",
            "stop",
            "scan_army",
            "preview_deployment",
            "pause",
            "history",
            "session_detail",
            "desktop_update",
            "restart_app",
        ]),
    ))
    .expect("Tauri build configuration");
}
