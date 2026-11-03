fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "screenshot",
            "save_settings",
            "discover",
            "start",
            "stop",
            "pause",
            "history",
            "session_detail",
        ]),
    ))
    .expect("Tauri build configuration");
}
